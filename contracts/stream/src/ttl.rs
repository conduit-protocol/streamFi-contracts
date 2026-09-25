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

/// Extends the stream's instance storage TTL.
///
/// Re-exported from [`drip_common::ttl::bump_instance`] — see the module docs.
pub use drip_common::ttl::bump_instance as bump;
