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

use soroban_sdk::{contracterror, contracttype, token, Address, Env};

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

/// Fixed protocol reward (in stroops) paid for each key whose TTL is extended
/// through the keeper reward pool.
pub const DEFAULT_KEEPER_REWARD_PER_KEY: i128 = 1_000_000;

/// On-chain pool that pays keepers for maintaining persistent storage.
///
/// `balance` is the amount of the configured reward token escrowed for TTL
/// maintenance, and `reward_per_key` is the fixed amount paid for every
/// successful [`bump_persistent_with_reward`] call. Contracts store this value
/// under their own instance-storage key, so the pool stays optional per
/// contract.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeeperRewardPool {
    pub reward_per_key: i128,
    pub balance: i128,
}

/// Errors returned by the keeper reward-pool helpers.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum KeeperRewardError {
    /// The reward per key is zero or negative.
    InvalidReward = 1,
    /// The supplied funding amount is zero or negative.
    InvalidAmount = 2,
    /// The pool does not hold enough tokens to pay the next reward.
    InsufficientPoolBalance = 3,
}

/// Creates or replaces the keeper reward pool under `pool_key`.
///
/// The pool starts with a zero balance; fund it with
/// [`fund_keeper_reward_pool`]. The reward must be positive so every successful
/// bump has a non-zero incentive.
pub fn configure_keeper_reward_pool<PK: StorageKey>(
    env: &Env,
    pool_key: &PK,
    reward_per_key: i128,
) -> Result<KeeperRewardPool, KeeperRewardError> {
    if reward_per_key <= 0 {
        return Err(KeeperRewardError::InvalidReward);
    }

    let pool = KeeperRewardPool {
        reward_per_key,
        balance: 0,
    };
    env.storage().instance().set(pool_key, &pool);
    Ok(pool)
}

/// Reads the keeper reward pool stored under `pool_key`.
pub fn keeper_reward_pool<PK: StorageKey>(env: &Env, pool_key: &PK) -> Option<KeeperRewardPool> {
    env.storage().instance().get(pool_key)
}

/// Transfers `amount` of `token` from `funder` into the contract and adds it to
/// the reward pool's balance.
pub fn fund_keeper_reward_pool<PK: StorageKey>(
    env: &Env,
    pool_key: &PK,
    token: &Address,
    funder: &Address,
    amount: i128,
) -> Result<KeeperRewardPool, KeeperRewardError> {
    if amount <= 0 {
        return Err(KeeperRewardError::InvalidAmount);
    }

    let mut pool: KeeperRewardPool = env
        .storage()
        .instance()
        .get(pool_key)
        .ok_or(KeeperRewardError::InvalidReward)?;
    pool.balance = pool
        .balance
        .checked_add(amount)
        .ok_or(KeeperRewardError::InvalidAmount)?;

    funder.require_auth();
    token::Client::new(env, token).transfer(funder, &env.current_contract_address(), &amount);
    env.storage().instance().set(pool_key, &pool);
    Ok(pool)
}

/// Extends the persistent entry under `key` and pays the keeper the pool's
/// fixed reward from the contract's escrowed balance.
///
/// The entry must exist before this call — `extend_ttl` on a missing entry is a
/// host error. The reward transfer happens only after the TTL bump, so the
/// keeper cannot be paid for a bump that did not occur. Returns the amount paid.
pub fn bump_persistent_with_reward<K: StorageKey, PK: StorageKey>(
    env: &Env,
    key: &K,
    pool_key: &PK,
    token: &Address,
    keeper: &Address,
) -> Result<i128, KeeperRewardError> {
    let mut pool: KeeperRewardPool = env
        .storage()
        .instance()
        .get(pool_key)
        .ok_or(KeeperRewardError::InvalidReward)?;

    if pool.reward_per_key <= 0 {
        return Err(KeeperRewardError::InvalidReward);
    }
    if pool.balance < pool.reward_per_key {
        return Err(KeeperRewardError::InsufficientPoolBalance);
    }

    bump_persistent(env, key);

    token::Client::new(env, token).transfer(
        &env.current_contract_address(),
        keeper,
        &pool.reward_per_key,
    );

    pool.balance -= pool.reward_per_key;
    env.storage().instance().set(pool_key, &pool);
    Ok(pool.reward_per_key)
}

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
    use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, token};

    #[contract]
    pub struct DummyContract;

    #[contractimpl]
    impl DummyContract {}

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestKey {
        RewardPool,
        PersistentEntry,
    }

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
    fn keeper_reward_pool_pays_fixed_reward_per_bumped_key() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, DummyContract);
        let keeper = Address::generate(&env);
        let funder = Address::generate(&env);
        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        token::StellarAssetClient::new(&env, &token_addr).mint(&funder, &5_000_000);

        env.as_contract(&contract_id, || {
            configure_keeper_reward_pool(&env, &TestKey::RewardPool, DEFAULT_KEEPER_REWARD_PER_KEY)
                .unwrap();
            fund_keeper_reward_pool(&env, &TestKey::RewardPool, &token_addr, &funder, 5_000_000)
                .unwrap();
            env.storage()
                .persistent()
                .set(&TestKey::PersistentEntry, &1u32);

            let paid = bump_persistent_with_reward(
                &env,
                &TestKey::PersistentEntry,
                &TestKey::RewardPool,
                &token_addr,
                &keeper,
            )
            .unwrap();
            assert_eq!(paid, DEFAULT_KEEPER_REWARD_PER_KEY);

            let pool = keeper_reward_pool(&env, &TestKey::RewardPool).unwrap();
            assert_eq!(pool.balance, 5_000_000 - DEFAULT_KEEPER_REWARD_PER_KEY);
        });

        assert_eq!(
            token::Client::new(&env, &token_addr).balance(&keeper),
            DEFAULT_KEEPER_REWARD_PER_KEY
        );
    }

    #[test]
    fn keeper_reward_pool_rejects_a_bump_when_unfunded() {
        let env = Env::default();
        let contract_id = env.register_contract(None, DummyContract);
        let keeper = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(Address::generate(&env))
            .address();

        env.as_contract(&contract_id, || {
            configure_keeper_reward_pool(&env, &TestKey::RewardPool, DEFAULT_KEEPER_REWARD_PER_KEY)
                .unwrap();
            env.storage()
                .persistent()
                .set(&TestKey::PersistentEntry, &1u32);

            assert_eq!(
                bump_persistent_with_reward(
                    &env,
                    &TestKey::PersistentEntry,
                    &TestKey::RewardPool,
                    &token_addr,
                    &keeper,
                ),
                Err(KeeperRewardError::InsufficientPoolBalance)
            );
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
