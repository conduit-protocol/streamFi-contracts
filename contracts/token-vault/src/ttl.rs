//! Instance TTL management for `TokenVault`.
//!
//! The threshold/extend-to values are protocol-wide policy and live in
//! [`drip_common::ttl`], so every contract renews instance storage on the same
//! schedule. This module only keeps the crate-local `ttl::bump_instance` call
//! sites unchanged — it must not restate the constants (issue #649).

/// Extends the vault's instance storage TTL so its state stays live and does
/// not archive during idle periods.
///
/// Re-exported from [`drip_common::ttl::bump_instance`] — see the module docs.
pub use drip_common::ttl::bump_instance;
