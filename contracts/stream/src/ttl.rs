//! Instance TTL management for `DripStream`.
//!
//! The threshold/extend-to values are protocol-wide policy and live in
//! [`drip_common::ttl`], so every contract renews instance storage on the same
//! schedule. This module only keeps the crate-local `ttl::bump` call sites
//! unchanged — it must not restate the constants (issue #649).

/// Maximum safe duration a stream may remain paused before the instance
/// storage TTL window is no longer sufficient to resume it safely.
///
/// A single `extend_ttl` bump only renews the instance record to
/// `TTL_EXTEND_TO` (200_000 ledgers). Any pause that exceeds the window can
/// leave the stream archived before a normal `resume()` or `force_cancel()`
/// call can run.
pub const MAX_PAUSE_SECS: u64 = 2_592_000; // 30 days

/// Approximate ledger close time used to convert a stream's remaining
/// wall-clock duration into ledgers.
pub const SECONDS_PER_LEDGER: u64 = 5;

/// Protocol maximum entry TTL (6_312_000 ledgers, ~1 year at 5s/ledger).
pub const MAX_TTL_LEDGERS: u32 = 6_312_000;

/// Dynamic `extend_to` for a stream that still has `remaining_secs` to run.
///
/// Short streams keep the protocol-wide 200k-ledger target; long streams get
/// at least their remaining lifetime plus the normal threshold buffer, capped
/// at the host maximum. This keeps a five-year stream alive without forcing
/// every short stream to pay for a year of TTL.
pub fn extend_to_for_remaining(remaining_secs: u64) -> u32 {
    let remaining_ledgers = remaining_secs
        .div_ceil(SECONDS_PER_LEDGER)
        .min(u32::MAX as u64) as u32;
    remaining_ledgers
        .saturating_add(drip_common::ttl::MIN_TTL_LEDGERS)
        .clamp(drip_common::ttl::EXTEND_TTL_LEDGERS, MAX_TTL_LEDGERS)
}

/// Extends the instance TTL using the stream's remaining duration (#846).
pub fn bump_for_remaining(env: &soroban_sdk::Env, remaining_secs: u64) {
    env.storage().instance().extend_ttl(
        drip_common::ttl::MIN_TTL_LEDGERS,
        extend_to_for_remaining(remaining_secs),
    );
}

/// Extends the stream's instance storage TTL.
///
/// Re-exported from [`drip_common::ttl::bump_instance`] — see the module docs.
pub use drip_common::ttl::bump_instance as bump;

/// Minimum threshold and bump target for instance TTL renewals on activity (Issue #750).
pub const LEDGER_THRESHOLD: u32 = drip_common::ttl::MIN_TTL_LEDGERS;
pub const LEDGER_BUMP: u32 = drip_common::ttl::EXTEND_TTL_LEDGERS;

