#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

#[contracttype]
pub enum DataKey {
    Admin,
    Config,
    Price,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle_address: Address,
    pub decimals: u32,
    pub asset_peg: u32,
    pub max_staleness: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: u64,
    pub updated_at: u64,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    OracleStalePrice = 1001,
    OracleNotConfigured = 1002,
    InvalidPrice = 1003,
    OracleLocked = 1004,
    CalculationOverflow = 1005,
    NotAuthorized = 1006,
    AlreadyInitialized = 1007,
    NoPriceAvailable = 1008,
    ArithmeticOverflow = 1009,
    InvalidDecimals = 1010,
}

#[contract]
pub struct TwapOracle;

#[contractimpl]
impl TwapOracle {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    pub fn configure_oracle(env: Env, caller: Address, config: OracleConfig) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if config.decimals > 38 {
            return Err(Error::InvalidDecimals);
        }

        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn submit_price(env: Env, caller: Address, price: u64) -> Result<(), Error> {
        require_admin(&env, &caller)?;

        if price == 0 {
            return Err(Error::InvalidPrice);
        }

        let data = PriceData {
            price,
            updated_at: env.ledger().timestamp(),
        };
        env.storage().instance().set(&DataKey::Price, &data);
        Ok(())
    }

    pub fn get_twap_price(env: Env) -> Result<u64, Error> {
        with_guard(&env, || {
            let config: OracleConfig = env
                .storage()
                .instance()
                .get(&DataKey::Config)
                .ok_or(Error::OracleNotConfigured)?;

            let data: PriceData = env
                .storage()
                .instance()
                .get(&DataKey::Price)
                .ok_or(Error::NoPriceAvailable)?;

            let age = env.ledger().timestamp().saturating_sub(data.updated_at);
            if age > config.max_staleness {
                return Err(Error::OracleStalePrice);
            }

            if data.price == 0 {
                return Err(Error::InvalidPrice);
            }

            Ok(data.price)
        })
    }

    /// Converts a nominal token amount into its fiat equivalent.
    ///
    /// **Nested-lock warning:** This method calls `get_twap_price`
    /// internally, which acquires and releases its own re-entrancy guard.
    /// If this method is ever wrapped in an outer guard (e.g., via a
    /// `with_guard` call), the depth counter will already be at 1 and the
    /// nested `get_twap_price` call will deadlock with `OracleLocked`.
    /// Do NOT add a guard to this function without first refactoring
    /// `get_twap_price` to accept an optional pre-acquired lock.
    pub fn calculate_fiat_stream_payout(env: Env, token_amount: u64) -> Result<u64, Error> {
        let current_price = Self::get_twap_price(env.clone())?;

        let config: OracleConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(Error::OracleNotConfigured)?;

        let precision = 10u128
            .checked_pow(config.decimals)
            .ok_or(Error::InvalidDecimals)?;

        let value = (token_amount as u128)
            .checked_mul(current_price as u128)
            .ok_or(Error::ArithmeticOverflow)?
            .checked_div(precision)
            .ok_or(Error::ArithmeticOverflow)?;

        if value > u64::MAX as u128 {
            return Err(Error::ArithmeticOverflow);
        }

        Ok(value as u64)
    }
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::OracleNotConfigured)?;
    if *caller != admin {
        return Err(Error::NotAuthorized);
    }
    caller.require_auth();
    Ok(())
}

/// Execute `f` under the re-entrancy guard, releasing the lock afterwards.
///
/// Uses a depth counter stored at the `O_Lock` symbol key instead of a
/// boolean flag. See `drip_stream::state::with_guard` for the rationale.
const MAX_REENTRANCY_DEPTH: u32 = 1;

fn with_guard<R>(env: &Env, f: impl FnOnce() -> Result<R, Error>) -> Result<R, Error> {
    let lock_key = soroban_sdk::symbol_short!("O_Lock");
    let depth: u32 = env.storage().instance().get(&lock_key).unwrap_or(0);
    if depth >= MAX_REENTRANCY_DEPTH {
        return Err(Error::OracleLocked);
    }
    env.storage().instance().set(&lock_key, &(depth + 1));
    let result = f();
    let d: u32 = env.storage().instance().get(&lock_key).unwrap_or(1);
    if d > 0 {
        env.storage().instance().set(&lock_key, &(d - 1));
    }
    result
}

