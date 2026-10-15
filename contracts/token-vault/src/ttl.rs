//! Instance and persistent TTL management for `TokenVault`.
//!
//! The threshold/extend-to values are protocol-wide policy and live in
//! [`drip_common::ttl`], so every contract renews storage on the same
//! schedule.

pub use drip_common::ttl::{bump_instance, bump_persistent};
