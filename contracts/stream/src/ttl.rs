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
