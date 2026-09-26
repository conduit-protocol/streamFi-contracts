#![no_std]

mod ttl;

#[cfg(test)]
mod tests;

use drip_common::{is_zero_address, ttl};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env,
    Symbol, Vec,
};

/// Maximum number of transfers permitted in a single batch.
const MAX_BATCH_SIZE: u32 = 100;

/// Behaviour version of this deployment.
///
/// Bump this whenever `process_batch` (or any other entry point) changes
/// observable behaviour — validation order, error codes, the event payload,
/// the batch cap, auth placement. Unlike `DripStream::storage_version` there
/// is no stored layout to migrate (the processor is stateless), but deployed
/// instances still need a way to report which behaviour set they run so
/// clients and upgrade tooling can tell an old build from a new one; see
/// [`BatchTransferProcessor::version`].
///
/// History:
///
/// - `1` — original surface: `process_batch`, `max_batch_size`, `version`.
/// - `2` — `process_batch` gained a real SEP-41 precondition on `token`
///   (wallet + `balance` probes, so a transposed `funder`/`token` pair fails
///   with `InvalidToken` before auth) and an instance-TTL keep-alive;
///   `preview_batch` was added.
const VERSION: u32 = 2;

/// Instance-storage key space.
///
/// The processor is stateless with respect to transfers — every call recomputes
/// its batch total from the arguments — but it does hold one durable value: the
/// admin that may replace the contract's own WASM (issue #651). Without a
/// stored authority an `upgrade` entry point would have to be permissionless,
/// which would let anyone replace the code that custodies funds in flight.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Address allowed to call `upgrade`. Set by `initialize`.
    Admin,
}
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Checked first: `recipients` and `amounts` have different lengths.
    LengthMismatch = 1,
    /// Checked second: the batch exceeds [`MAX_BATCH_SIZE`].
    BatchTooLarge = 2,
    /// Checked third: an individual amount is zero or negative.
    InvalidAmount = 3,
    /// Checked fourth: integer overflow computing the total to pull from the
    /// funder.
    ArithmeticOverflow = 4,
    /// Checked fifth (last, immediately before auth): `token` does not
    /// behave like a SEP-41 asset — it is the all-zero Stellar address, a
    /// `G...` wallet, or a contract that fails the `balance` probe. This is
    /// also the error a caller gets for transposing the `funder` and `token`
    /// arguments (see the argument-order example on
    /// [`BatchTransferProcessor::process_batch`]). Note `preview_batch` takes
    /// no `token`, so it can only ever return the first four codes.
    InvalidToken = 5,
    /// The caller is not the admin stored by `initialize`, so it may not
    /// replace the contract's WASM.
    NotAuthorized = 6,
    /// The WASM hash provided to `upgrade` is all zeros (invalid).
    InvalidWasmHash = 7,
    /// `initialize` was called on a processor that already has an admin.
    AlreadyInitialized = 8,
    /// `upgrade` was called before `initialize` has set an admin.
    NotInitialized = 9,
}

#[contract]
pub struct BatchTransferProcessor;

/// Checks 1–4 of the documented `process_batch` validation order: length,
/// batch size, per-amount sign, and checked accumulation of the total.
///
/// Deliberately shared with [`BatchTransferProcessor::preview_batch`] so the
/// read-only preview can never disagree with the paying call about what a
/// valid batch is — same checks, same order, same error codes. `token` is not
/// a parameter here because it is check 5 and only `process_batch` takes one.
fn validate_total(recipients: &Vec<Address>, amounts: &Vec<i128>) -> Result<i128, Error> {
    if recipients.len() != amounts.len() {
        return Err(Error::LengthMismatch);
    }

    if amounts.len() > MAX_BATCH_SIZE {
        return Err(Error::BatchTooLarge);
    }

    // Validate every amount and accumulate the total in one pass so we
    // pull a single lump sum from the funder instead of N separate auths.
    let mut total: i128 = 0;
    for amount in amounts.iter() {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        total = total.checked_add(amount).ok_or(Error::ArithmeticOverflow)?;
    }

    Ok(total)
}

