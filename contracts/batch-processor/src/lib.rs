#![no_std]

mod ttl;

#[cfg(test)]
mod tests;

use drip_common::is_zero_address;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env,
    IntoVal, Symbol, Vec,
};

pub use ttl::MaintenanceReserve;

/// Maximum number of transfers permitted in a single batch (Issue #677).
///
/// Bounded to 50 to prevent transaction CPU and footprint budget exhaustion while still
/// allowing payroll-sized batches to fail fast before any auth or token movement (Issue #964).
pub const MAX_BATCH_SIZE: u32 = 50;

/// Maximum batch limit alias for MAX_BATCH_SIZE (Issue #677).
pub const MAX_BATCH_LIMIT: u32 = MAX_BATCH_SIZE;

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
/// - `3` — added `process_batch_with_limit`, which lets callers bound work
///   deterministically and reports the processed prefix and remaining items.
/// - `4` — added `process_batch_with_allowance`, which pre-flights the
///   aggregate allowance before pulling funds via `transfer_from` (issue #925).
/// - `5` — added admin-only `batch_pause` / `batch_resume` (issue #892) and a
///   `batch_metrics` event after every stream batch (issue #891).
/// - `6` — raised the explicit batch cap to 50 and added partial/atomic
///   cancellation mode for payroll administration (issues #963-#964).
const VERSION: u32 = 6;

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
    /// Temporary lock during batch execution to prevent reentrancy loops.
    ReentrancyLock,
    /// Maintenance reserve for keeper gas rebate (Issue #817).
    MaintenanceReserve,
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
    /// Another batch execution or reentrant call is already in progress.
    ReentrancyForbidden = 10,
    /// The requested per-call item cap is zero or exceeds `MAX_BATCH_SIZE`.
    InvalidBatchLimit = 11,
    /// The funder has not approved enough token allowance to cover the
    /// aggregate batch on the allowance-funded path.
    InsufficientAllowance = 12,
}

/// Individual execution status for an item in a batch execution (Issue #697).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchResult {
    pub item_index: u32,
    pub success: bool,
    pub error_code: Option<u32>,
}

/// Outcome of a deterministically capped batch transfer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialBatchResult {
    pub results: Vec<BatchResult>,
    pub requested_count: u32,
    pub processed_count: u32,
    pub remaining_count: u32,
    pub transferred_total: i128,
}

