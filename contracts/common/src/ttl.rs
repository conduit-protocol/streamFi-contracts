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

/// Standardized TTL configuration parameters (in ledgers) for a target network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TtlConfig {
    /// Minimum remaining TTL (in ledgers) before an entry is extended.
    pub min_ttl_ledgers: u32,
    /// Target TTL (in ledgers) to extend an entry to when below `min_ttl_ledgers`.
    pub extend_ttl_ledgers: u32,
}

/// Testnet minimum TTL threshold: 17,280 ledgers × ~5s/ledger ≈ 1 day.
pub const TESTNET_MIN_TTL_LEDGERS: u32 = 17_280;

/// Testnet target TTL extension: 34,560 ledgers × ~5s/ledger ≈ 2 days.
pub const TESTNET_EXTEND_TTL_LEDGERS: u32 = 34_560;

/// Mainnet minimum TTL threshold: 100,000 ledgers × ~5s/ledger ≈ 5.8 days.
pub const MAINNET_MIN_TTL_LEDGERS: u32 = 100_000;

/// Mainnet target TTL extension: 200,000 ledgers × ~5s/ledger ≈ 11.6 days.
pub const MAINNET_EXTEND_TTL_LEDGERS: u32 = 200_000;

/// Standardized TTL configuration for Stellar Testnet.
pub const TESTNET_TTL_CONFIG: TtlConfig = TtlConfig {
    min_ttl_ledgers: TESTNET_MIN_TTL_LEDGERS,
    extend_ttl_ledgers: TESTNET_EXTEND_TTL_LEDGERS,
};

/// Standardized TTL configuration for Stellar Mainnet.
pub const MAINNET_TTL_CONFIG: TtlConfig = TtlConfig {
    min_ttl_ledgers: MAINNET_MIN_TTL_LEDGERS,
    extend_ttl_ledgers: MAINNET_EXTEND_TTL_LEDGERS,
};

/// Testnet-specific TTL constants module.
pub mod testnet {
    pub const MIN_TTL_LEDGERS: u32 = super::TESTNET_MIN_TTL_LEDGERS;
    pub const EXTEND_TTL_LEDGERS: u32 = super::TESTNET_EXTEND_TTL_LEDGERS;
    pub const CONFIG: super::TtlConfig = super::TESTNET_TTL_CONFIG;
}

/// Mainnet-specific TTL constants module.
pub mod mainnet {
    pub const MIN_TTL_LEDGERS: u32 = super::MAINNET_MIN_TTL_LEDGERS;
    pub const EXTEND_TTL_LEDGERS: u32 = super::MAINNET_EXTEND_TTL_LEDGERS;
    pub const CONFIG: super::TtlConfig = super::MAINNET_TTL_CONFIG;
}

/// Centralized minimum TTL threshold (in ledgers) before storage is extended.
///
/// Defaults to [`MAINNET_MIN_TTL_LEDGERS`] (100,000 ledgers ≈ 5.8 days), or
/// [`TESTNET_MIN_TTL_LEDGERS`] when built with the `testnet` feature enabled
/// (and `mainnet` not enabled).
#[cfg(all(feature = "testnet", not(feature = "mainnet")))]
pub const MIN_TTL_LEDGERS: u32 = TESTNET_MIN_TTL_LEDGERS;

/// Centralized minimum TTL threshold (in ledgers) before storage is extended.
///
/// Defaults to [`MAINNET_MIN_TTL_LEDGERS`] (100,000 ledgers ≈ 5.8 days).
#[cfg(not(all(feature = "testnet", not(feature = "mainnet"))))]
pub const MIN_TTL_LEDGERS: u32 = MAINNET_MIN_TTL_LEDGERS;

/// Centralized target TTL extension (in ledgers) applied when remaining TTL
/// drops below [`MIN_TTL_LEDGERS`].
///
/// Defaults to [`MAINNET_EXTEND_TTL_LEDGERS`] (200,000 ledgers ≈ 11.6 days), or
/// [`TESTNET_EXTEND_TTL_LEDGERS`] when built with the `testnet` feature enabled
/// (and `mainnet` not enabled).
#[cfg(all(feature = "testnet", not(feature = "mainnet")))]
pub const EXTEND_TTL_LEDGERS: u32 = TESTNET_EXTEND_TTL_LEDGERS;

/// Centralized target TTL extension (in ledgers) applied when remaining TTL
/// drops below [`MIN_TTL_LEDGERS`].
///
/// Defaults to [`MAINNET_EXTEND_TTL_LEDGERS`] (200,000 ledgers ≈ 11.6 days).
#[cfg(not(all(feature = "testnet", not(feature = "mainnet"))))]
pub const EXTEND_TTL_LEDGERS: u32 = MAINNET_EXTEND_TTL_LEDGERS;

/// Legacy alias for [`MIN_TTL_LEDGERS`].
pub const TTL_THRESHOLD: u32 = MIN_TTL_LEDGERS;

/// Legacy alias for [`EXTEND_TTL_LEDGERS`].
pub const TTL_EXTEND_TO: u32 = EXTEND_TTL_LEDGERS;

/// Extends the contract's instance storage TTL to [`EXTEND_TTL_LEDGERS`] whenever
/// the remaining TTL drops below [`MIN_TTL_LEDGERS`].
///
/// Called from every state-mutating entry point. A no-op when the contract has
/// no instance storage open (e.g. a read-only invocation), so it is always
/// safe to call first.
pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(MIN_TTL_LEDGERS, EXTEND_TTL_LEDGERS);
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
        .extend_ttl(key, MIN_TTL_LEDGERS, EXTEND_TTL_LEDGERS);
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
    use soroban_sdk::{contract, contractimpl};

    #[contract]
    pub struct DummyContract;

    #[contractimpl]
    impl DummyContract {}

    #[test]
    fn test_ensure_active_ttl_does_not_panic() {
        let env = Env::default();
        let contract_id = env.register_contract(None, DummyContract);
        env.as_contract(&contract_id, || {
            ensure_active_ttl(&env);
            bump_instance_ttl(&env);
        });
    }

    #[test]
    fn test_macro_ensure_active_ttl() {
        let env = Env::default();
        let contract_id = env.register_contract(None, DummyContract);
        env.as_contract(&contract_id, || {
            ensure_active_ttl!(&env);
        });
    }

    #[test]
    fn test_standardized_ttl_constants() {
        const {
            assert!(TESTNET_MIN_TTL_LEDGERS > 0);
            assert!(TESTNET_EXTEND_TTL_LEDGERS > TESTNET_MIN_TTL_LEDGERS);
            assert!(MAINNET_MIN_TTL_LEDGERS > 0);
            assert!(MAINNET_EXTEND_TTL_LEDGERS > MAINNET_MIN_TTL_LEDGERS);
            assert!(EXTEND_TTL_LEDGERS > MIN_TTL_LEDGERS);
        }
        assert_eq!(TTL_THRESHOLD, MIN_TTL_LEDGERS);
        assert_eq!(TTL_EXTEND_TO, EXTEND_TTL_LEDGERS);
        assert_eq!(testnet::MIN_TTL_LEDGERS, TESTNET_MIN_TTL_LEDGERS);
        assert_eq!(testnet::EXTEND_TTL_LEDGERS, TESTNET_EXTEND_TTL_LEDGERS);
        assert_eq!(mainnet::MIN_TTL_LEDGERS, MAINNET_MIN_TTL_LEDGERS);
        assert_eq!(mainnet::EXTEND_TTL_LEDGERS, MAINNET_EXTEND_TTL_LEDGERS);
    }
}
