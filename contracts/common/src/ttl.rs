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