impl Error {
    /// Alias for BatchTooLarge per issue #677.
    #[allow(non_upper_case_globals)]
    pub const BatchSizeExceeded: Error = Error::BatchTooLarge;
    pub const BATCH_SIZE_EXCEEDED: Error = Error::BatchTooLarge;
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
/// Telemetry for keepers sizing their batches (issue #891).
///
/// Topics: `("batch_metrics", batch_type)`; data: `(stream_count, elapsed_nanos)`.
///
/// Soroban gives contracts no wall clock or CPU-instruction counter, so
/// `elapsed_nanos` is measured from the ledger timestamp and is `0` for a
/// batch that completes within one invocation (the normal case). Keepers
/// should pair `stream_count` with the transaction's reported resource usage
/// to derive per-stream cost; the field is kept so the payload stays stable
/// if a finer-grained host clock becomes available.
fn event_batch_metrics(env: &Env, batch_type: Symbol, stream_count: u32, started_at: u64) {
    let elapsed_nanos = env
        .ledger()
        .timestamp()
        .saturating_sub(started_at)
        .saturating_mul(1_000_000_000);
    env.events().publish(
        (Symbol::new(env, "batch_metrics"), batch_type),
        (stream_count, elapsed_nanos),
    );
}

/// Shared body of `batch_pause` / `batch_resume`: admin check, size cap,
/// duplicate-skipping, then one call per unique stream. Any stream call that
/// fails traps, so the whole batch reverts and no stream is left half-handled.
fn batch_emergency_toggle(
    env: &Env,
    admin: &Address,
    stream_ids: &Vec<Address>,
    stream_fn: &str,
    batch_type: Symbol,
) -> Result<u32, Error> {
    let stored: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if admin != &stored {
        return Err(Error::NotAuthorized);
    }
    if stream_ids.len() > MAX_BATCH_SIZE {
        return Err(Error::BatchTooLarge);
    }
    admin.require_auth();
    if stream_ids.is_empty() {
        return Ok(0);
    }
    ttl::bump(env);

    let started_at = env.ledger().timestamp();
    let func = Symbol::new(env, stream_fn);
    let mut handled: Vec<Address> = Vec::new(env);
    for stream_id in stream_ids.iter() {
        if handled.contains(&stream_id) {
            continue;
        }
        let mut args = Vec::new(env);
        args.push_back(admin.to_val());
        let _: () = env.invoke_contract(&stream_id, &func, args);
        handled.push_back(stream_id);
    }

    event_batch_metrics(env, batch_type, handled.len(), started_at);
    Ok(handled.len())
}

fn event_upgraded(env: &Env, caller: &Address, upgraded_at: u64) {
    env.events()
        .publish((symbol_short!("upgraded"), caller.clone()), upgraded_at);
}

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
        ttl::bump(&env);
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
    /// 2. `amounts.len() <= MAX_BATCH_SIZE` (30) — else [`Error::BatchTooLarge`]
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
    /// Executes a batch transfer and returns per-item execution status codes (Issue #697).
    pub fn process_batch_detailed(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<Vec<BatchResult>, Error> {
        let total = validate_total(&recipients, &amounts)?;
        validate_token(&env, &token)?;

        let mut results = Vec::new(&env);
        if total == 0 {
            return Ok(results);
        }

        ttl::bump(&env);

        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

        funder.require_auth();

        let tk = token::Client::new(&env, &token);
        let contract_addr = env.current_contract_address();

        tk.transfer(&funder, &contract_addr, &total);

        for (idx, (recipient, amount)) in recipients.iter().zip(amounts.iter()).enumerate() {
            tk.transfer(&contract_addr, &recipient, &amount);
            results.push_back(BatchResult {
                item_index: idx as u32,
                success: true,
                error_code: None,
            });
        }

        env.storage().temporary().remove(&DataKey::ReentrancyLock);

        env.events().publish(
            (Symbol::new(&env, "batch_transferred"), funder.clone()),
            (token.clone(), recipients.len(), total),
        );

        Ok(results)
    }

    /// Process at most `max_items` entries from the front of a batch.
    ///
    /// This is a deterministic alternative to querying live transaction gas,
    /// which contract code cannot inspect. `max_items` must be between 1 and
    /// [`MAX_BATCH_SIZE`]. If the input contains more entries, the unprocessed
    /// suffix is left untouched and reported in `remaining_count`.
    pub fn process_batch_with_limit(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
        max_items: u32,
    ) -> Result<PartialBatchResult, Error> {
        if recipients.len() != amounts.len() {
            return Err(Error::LengthMismatch);
        }
        if max_items == 0 || max_items > MAX_BATCH_SIZE {
            return Err(Error::InvalidBatchLimit);
        }

        let requested_count = recipients.len();
        let processed_count = requested_count.min(max_items);
        let remaining_count = requested_count - processed_count;
        let mut total = 0_i128;
        for index in 0..processed_count {
            let amount = amounts.get(index).ok_or(Error::LengthMismatch)?;
            if amount <= 0 {
                return Err(Error::InvalidAmount);
            }
            total = total.checked_add(amount).ok_or(Error::ArithmeticOverflow)?;
        }

        validate_token(&env, &token)?;
        let mut results = Vec::new(&env);
        if processed_count == 0 {
            return Ok(PartialBatchResult {
                results,
                requested_count,
                processed_count,
                remaining_count,
                transferred_total: 0,
            });
        }

        ttl::bump(&env);
        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

        funder.require_auth();

        let tk = token::Client::new(&env, &token);
        let contract_addr = env.current_contract_address();
        tk.transfer(&funder, &contract_addr, &total);

        for index in 0..processed_count {
            let recipient = recipients.get(index).ok_or(Error::LengthMismatch)?;
            let amount = amounts.get(index).ok_or(Error::LengthMismatch)?;
            tk.transfer(&contract_addr, &recipient, &amount);
            results.push_back(BatchResult {
                item_index: index,
                success: true,
                error_code: None,
            });
        }

        env.storage().temporary().remove(&DataKey::ReentrancyLock);
        env.events().publish(
            (Symbol::new(&env, "batch_transferred"), funder),
            (token, processed_count, total),
        );

        Ok(PartialBatchResult {
            results,
            requested_count,
            processed_count,
            remaining_count,
            transferred_total: total,
        })
    }

    /// Read-only alias for process_batch_detailed (Issue #697).
    pub fn execute_batch(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<Vec<BatchResult>, Error> {
        Self::process_batch_detailed(env, funder, token, recipients, amounts)
    }

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

        // ── Reentrancy guard ─────────────────────────────────────────────────
        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

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

        env.storage().temporary().remove(&DataKey::ReentrancyLock);

        // Emitted only after the fan-out above succeeds: a reverted transfer
        // rolls the whole transaction back, so this event never describes a
        // batch that did not actually move funds.
        env.events().publish(
            (Symbol::new(&env, "batch_transferred"), funder.clone()),
            (token.clone(), recipients.len(), total),
        );

        Ok(total)
    }

    /// Execute a batch funded by a pre-approved SEP-41 allowance (Issue #925).
    ///
    /// Unlike [`Self::process_batch`], which pulls the total with the funder's
    /// own signature, this entry point spends the allowance the funder granted
    /// to this processor (`approve(funder, processor, allowance, expiry)`).
    /// The aggregate allowance is checked once, before any transfer, so an
    /// under-approved batch fails up front instead of partway through the
    /// fan-out.
    ///
    /// # Validation
    /// Checks 1–5 match [`Self::process_batch`], followed by the allowance
    /// pre-flight:
    ///
    /// 6. `allowance(funder, processor) >= sum(amounts)` — else
    ///    [`Error::InsufficientAllowance`]
    ///
    /// The pre-flight is read-only and runs before the instance TTL bump, the
    /// reentrancy lock, and any token movement.
    ///
    /// # Auth
    /// `transfer_from` consumes the funder's pre-approved allowance; the
    /// processor authorizes the pull as the spender. No direct
    /// `funder.require_auth()` is requested, so the funder signs only the
    /// standalone `approve` transaction.
    ///
    /// # Returns
    /// The total number of tokens transferred on success.
    pub fn process_batch_with_allowance(
        env: Env,
        funder: Address,
        token: Address,
        recipients: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<i128, Error> {
        let total = validate_total(&recipients, &amounts)?;
        validate_token(&env, &token)?;

        if total == 0 {
            return Ok(0);
        }

        let tk = token::Client::new(&env, &token);
        let contract_addr = env.current_contract_address();

        // ── Allowance pre-flight ─────────────────────────────────────────
        // One read for the whole batch: without it, an allowance smaller than
        // the total would only surface inside `transfer_from`, after the
        // reentrancy lock and TTL write have already been paid for.
        if tk.allowance(&funder, &contract_addr) < total {
            return Err(Error::InsufficientAllowance);
        }

        ttl::bump(&env);

        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

        tk.transfer_from(&contract_addr, &funder, &contract_addr, &total);

        for (recipient, amount) in recipients.iter().zip(amounts.iter()) {
            tk.transfer(&contract_addr, &recipient, &amount);
        }

        env.storage().temporary().remove(&DataKey::ReentrancyLock);

        env.events().publish(
            (Symbol::new(&env, "batch_transferred"), funder),
            (token, recipients.len(), total),
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
    /// See `VERSION` for the current value and its history.
    pub fn version(_env: Env) -> u32 {
        VERSION
    }

    // ── Batch Cancel & Keeper Maintenance Reserve ─────────────────────────

    /// Gas-optimized batch cancellation for multiple streams belonging to a sender (Issue #816).
    ///
    /// Iterates through streams and invokes `cancel` on each in a single atomic transaction.
    /// Reclaims remaining deposit balances back to the authorized sender/caller.
    pub fn batch_cancel(env: Env, caller: Address, stream_ids: Vec<Address>) -> Result<u32, Error> {
        if stream_ids.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }
        if stream_ids.is_empty() {
            return Ok(0);
        }

        caller.require_auth();
        ttl::bump_adaptive_for_streams(&env, &stream_ids);

        let started_at = env.ledger().timestamp();
        let mut cancelled_count: u32 = 0;
        for stream_id in stream_ids.iter() {
            let mut args = Vec::new(&env);
            args.push_back(caller.to_val());
            let _: () = env.invoke_contract(&stream_id, &Symbol::new(&env, "cancel"), args);
            cancelled_count = cancelled_count.saturating_add(1);
        }

        env.events().publish(
            (Symbol::new(&env, "batch_cancelled"), caller),
            stream_ids.len(),
        );
        event_batch_metrics(&env, symbol_short!("cancel"), cancelled_count, started_at);

        Ok(cancelled_count)
    }

    /// Cancel several stream contracts with caller-selectable failure semantics (Issue #963).
    ///
    /// With `allow_partial = false`, every cancellation is invoked normally so any
    /// failure aborts the Soroban transaction and rolls back every earlier cancellation.
    /// With `allow_partial = true`, individual contract errors are captured and returned
    /// as per-item status codes while the remaining stream cancellations continue.
    ///
    /// The batch-size guard runs before auth or any cross-contract invocation.
    pub fn batch_cancel_with_mode(
        env: Env,
        caller: Address,
        stream_ids: Vec<Address>,
        allow_partial: bool,
    ) -> Result<Vec<BatchResult>, Error> {
        if stream_ids.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        let mut results = Vec::new(&env);
        if stream_ids.is_empty() {
            return Ok(results);
        }

        caller.require_auth();
        ttl::bump(&env);

        for (idx, stream_id) in stream_ids.iter().enumerate() {
            let mut args = Vec::new(&env);
            args.push_back(caller.to_val());

            if !allow_partial {
                let _: () = env.invoke_contract(&stream_id, &Symbol::new(&env, "cancel"), args);
                results.push_back(BatchResult {
                    item_index: idx as u32,
                    success: true,
                    error_code: None,
                });
                continue;
            }

            let res = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &stream_id,
                &Symbol::new(&env, "cancel"),
                args,
            );
            let (success, error_code) = match res {
                Ok(Ok(())) => (true, None),
                // The target returned a value that could not be converted to
                // the expected unit return type. Treat it as an item failure
                // in partial mode rather than aborting the whole batch.
                Ok(Err(_)) => (false, Some(9998)),
                Err(Ok(err)) => (false, Some(err.get_code())),
                Err(Err(soroban_sdk::InvokeError::Contract(code))) => (false, Some(code)),
                Err(Err(soroban_sdk::InvokeError::Abort)) => (false, Some(9999)),
            };
            results.push_back(BatchResult {
                item_index: idx as u32,
                success,
                error_code,
            });
        }

        env.events().publish(
            (Symbol::new(&env, "batch_cancelled"), caller),
            (stream_ids.len(), allow_partial),
        );

        Ok(results)
    }

    /// Lenient batch withdrawal across multiple streams (Issue #761).
    ///
    /// Catches execution errors per stream so a single failing or revoked stream does not
    /// revert the remaining payouts.
    /// Returns a vector of tuples: `(stream_id, success, error_code)`.
    pub fn batch_withdraw_lenient(
        env: Env,
        stream_ids: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<Vec<(Address, bool, u32)>, Error> {
        if stream_ids.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }
        if !amounts.is_empty() && amounts.len() != stream_ids.len() {
            return Err(Error::LengthMismatch);
        }

        let mut results = Vec::new(&env);
        if stream_ids.is_empty() {
            return Ok(results);
        }

        ttl::bump_adaptive_for_streams(&env, &stream_ids);

        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

        let started_at = env.ledger().timestamp();
        for (idx, stream_id) in stream_ids.iter().enumerate() {
            let amount = if amounts.is_empty() {
                // Query withdrawable amount if not provided
                let withdrawable_res = env.try_invoke_contract::<i128, soroban_sdk::Error>(
                    &stream_id,
                    &Symbol::new(&env, "withdrawable"),
                    Vec::new(&env),
                );
                match withdrawable_res {
                    Ok(Ok(val)) => val,
                    _ => 0,
                }
            } else {
                amounts.get(idx as u32).unwrap_or(0)
            };

            if amount <= 0 {
                results.push_back((stream_id.clone(), false, Error::InvalidAmount as u32));
                continue;
            }

            let mut args = Vec::new(&env);
            args.push_back(amount.into_val(&env));

            let res = env.try_invoke_contract::<i128, soroban_sdk::Error>(
                &stream_id,
                &Symbol::new(&env, "withdraw"),
                args,
            );

            match res {
                Ok(Ok(_)) => {
                    results.push_back((stream_id.clone(), true, 0));
                }
                Ok(Err(err)) => {
                    results.push_back((stream_id.clone(), false, err.get_code()));
                }
                Err(Ok(err)) => {
                    results.push_back((stream_id.clone(), false, err.get_code()));
                }
                Err(Err(soroban_sdk::InvokeError::Contract(code))) => {
                    results.push_back((stream_id.clone(), false, code));
                }
                Err(Err(soroban_sdk::InvokeError::Abort)) => {
                    results.push_back((stream_id.clone(), false, 9999));
                }
            }
        }

        env.storage().temporary().remove(&DataKey::ReentrancyLock);

        env.events().publish(
            (
                Symbol::new(&env, "batch_withdrawn_lenient"),
                env.current_contract_address(),
            ),
            (results.len(), stream_ids.len()),
        );
        event_batch_metrics(
            &env,
            symbol_short!("withdraw"),
            stream_ids.len(),
            started_at,
        );

        Ok(results)
    }

    /// Fault-tolerant batch top-up with per-item failure telemetry (Issue #888).
    ///
    /// Executes `top_up(caller, amount)` against each stream in `stream_ids`,
    /// catching individual stream failures (e.g. cancelled stream, invalid
    /// amount, or insufficient token allowance) so that remaining top-ups in
    /// the batch still complete.
    ///
    /// Emits `("top_up_item_failed", stream_id)` with `(item_index, error_code)`
    /// for each failed item, and `("batch_top_up_lenient", caller)` with
    /// `(succeeded_count, failed_count, total_topped_up)` at completion.
    pub fn batch_top_up_lenient(
        env: Env,
        caller: Address,
        stream_ids: Vec<Address>,
        amounts: Vec<i128>,
    ) -> Result<Vec<BatchResult>, Error> {
        if stream_ids.len() != amounts.len() {
            return Err(Error::LengthMismatch);
        }
        if stream_ids.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        let mut results = Vec::new(&env);
        if stream_ids.is_empty() {
            return Ok(results);
        }

        caller.require_auth();
        ttl::bump_adaptive_for_streams(&env, &stream_ids);

        if env.storage().temporary().has(&DataKey::ReentrancyLock) {
            return Err(Error::ReentrancyForbidden);
        }
        env.storage()
            .temporary()
            .set(&DataKey::ReentrancyLock, &true);

        let mut succeeded: u32 = 0;
        let mut failed: u32 = 0;
        let mut total_topped_up: i128 = 0;

        for (idx, (stream_id, amount)) in stream_ids.iter().zip(amounts.iter()).enumerate() {
            let item_index = idx as u32;
            if amount <= 0 {
                let code = Error::InvalidAmount as u32;
                failed = failed.saturating_add(1);
                results.push_back(BatchResult {
                    item_index,
                    success: false,
                    error_code: Some(code),
                });
                env.events().publish(
                    (Symbol::new(&env, "top_up_item_failed"), stream_id.clone()),
                    (item_index, code),
                );
                continue;
            }

            let mut args = Vec::new(&env);
            args.push_back(caller.to_val());
            args.push_back(amount.into_val(&env));

            let res = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &stream_id,
                &Symbol::new(&env, "top_up"),
                args,
            );

            match res {
                Ok(Ok(())) => {
                    succeeded = succeeded.saturating_add(1);
                    total_topped_up = total_topped_up.saturating_add(amount);
                    results.push_back(BatchResult {
                        item_index,
                        success: true,
                        error_code: None,
                    });
                }
                Ok(Err(_)) => {
                    let code = 9998u32;
                    failed = failed.saturating_add(1);
                    results.push_back(BatchResult {
                        item_index,
                        success: false,
                        error_code: Some(code),
                    });
                    env.events().publish(
                        (Symbol::new(&env, "top_up_item_failed"), stream_id.clone()),
                        (item_index, code),
                    );
                }
                Err(Ok(err)) => {
                    let code = err.get_code();
                    failed = failed.saturating_add(1);
                    results.push_back(BatchResult {
                        item_index,
                        success: false,
                        error_code: Some(code),
                    });
                    env.events().publish(
                        (Symbol::new(&env, "top_up_item_failed"), stream_id.clone()),
                        (item_index, code),
                    );
                }
                Err(Err(soroban_sdk::InvokeError::Contract(code))) => {
                    failed = failed.saturating_add(1);
                    results.push_back(BatchResult {
                        item_index,
                        success: false,
                        error_code: Some(code),
                    });
                    env.events().publish(
                        (Symbol::new(&env, "top_up_item_failed"), stream_id.clone()),
                        (item_index, code),
                    );
                }
                Err(Err(soroban_sdk::InvokeError::Abort)) => {
                    let code = 9999u32;
                    failed = failed.saturating_add(1);
                    results.push_back(BatchResult {
                        item_index,
                        success: false,
                        error_code: Some(code),
                    });
                    env.events().publish(
                        (Symbol::new(&env, "top_up_item_failed"), stream_id.clone()),
                        (item_index, code),
                    );
                }
            }
        }

        env.storage().temporary().remove(&DataKey::ReentrancyLock);

        env.events().publish(
            (Symbol::new(&env, "batch_top_up_lenient"), caller),
            (succeeded, failed, total_topped_up),
        );

        Ok(results)
    }

    /// Adaptively extend batch processor storage TTL based on the longest-lived
    /// stream in `stream_ids` (Issue #889). Returns the computed ledger bump.
    pub fn bump_ttl_adaptive(env: Env, stream_ids: Vec<Address>) -> u32 {
        ttl::bump_adaptive_for_streams(&env, &stream_ids)
    }

    /// Extend batch processor storage TTL to match target stream expiration (Issue #762).
    pub fn bump_ttl_to_stream_expiration(env: Env, target_stream_ledger: u32) {
        ttl::bump_to_match_stream_expiration(&env, target_stream_ledger);
    }

    /// Automated helper to extend instance and all active temporary keys (Issue #762).
    pub fn bump_instance_and_keys(env: Env) {
        ttl::bump_instance_and_keys(&env);
    }

    /// Admin-gated configuration for batch processor keeper maintenance rebate (Issue #817).
    pub fn configure_maintenance_reserve(
        env: Env,
        caller: Address,
        token: Address,
        rebate_amount: i128,
    ) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if caller != admin {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        ttl::configure_maintenance_reserve(&env, &token, rebate_amount)?;
        Ok(())
    }

    /// Fund the batch processor maintenance reserve pool (Issue #817).
    pub fn fund_maintenance_reserve(env: Env, funder: Address, amount: i128) -> Result<(), Error> {
        ttl::fund_maintenance_reserve(&env, &funder, amount)?;
        Ok(())
    }

    /// Extend batch processor storage TTL and reimburse keeper rebate if reserve is funded (Issue #817).
    pub fn bump_ttl_with_rebate(env: Env, keeper: Address) -> Result<i128, Error> {
        ttl::bump_with_keeper_rebate(&env, &keeper)
    }

    /// Read-only: get current maintenance reserve details (Issue #817).
    pub fn maintenance_reserve(env: Env) -> Option<MaintenanceReserve> {
        ttl::get_maintenance_reserve(&env)
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

        ttl::bump(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
        event_upgraded(&env, &caller, env.ledger().timestamp());
        Ok(())
    }
}