/// Returns true when `address` is a Stellar *account* (a `G...` strkey
/// wallet) rather than a contract (`C...`).
///
/// There is no `Address::is_contract` in `soroban-sdk` 21, and the
/// `Address → ScAddress` conversion is compiled out on the wasm target, so
/// rendering the strkey form — the same encoding `is_zero_address` compares
/// against — is the only on-chain way to tell a wallet from a contract
/// without provoking an opaque host-level failure.
fn is_stellar_account(address: &Address) -> bool {
    let strkey = address.to_string();
    // Both strkey variants render as exactly 56 characters. If that ever
    // changes, skip the check and let the SEP-41 probe below decide rather
    // than panicking on `copy_into_slice`'s length precondition.
    let mut buf = [0u8; 56];
    if strkey.len() as usize != buf.len() {
        return false;
    }
    strkey.copy_into_slice(&mut buf);
    buf[0] == b'G'
}

/// Check 5 of the documented `process_batch` validation order: `token` must
/// behave like a SEP-41 asset. Runs before `funder.require_auth()`.
///
/// Three probes, cheapest first:
///
/// 1. the all-zero Stellar address can never be a token contract;
/// 2. a `G...` account is a wallet, not a contract, so it can never expose
///    SEP-41 `balance` — this is exactly what a transposed `funder`/`token`
///    call puts in the `token` slot, and rejecting it here turns that mistake
///    into a clear [`Error::InvalidToken`] instead of an auth request aimed
///    at the token contract or an opaque host failure; and
/// 3. a SEP-41 `balance` call asks the remaining case — a contract address —
///    to answer like a token. A deployed contract without `balance` is
///    rejected with the same [`Error::InvalidToken`].
fn validate_token(env: &Env, token: &Address) -> Result<(), Error> {
    if is_zero_address(env, token) {
        return Err(Error::InvalidToken);
    }

    if is_stellar_account(token) {
        return Err(Error::InvalidToken);
    }

    let contract_addr = env.current_contract_address();
    match token::Client::new(env, token).try_balance(&contract_addr) {
        Ok(Ok(_)) => Ok(()),
        _ => Err(Error::InvalidToken),
    }
}