#[cfg(test)]
mod tests {
    extern crate std;

    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env,
    };

    use super::*;

    fn setup() -> (Env, TwapOracleClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, TwapOracle);
        let client = TwapOracleClient::new(&env, &contract_id);
        (env, client, admin)
    }

    #[test]
    fn initialize_sets_admin() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);
        // Second init should fail
        let result = client.try_initialize(&admin);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn configure_oracle_requires_admin() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);

        let non_admin = Address::generate(&env);
        let result = client.try_configure_oracle(&non_admin, &config);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    }

    #[test]
    fn configure_oracle_rejects_excessive_decimals() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 39,
            asset_peg: 1,
            max_staleness: 300,
        };
        let result = client.try_configure_oracle(&admin, &config);
        assert_eq!(result, Err(Ok(Error::InvalidDecimals)));
    }

    #[test]
    fn submit_price_requires_admin() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_submit_price(&Address::generate(&env), &100);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    }

    #[test]
    fn submit_price_rejects_zero() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_submit_price(&admin, &0);
        assert_eq!(result, Err(Ok(Error::InvalidPrice)));
    }

    #[test]
    fn get_twap_price_requires_config() {
        let (_env, client, _admin) = setup();
        let result = client.try_get_twap_price();
        assert_eq!(result, Err(Ok(Error::OracleNotConfigured)));
    }

    #[test]
    fn get_twap_price_requires_price_submission() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);

        let result = client.try_get_twap_price();
        assert_eq!(result, Err(Ok(Error::NoPriceAvailable)));
    }

    #[test]
    fn get_twap_price_rejects_stale_price() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 60,
        };
        client.configure_oracle(&admin, &config);

        client.submit_price(&admin, &50_000_000);

        // Advance time beyond max_staleness
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 61,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });

        let result = client.try_get_twap_price();
        assert_eq!(result, Err(Ok(Error::OracleStalePrice)));
    }

    #[test]
    fn get_twap_price_returns_fresh_price() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);

        client.submit_price(&admin, &50_000_000);

        let price = client.get_twap_price();
        assert_eq!(price, 50_000_000);
    }

    #[test]
    fn calculate_fiat_stream_payout_works() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);

        // Price = 50_000_000 with 8 decimals = $0.50 per token
        client.submit_price(&admin, &50_000_000);

        // 100 tokens * 50_000_000 / 10^8 = 50
        let payout = client.calculate_fiat_stream_payout(&100);
        assert_eq!(payout, 50);
    }

    #[test]
    fn calculate_fiat_stream_payout_overflow() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 0,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);

        client.submit_price(&admin, &u64::MAX);

        let result = client.try_calculate_fiat_stream_payout(&(u64::MAX));
        assert_eq!(result, Err(Ok(Error::ArithmeticOverflow)));
    }

    /// Regression test for the nested-lock warning on
    /// `calculate_fiat_stream_payout`.
    ///
    /// `calculate_fiat_stream_payout` calls `get_twap_price` internally,
    /// which acquires its own re-entrancy guard. If this method were ever
    /// wrapped in an outer `with_guard` call, the depth counter would
    /// already be at 1 and the nested `get_twap_price` call would
    /// deadlock with `OracleLocked`.
    ///
    /// This test documents/locks in that invariant so a future refactor
    /// that accidentally adds an outer guard is caught immediately.
    #[test]
    fn calculate_fiat_stream_payout_deadlocks_when_outer_lock_held() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);
        client.submit_price(&admin, &50_000_000);

        // Verify the happy path works without a held lock.
        let payout = client.calculate_fiat_stream_payout(&100);
        assert_eq!(payout, 50);

        // Simulate an outer re-entrancy guard already held at depth 1.
        // This is exactly what would happen if someone wrapped
        // `calculate_fiat_stream_payout` in a `with_guard` call.
        let lock_key = soroban_sdk::symbol_short!("O_Lock");
        env.as_contract(&client.address, || {
            env.storage().instance().set(&lock_key, &1_u32);
        });

        // The nested `get_twap_price` call inside
        // `calculate_fiat_stream_payout` must now deadlock.
        let result = client.try_calculate_fiat_stream_payout(&100);
        assert_eq!(result, Err(Ok(Error::OracleLocked)));
    }
}
