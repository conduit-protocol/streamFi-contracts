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
