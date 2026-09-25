#![no_std]

#[cfg(test)]
mod tests;

use drip_common::is_zero_address;
use soroban_sdk::{contract, contracterror, contractimpl, token, Address, Env, Symbol, Vec};

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

/// Errors returned by [`BatchTransferProcessor::process_batch`].
///
/// # Validation order
/// The numeric order of the variants **is** the check order: `process_batch`
/// checks `1`, then `2`, then `3`, and so on, and returns the first failure
/// without evaluating the rest. A client pre-checking a batch before
/// submitting it should apply the same sequence (length → size → per-amount →
/// total → token), mirroring the validation list the README documents for
/// `DripFactory::create_stream`. A batch that is both too large and contains
/// a zero amount always reports `BatchTooLarge`, never `InvalidAmount`.
///
/// All five checks run before `funder.require_auth()` and before any token
/// movement.
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
}

#[contract]
pub struct BatchTransferProcessor;

#[contractimpl]
impl BatchTransferProcessor {
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
    /// # Events
    /// After every transfer completes, a `batch_transferred` event is
    /// published so indexers and off-chain listeners can observe the batch
    /// without diffing token balances. Topics: `[funder]` (the event symbol
    /// `batch_transferred` is topic 0, matching the workspace convention);
    /// data: `{ token, recipient_count, total }`. Rejected calls and the
    /// empty-batch short-circuit emit nothing.
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
}
