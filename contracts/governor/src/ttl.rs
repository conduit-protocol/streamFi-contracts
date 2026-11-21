//! Instance TTL management for `DripGovernor`.
//!
//! The threshold/extend-to values are protocol-wide policy and live in
//! [`drip_common::ttl`], so every contract renews instance storage on the same
//! schedule. This module only keeps the crate-local `ttl::bump` call sites
//! unchanged — it must not restate the constants (issue #649).

/// Extends the governor's instance storage TTL.
///
/// Re-exported from [`drip_common::ttl::bump_instance`]; kept under the
/// `bump` name because the governor's rbac `on_success` hook is wired to
/// `Some(ttl::bump)` (see `drip_common::rbac::require_role_or_admin`).
pub use drip_common::ttl::bump_instance as bump;

/// Extends proposal storage TTL to cover the entire proposal lifecycle.
///
/// Ensures proposal entries remain available through the execution window to prevent
/// storage expiration before execution completes. Extends TTL by timelock_delay +
/// execution_window_secs to cover the full proposal lifetime from creation through
/// potential delayed execution.
pub fn extend_for_proposal(env: &soroban_sdk::Env, timelock_secs: u64, execution_window_secs: u64) {
    // Stellar closes a ledger roughly every five seconds. Round up so the
    // requested wall-clock lifetime is never shortened by conversion.
    const SECONDS_PER_LEDGER: u64 = 5;
    let total_secs = timelock_secs.saturating_add(execution_window_secs);
    let lifecycle_ledgers = total_secs
        .saturating_add(SECONDS_PER_LEDGER - 1)
        / SECONDS_PER_LEDGER;
    let lifecycle_ledgers = u32::try_from(lifecycle_ledgers).unwrap_or(u32::MAX);

    // Keep the normal protocol TTL floor, but if a proposal can live longer,
    // raise both the threshold and target to that full lifecycle.
    let target = lifecycle_ledgers.max(drip_common::ttl::EXTEND_TTL_LEDGERS);
    drip_common::ttl::check_and_bump_ttl(env, target, target);
}