/// Automatic instance TTL extension on withdraw and top_up (Issue #750).
pub fn extend_on_activity(env: &soroban_sdk::Env) {
    env.storage()
        .instance()
        .extend_ttl(LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn extend_persistent_on_activity(env: &soroban_sdk::Env) {
    for key in [
        crate::storage::DataKey::MetadataUri,
        crate::storage::DataKey::CancelledRecord,
    ] {
        if env.storage().persistent().has(&key) {
            drip_common::ttl::bump_persistent(env, &key);
        }
    }
}

/// Emergency instance TTL extension by any sender, recipient, or third-party (Issue #805).
/// Bumps the instance storage TTL without requiring withdrawals or state mutations.
pub fn extend_stream_ttl(env: &soroban_sdk::Env) {
    if let Ok(info) = crate::state::try_load(env) {
        let remaining = if info.end_time > 0 {
            info.end_time.saturating_sub(env.ledger().timestamp())
        } else {
            0
        };
        bump_for_remaining(env, remaining);
    } else {
        bump(env);
    }

    // Keep persistent side records alive with the instance as well (#945).
    for key in [
        crate::storage::DataKey::MetadataUri,
        crate::storage::DataKey::CancelledRecord,
    ] {
        if env.storage().persistent().has(&key) {
            drip_common::ttl::bump_persistent(env, &key);
        }
    }
}

/// Emergency keeper renewal with caller-specified TTL threshold and extend
/// target, guarded by protocol sanity bounds (Issue #887).
///
/// # Bounds
/// - `min_ledgers_to_live > 0`
/// - `ledgers_to_extend > 0`
/// - `min_ledgers_to_live <= ledgers_to_extend`
/// - `ledgers_to_extend <= MAX_TTL_LEDGERS`
pub fn emergency_extend_custom(
    env: &soroban_sdk::Env,
    min_ledgers_to_live: u32,
    ledgers_to_extend: u32,
) -> Result<(), crate::errors::Error> {
    if min_ledgers_to_live == 0
        || ledgers_to_extend == 0
        || min_ledgers_to_live > ledgers_to_extend
        || ledgers_to_extend > MAX_TTL_LEDGERS
    {
        return Err(crate::errors::Error::InvalidAmount);
    }
    env.storage()
        .instance()
        .extend_ttl(min_ledgers_to_live, ledgers_to_extend);
    Ok(())
}

/// Total schedule length (one year) from which a stream counts as a
/// multi-year vesting schedule for `bump_long_term_stream_ttl` (#880).
pub const LONG_TERM_STREAM_MIN_SECS: u64 = 31_536_000;

/// Keeper heartbeat for multi-year vesting streams (#880).
///
/// Long grants can go months without a withdrawal, letting the instance TTL
/// run out. Any relayer may call this to extend the instance TTL with the
/// given threshold and target, so participants never need an emergency
/// restore before withdrawing.
///
/// Only an active (not cancelled, not yet ended) bounded stream whose
/// schedule spans at least [`LONG_TERM_STREAM_MIN_SECS`] qualifies; others
/// get `StreamNotLongTerm`. Bounds match `emergency_extend_custom`.
pub fn bump_long_term_stream_ttl(
    env: &soroban_sdk::Env,
    threshold: u32,
    bump_ledgers: u32,
) -> Result<(), crate::errors::Error> {
    use crate::errors::Error;

    if threshold == 0
        || bump_ledgers == 0
        || threshold > bump_ledgers
        || bump_ledgers > MAX_TTL_LEDGERS
    {
        return Err(Error::InvalidAmount);
    }

    let info = crate::state::try_load(env)?;
    crate::state::assert_not_cancelled(&info)?;
    let schedule_secs = info.end_time.saturating_sub(info.start_time);
    if info.end_time == 0
        || schedule_secs < LONG_TERM_STREAM_MIN_SECS
        || env.ledger().timestamp() >= info.end_time
    {
        return Err(Error::StreamNotLongTerm);
    }

    env.storage().instance().extend_ttl(threshold, bump_ledgers);
    Ok(())
}

/// Low-cost keeper maintenance function for stream instances.
///
/// Extends the TTL of an active stream's instance storage when the remaining
/// TTL falls below `TTL_THRESHOLD`. This allows keepers or automated processes
/// to maintain stream instances with minimal gas cost, avoiding archival of
/// paused streams during long-duration pauses.
///
/// # Arguments
/// - `env` — Soroban environment
/// - `stream_id` — The unique identifier of the stream to bump
///
/// # Errors
/// - `StreamNotFound` if the stream instance does not exist
#[allow(dead_code)]
pub fn bump_stream_instance(
    env: &soroban_sdk::Env,
    stream_id: &str,
) -> Result<(), crate::errors::Error> {
    use crate::storage::DataKey;

    let _ = stream_id;

    if !env.storage().instance().has(&DataKey::Config) {
        return Err(crate::errors::Error::StreamNotFound);
    }

    let info = crate::state::load(env);
    let remaining = if info.end_time > 0 {
        info.end_time.saturating_sub(env.ledger().timestamp())
    } else {
        0
    };
    bump_for_remaining(env, remaining);
    Ok(())
}

/// Verifies that an active stream's persistent storage TTL is above the protocol minimum.
///
/// For long-running streams spanning multiple months, this ensures that critical
/// persistent data entries (MetadataUri, CancelledRecord) maintain their TTL above
/// the protocol threshold to prevent state expiration during extended periods of
/// recipient inactivity.
///
/// # Arguments
/// - `env` — Soroban environment
/// - `threshold_ledgers` — Minimum acceptable remaining TTL in ledgers
///
/// # Returns
/// - `Ok(())` if all persistent entries meet or exceed the threshold
/// - `Err(Error::DataExpired)` if any persistent entry falls below threshold
///
/// # Note
/// This helper should be called after stream interactions to verify that
/// `extend_stream_ttl` or similar bumps have properly renewed persistent storage.
pub fn verify_persistent_ttl_above_threshold(
    env: &soroban_sdk::Env,
    threshold_ledgers: u32,
) -> Result<(), crate::errors::Error> {
    use crate::storage::DataKey;

    let keys_to_check = [
        DataKey::MetadataUri,
        DataKey::CancelledRecord,
    ];

    for key in &keys_to_check {
        if env.storage().persistent().has(key) {
            let remaining_ttl = env.storage().persistent().get_ttl(key)
                .ok_or(crate::errors::Error::DataExpired)?;
            if remaining_ttl < threshold_ledgers {
                return Err(crate::errors::Error::DataExpired);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod duration_ttl_tests {
    use super::*;

    #[test]
    fn short_streams_keep_the_default_extend_target() {
        assert_eq!(
            extend_to_for_remaining(0),
            drip_common::ttl::EXTEND_TTL_LEDGERS
        );
        assert_eq!(
            extend_to_for_remaining(60),
            drip_common::ttl::EXTEND_TTL_LEDGERS
        );
    }

    #[test]
    fn longer_streams_scale_the_extend_target_and_clamp_at_the_host_max() {
        let medium = extend_to_for_remaining(30 * 24 * 60 * 60);
        assert!(medium > extend_to_for_remaining(3_600));
        assert_eq!(
            extend_to_for_remaining(5 * 365 * 24 * 60 * 60),
            MAX_TTL_LEDGERS
        );
        assert!(extend_to_for_remaining(u64::MAX) <= MAX_TTL_LEDGERS);
    }
}
