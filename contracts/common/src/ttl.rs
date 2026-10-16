//! Shared storage-TTL extension helpers for the Drip protocol contracts.
//!
//! Every contract that keeps state in `instance()` storage must renew that
//! storage on its state-mutating paths, or an idle contract is archived by the
//! host and every later call fails with an opaque "entry archived" error. The
//! threshold/extend-to pair that drives those renewals is protocol-wide policy,
//! so it lives here — once — instead of being restated in each contract's
//! `ttl.rs` (four near-identical `bump()` wrappers plus an inline copy in
//! `TwapOracle`, which had drifted onto its own hardcoded literals).
//!
//! A per-contract `ttl.rs` may still exist to keep a crate-local call-site
//! name stable, but it must delegate here rather than re-deriving the
//! constants: a duplicated literal is exactly the kind of drift this module
//! exists to prevent.

use soroban_sdk::Env;

use crate::rbac::StorageKey;

pub use crate::{TTL_EXTEND_TO, TTL_THRESHOLD};

/// Extends the contract's instance storage TTL to [`TTL_EXTEND_TO`] whenever
/// the remaining TTL drops below [`TTL_THRESHOLD`].
///
/// Called from every state-mutating entry point. A no-op when the contract has
/// no instance storage open (e.g. a read-only invocation), so it is always
/// safe to call first.
pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

/// Alias for [`bump_instance`] (Issue #683).
pub use bump_instance as bump_instance_ttl;

/// Extends the TTL of a single `persistent()` entry under `key` on the same
/// schedule as [`bump_instance`].
///
/// Persistent entries are per-entity registry rows (e.g. the factory's
/// `StreamAddr(id)` map) and are not covered by an instance bump, so each one
/// has to be renewed on access. Callers must confirm the entry exists first
/// (`has` / `get`) — `extend_ttl` on a missing entry is a host error, not a
/// silent no-op.
pub fn bump_persistent<K: StorageKey>(env: &Env, key: &K) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

/// Alias for [`bump_persistent`] (Issue #683).
pub use bump_persistent as bump_persistent_ttl;

/// Automatically checks and renews the instance TTL if within threshold (Issue #683).
/// Safe to call on state reads or writes to keep active entries from expiring.
pub fn ensure_active_ttl(env: &Env) {
    bump_instance(env);
}

/// Automatically checks and renews persistent storage entry TTL under `key` (Issue #683).
pub fn ensure_persistent_active_ttl<K: StorageKey>(env: &Env, key: &K) {
    bump_persistent(env, key);
}

/// Helper macro to ensure active TTL on instance or persistent storage (Issue #683).
///
/// Usage:
/// - `ensure_active_ttl!(env);` for instance storage
/// - `ensure_active_ttl!(env, &key);` for persistent storage
#[macro_export]
macro_rules! ensure_active_ttl {
    ($env:expr) => {
        $crate::ttl::bump_instance($env);
    };
    ($env:expr, $key:expr) => {
        $crate::ttl::bump_persistent($env, $key);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_active_ttl_does_not_panic() {
        let env = Env::default();
        ensure_active_ttl(&env);
        bump_instance_ttl(&env);
    }

    #[test]
    fn test_macro_ensure_active_ttl() {
        let env = Env::default();
        ensure_active_ttl!(&env);
    }
}
