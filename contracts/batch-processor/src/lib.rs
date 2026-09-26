#![no_std]

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
const VERSION: u32 = 1;

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
    /// Checked fifth (last, immediately before auth): `token` is the
    /// all-zero Stellar address, which cannot be a SEP-41 token contract.
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

/// Emitted by `initialize` when the processor's admin is first set.
fn event_initialized(env: &Env, admin: &Address) {
    env.events()
        .publish((symbol_short!("init"), admin.clone()), admin.clone());
}

/// Emitted by `upgrade` after the processor's own WASM has been replaced.
///
/// Topics: `("upgraded", caller)` — the admin that authorized the swap.
/// Data:   `upgraded_at` — the ledger timestamp at which the swap took effect.
fn event_upgraded(env: &Env, caller: &Address, upgraded_at: u64) {
    env.events()
        .publish((symbol_short!("upgraded"), caller.clone()), upgraded_at);
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
    /// 5. `token` is not the all-zero Stellar address — else [`Error::InvalidToken`]
    ///
    /// Only the first failing check is returned, so a batch that is both too
    /// large and contains a zero amount always reports `BatchTooLarge`.
    /// Validation then short-circuits an empty batch (`total == 0`) to `Ok(0)`
    /// before auth; `funder.require_auth()` runs only once all checks pass.
    ///
    /// # Panics
    /// `token` must be a contract implementing SEP-41. The zero-address
    /// precondition above is the only token check this contract performs —
    /// the same guard `DripFactory::create_stream` applies — so an address
    /// that is not a SEP-41 token contract (a Stellar account, or a contract
    /// without `transfer`) is discovered at the first
    /// `tk.transfer(&funder, ...)` call instead, which aborts with an opaque
    /// host-level error (contract function not found / non-contract address)
    /// rather than returning an `Error` variant. The abort rolls the entire
    /// transaction back, so no funds move; clients should probe the token
    /// off-chain before submitting.
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

        // Fifth and final check: a zeroed token address can never be a
        // SEP-41 contract, so reject it here instead of letting the first
        // `tk.transfer` below die with an opaque host-level error.
        if is_zero_address(&env, &token) {
            return Err(Error::InvalidToken);
        }

        // Empty batch — nothing to do.
        if total == 0 {
            return Ok(0);
        }

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
    /// (nothing to migrate), but future changes such as keep-alive, fee
    /// deduction, or event-emission updates would otherwise be invisible to a
    /// client holding only the deployed address: read `version()` first and
    /// branch on it instead of feature-probing. Mirrors the role
    /// `DripStream::storage_version` plays for stored layout.
    ///
    /// See `VERSION` for the current value; it starts at `1`.
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