#[contractimpl]
impl BatchTransferProcessor {
    /// One-time setup: record the admin allowed to `upgrade` this contract.
    ///
    /// `process_batch` is permissionless and works without initialization; this
    /// call only establishes who may replace the implementation. Guarded
    /// against re-initialization so a second call cannot hand the upgrade
    /// authority to a different address after the fact.
    ///
    /// # Errors
    ///
    /// - `AlreadyInitialized` — an admin is already recorded.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        ttl::bump_instance(&env);
        env.storage().instance().set(&DataKey::Admin, &admin);
        event_initialized(&env, &admin);
        Ok(())
    }

    /// Transfer tokens from `funder` to each address in `recipients`.
    ///
    /// # Argument order
    ///
    /// `funder` comes **before** `token`, and both parameters are bare
    /// `Address`, so transposing them compiles cleanly and is only caught at
    /// runtime:
    ///
    /// ```ignore
    /// // Correct — funder first, then token.
    /// client.process_batch(&funder, &token, &recipients, &amounts);
    ///
    /// // Wrong — funder and token swapped. This compiles, but the wallet now
    /// // sitting in the `token` slot is rejected by check 5 and the call
    /// // returns `Error::InvalidToken` before any auth request or transfer.
    /// client.process_batch(&token, &funder, &recipients, &amounts);
    /// ```
    ///
    /// # Auth
    /// `funder.require_auth()` is called before any state mutation.  The funder
    /// must have pre-authorised a transfer of at least `sum(amounts)` tokens to
    /// this contract, which then fans the funds out to the recipients in order.
    ///
    /// # Validation (all checks precede auth and any token movement)
    ///
    /// Checks run in this exact order and stop at the first failure — the
    /// same fail-early sequence a client should reproduce off-chain to give a
    /// specific pre-submission error, mirroring the `create_stream` validation
    /// list in the README:
    ///
    /// 1. `recipients.len() == amounts.len()` — else [`Error::LengthMismatch`]
    /// 2. `amounts.len() <= MAX_BATCH_SIZE` (100) — else [`Error::BatchTooLarge`]
    /// 3. every `amount > 0` — else [`Error::InvalidAmount`]
    /// 4. `sum(amounts)` fits in `i128` — else [`Error::ArithmeticOverflow`]
    /// 5. `token` behaves like a SEP-41 asset — else [`Error::InvalidToken`]
    ///
    /// Checks 1–4 are shared verbatim with [`Self::preview_batch`], so a
    /// client can pre-compute the total (and surface the exact error) without
    /// auth; check 5 needs `token` and therefore only exists here.
    ///
    /// Only the first failing check is returned, so a batch that is both too
    /// large and contains a zero amount always reports `BatchTooLarge`.
    /// Validation then short-circuits an empty batch (`total == 0`) to `Ok(0)`
    /// before auth; `funder.require_auth()` runs only once all checks pass.
    ///
    /// # Token precondition (check 5)
    /// `token` must be a contract implementing SEP-41. Three probes run, all
    /// before `funder.require_auth()`:
    ///
    /// 1. the all-zero Stellar address is rejected — the same guard
    ///    `DripFactory::create_stream` applies;
    /// 2. a `G...` account is rejected — a wallet can never be a token
    ///    contract; and
    /// 3. a SEP-41 `balance` probe asks whatever contract address is left to
    ///    answer like a token; a contract without `balance` is rejected with
    ///    the same [`Error::InvalidToken`].
    ///
    /// Together these turn a transposed `funder`/`token` pair (see the
    /// argument-order example above) into a clear [`Error::InvalidToken`]
    /// raised during validation — no auth request against the wrong address,
    /// no host-level failure mid-transfer, and no funds moved. Clients should
    /// still probe the token off-chain before submitting.
    ///
    /// # Keep-alive (instance TTL)
    /// A successful, non-empty call renews this contract's instance storage
    /// TTL (`ttl::bump`, the same `TTL_THRESHOLD` → `TTL_EXTEND_TO` extension
    /// `DripStream`, `DripFactory`, and `DripGovernor` apply on every
    /// state-mutating call), so a processor that keeps processing batches
    /// cannot be archived out from under its users. Validation failures, the
    /// empty-batch no-op, and [`Self::preview_batch`] leave TTL untouched.
    ///
    /// # Protocol fee
    /// Batch transfers are **fee-exempt by design**: unlike
    /// `DripFactory::create_stream`, which deducts `DripGovernor::config().fee_bps`
    /// on every stream creation, `process_batch` moves the exact `sum(amounts)`
    /// with no protocol fee. The processor is a separate contract scoped to
    /// bounded execution only (see ADR-007) and deliberately holds no governor
    /// coupling; this is an intentional design decision, not an omission.
    ///
    /// # Retries and idempotency
    /// **This function is NOT idempotent and must not be retried blindly.**
    /// There is no batch identifier or idempotency key: each successful call
    /// re-pulls the full `sum(amounts)` from the funder and re-transfers to
    /// every recipient. If a submission appears to fail (e.g. timeout after
    /// simulation), the caller **must** confirm on-chain state — funder balance,
    /// recipient balances, or the transaction result — before resubmitting.
    /// Retrying a call that actually landed pays every recipient a second time
    /// and drains the funder twice.
    ///
    /// # Duplicate recipients
    /// `recipients` is not deduplicated. If the same address appears more than
    /// once, each occurrence receives its own separate transfer of the matching
    /// `amount` entry; duplicates are neither merged nor rejected. The total
    /// pulled from the funder is still the sum of all entries (each duplicate
    /// counted once per occurrence), so a recipient listed twice with amounts
    /// `[10, 20]` ends up with `30`.
    ///
    /// # Single-entry batches
    /// A batch of size 1 is accepted and is functionally equivalent to a plain
    /// `token.transfer`, but pays this contract's auth and loop overhead. For
    /// single-recipient payouts, prefer calling `token.transfer` directly;
    /// route through `process_batch` only when fanning out to multiple
    /// recipients.
    ///
    /// # Returns
    /// The total number of tokens transferred on success.
    pub fn process_batch(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<i128, Error> {
        // ── Input validation (before auth and any token movement) ────────────

        // Checks 1–4, shared verbatim with `preview_batch`.
        let total = validate_total(&recipients, &amounts)?;

        // Check 5 — token looks like a SEP-41 asset: zero-address guard,
        // wallet guard, `balance` probe. See `validate_token`.
        validate_token(&env, &token)?;

        // Empty batch — nothing to do.
        if total == 0 {
            return Ok(0);
        }

        // ── Keep-alive ───────────────────────────────────────────────────────
        // Renew the instance TTL on the path that actually moves funds, so a
        // busy processor can never be archived between payouts. Validation
        // failures and the empty-batch no-op above return without touching
        // storage, exactly like the other three contracts' fail-early paths.
        ttl::bump(&env);

        // ── Auth ─────────────────────────────────────────────────────────────
        funder.require_auth();

        // ── Token transfers ───────────────────────────────────────────────────
        let tk = token::Client::new(&env, &token);
        let contract_addr = env.current_contract_address();

        // Pull the full batch total from the funder into this contract in one
        // transfer, then fan out to each recipient individually.  One inbound
        // transfer keeps the auth surface minimal (funder signs once).
        tk.transfer(&funder, &contract_addr, &total);

        for (recipient, amount) in recipients.iter().zip(amounts.iter()) {
            tk.transfer(&contract_addr, &recipient, &amount);
        }

        // Emitted only after the fan-out above succeeds: a reverted transfer
        // rolls the whole transaction back, so this event never describes a
        // batch that did not actually move funds.
        env.events().publish(
            (Symbol::new(&env, "batch_transferred"), funder.clone()),
            (token.clone(), recipients.len(), total),
        );

        Ok(total)
    }

    /// Read-only: the total `process_batch` would move for this batch.
    ///
    /// Runs the same checks 1–4 as [`Self::process_batch`] — length, batch
    /// size, per-amount sign, checked accumulation — in the same order and
    /// with the same error codes, but takes no `funder`, no `token`, requests
    /// no auth, extends no TTL, and moves no funds. Use it to show a user
    /// "this batch will cost X tokens" *before* they sign, without
    /// re-implementing the contract's validation off-chain: a client-side sum
    /// that disagrees with the contract's (a different overflow boundary, a
    /// forgotten cap check) would otherwise stay invisible until the
    /// transaction fails on-chain.
    ///
    /// ```ignore
    /// let total = client.preview_batch(&recipients, &amounts)?; // no auth
    /// // ... show `total` to the user, then:
    /// client.process_batch(&funder, &token, &recipients, &amounts);
    /// ```
    ///
    /// Check 5 (`token` behaves like a SEP-41 asset) is not run: this
    /// function has no `token` parameter and never touches the token, so a
    /// preview always succeeds or fails purely on the batch's shape. The token
    /// check still runs in [`Self::process_batch`] before auth or transfer.
    ///
    /// An empty batch previews as `Ok(0)`, matching the `process_batch`
    /// no-op.
    pub fn preview_batch(
        _env: Env,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<i128, Error> {
        validate_total(&recipients, &amounts)
    }

    /// Largest number of transfers `process_batch` will accept in one call.
    ///
    /// Exposed as a read-only entry point so a client integrating against a
    /// deployed instance can discover the cap on-chain instead of hardcoding
    /// it. A hardcoded client-side copy silently drifts if the contract is
    /// ever upgraded with a different limit: the client would keep building
    /// batches it believes are valid until `process_batch` starts rejecting
    /// them with [`Error::BatchTooLarge`].
    ///
    /// This returns the same value [`Self::process_batch`] compares against,
    /// so a batch of exactly `max_batch_size()` entries is accepted and one
    /// more is rejected.
    pub fn max_batch_size(_env: Env) -> u32 {
        MAX_BATCH_SIZE
    }

    /// Behaviour version of this deployed instance.
    ///
    /// Bumped whenever any entry point's observable behaviour changes —
    /// validation order, error codes, the `batch_transferred` payload, the
    /// batch cap, or where `require_auth` sits. The processor is stateless
    /// (nothing to migrate), but changes such as the SEP-41 token probe, the
    /// instance-TTL keep-alive, `preview_batch`, or future fee-deduction and
    /// event-emission updates would otherwise be invisible to a client holding
    /// only the deployed address: read `version()` first and branch on it
    /// instead of feature-probing. Mirrors the role
    /// `DripStream::storage_version` plays for stored layout.
    ///
    /// See `VERSION` for the current value (`2`) and its history.
    pub fn version(_env: Env) -> u32 {
        VERSION
    }

    // ── Self-upgrade (admin-gated) ──────────────────────────────────────────

    /// Replace this contract's own WASM bytecode.
    ///
    /// The new WASM must already be uploaded to the ledger (via
    /// `stellar contract upload`); only the hash is passed here. Gated on the
    /// admin recorded by [`Self::initialize`], mirroring
    /// `DripGovernor::upgrade` and `DripFactory::upgrade_self` — which this
    /// contract previously lacked entirely (issue #651). The gate matters here
    /// for the same reason it does elsewhere: the processor custodies the whole
    /// batch total between the inbound pull and the outbound fan-out, so a
    /// replaced implementation runs with funds in flight.
    ///
    /// An all-zero hash is rejected: `update_current_contract_wasm` with it
    /// would replace the contract with something that cannot transfer.
    pub fn upgrade(env: Env, caller: Address, new_wasm_hash: BytesN<32>) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if caller != admin {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();

        if new_wasm_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(Error::InvalidWasmHash);
        }

        ttl::bump_instance(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        event_upgraded(&env, &caller, env.ledger().timestamp());
        Ok(())
    }
}
