#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

/// Protocol administration roles for the oracle.
///
/// Separates concerns so independent wallets can own price submission
/// versus oracle configuration:
///
/// - `Admin`       — configure oracle, grant/revoke roles, emergency pause.
/// - `PriceFeeder` — submit prices (or Admin, acting as super-user).
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Admin,
    PriceFeeder,
}

/// Composite key identifying a single (role, account) grant.
#[contracttype]
#[derive(Clone)]
pub struct RoleKey {
    pub role: Role,
    pub account: Address,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Config,
    Price,
    Role(RoleKey),
    AdminCount,
    Paused,
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
    /// The oracle is under an emergency pause; price submission is halted.
    ContractPaused = 1011,
    /// `pause` was called while the oracle was already paused.
    AlreadyPaused = 1012,
    /// `unpause` was called while the oracle was not paused.
    NotPaused = 1013,
    /// Refused to revoke the last `Admin`, which would freeze oracle governance.
    LastAdmin = 1014,
}

#[contract]
pub struct TwapOracle;

#[contractimpl]
impl TwapOracle {
    /// One-time setup — called by the deploy script.
    ///
    /// Grants every role to `admin` so a single wallet can bootstrap the
    /// oracle and later delegate price submission to a separate
    /// `PriceFeeder` wallet via [`TwapOracle::grant_role`].
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        grant_role_inner(&env, Role::Admin, &admin);
        Ok(())
    }

    // ── Role administration (Admin-gated) ────────────────────────────────

    /// Whether `account` currently holds `role`.
    pub fn has_role(env: Env, role: Role, account: Address) -> bool {
        has_role(&env, role, &account)
    }

    /// Grants `role` to `account`. Only an `Admin` may call this.
    pub fn grant_role(
        env: Env,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;
        if grant_role_inner(&env, role, &account) {
            events::grant_role(&env, &caller, role, &account);
        }
        Ok(())
    }

    /// Revokes `role` from `account`. Only an `Admin` may call this.
    ///
    /// Rejected with `LastAdmin` if it would remove the final `Admin`.
    pub fn revoke_role(
        env: Env,
        caller: Address,
        role: Role,
        account: Address,
    ) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;
        if revoke_role_inner(&env, role, &account)? {
            events::revoke_role(&env, &caller, role, &account);
        }
        Ok(())
    }

    // ── Reads ────────────────────────────────────────────────────────────

    pub fn configure_oracle(env: Env, caller: Address, config: OracleConfig) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;

        if config.decimals > 38 {
            return Err(Error::InvalidDecimals);
        }

        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    /// Submit a price observation. Gated on `PriceFeeder` (or `Admin`).
    ///
    /// Blocked while the oracle is under an emergency pause.
    pub fn submit_price(env: Env, caller: Address, price: u64) -> Result<(), Error> {
        if is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        require_role_or_admin(&env, &caller, Role::PriceFeeder)?;

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

    /// Returns the current oracle price, guarded against re-entrancy.
    ///
    /// Errors:
    /// - `OracleNotConfigured` if `configure_oracle` has not been called.
    /// - `NoPriceAvailable` if no price has been submitted yet.
    /// - `OracleStalePrice` if the last submitted price is older than the
    ///   configured `max_staleness`.
    /// - `InvalidPrice` if the stored price is zero.
    /// - `OracleLocked` if called while the re-entrancy guard is already held
    ///   (see the nested-lock warning on `calculate_fiat_stream_payout`).
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

    // ── Emergency pause (Admin-gated) ────────────────────────────────────

    /// Emergency halt: freeze price submission.
    ///
    /// While paused, `submit_price` reverts with `ContractPaused` before
    /// any state is touched. Read-only methods (`get_twap_price`,
    /// `calculate_fiat_stream_payout`) continue to work with the last
    /// committed price.
    pub fn pause(env: Env, caller: Address) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;
        if is_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        set_paused(&env, true);
        events::paused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the emergency pause, allowing `submit_price` again.
    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;
        if !is_paused(&env) {
            return Err(Error::NotPaused);
        }
        set_paused(&env, false);
        events::unpaused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Read-only: whether the oracle is currently under an emergency pause.
    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

fn has_role(env: &Env, role: Role, account: &Address) -> bool {
    let key = DataKey::Role(RoleKey {
        role,
        account: account.clone(),
    });
    env.storage().instance().has(&key)
}

fn grant_role_inner(env: &Env, role: Role, account: &Address) -> bool {
    if has_role(env, role, account) {
        return false;
    }
    let key = DataKey::Role(RoleKey {
        role,
        account: account.clone(),
    });
    env.storage().instance().set(&key, &true);
    if role == Role::Admin {
        let next: u32 = env
            .storage()
            .instance()
            .get(&DataKey::AdminCount)
            .unwrap_or(0)
            + 1;
        env.storage().instance().set(&DataKey::AdminCount, &next);
    }
    true
}

fn revoke_role_inner(env: &Env, role: Role, account: &Address) -> Result<bool, Error> {
    if !has_role(env, role, account) {
        return Ok(false);
    }
    if role == Role::Admin {
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::AdminCount)
            .unwrap_or(0);
        if count <= 1 {
            return Err(Error::LastAdmin);
        }
        env.storage()
            .instance()
            .set(&DataKey::AdminCount, &(count - 1));
    }
    let key = DataKey::Role(RoleKey {
        role,
        account: account.clone(),
    });
    env.storage().instance().remove(&key);
    Ok(true)
}

/// Requires that `caller` authorized the transaction and holds `role` **or**
/// is an `Admin`. Admin acts as a super-user.
fn require_role_or_admin(env: &Env, caller: &Address, role: Role) -> Result<(), Error> {
    caller.require_auth();
    if has_role(env, Role::Admin, caller) || has_role(env, role, caller) {
        Ok(())
    } else {
        Err(Error::NotAuthorized)
    }
}

fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
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

// ── Events ────────────────────────────────────────────────────────────────

mod events {
    use soroban_sdk::{symbol_short, Address, Env};

    use super::Role;

    pub fn grant_role(env: &Env, caller: &Address, role: Role, account: &Address) {
        env.events().publish(
            (symbol_short!("grant"), caller.clone()),
            (role, account.clone()),
        );
    }

    pub fn revoke_role(env: &Env, caller: &Address, role: Role, account: &Address) {
        env.events().publish(
            (symbol_short!("revoke"), caller.clone()),
            (role, account.clone()),
        );
    }

    pub fn paused(env: &Env, caller: &Address, paused_at: u64) {
        env.events()
            .publish((symbol_short!("paused"), caller.clone()), paused_at);
    }

    pub fn unpaused(env: &Env, caller: &Address, resumed_at: u64) {
        env.events()
            .publish((symbol_short!("unpaused"), caller.clone()), resumed_at);
    }
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
    fn initialize_sets_admin_and_grants_role() {
        let (env, client, admin) = setup();
        client.initialize(&admin);
        assert!(client.has_role(&Role::Admin, &admin));
        // Second init should fail
        let result = client.try_initialize(&admin);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn configure_oracle_requires_admin_role() {
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
    fn submit_price_requires_price_feeder_role() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_submit_price(&Address::generate(&env), &100);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    }

    #[test]
    fn admin_can_submit_price() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_submit_price(&admin, &100);
        assert!(result.is_ok());
    }

    #[test]
    fn price_feeder_can_submit_price() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let feeder = Address::generate(&env);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder);

        let result = client.try_submit_price(&feeder, &100);
        assert!(result.is_ok());
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
        let lock_key = soroban_sdk::symbol_short!("O_Lock");
        env.as_contract(&client.address, || {
            env.storage().instance().set(&lock_key, &1_u32);
        });

        let result = client.try_calculate_fiat_stream_payout(&100);
        assert_eq!(result, Err(Ok(Error::OracleLocked)));
    }

    // ── Role management tests ────────────────────────────────────────────

    #[test]
    fn admin_can_grant_and_revoke_price_feeder() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let feeder = Address::generate(&env);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder);
        assert!(client.has_role(&Role::PriceFeeder, &feeder));

        client.revoke_role(&admin, &Role::PriceFeeder, &feeder);
        assert!(!client.has_role(&Role::PriceFeeder, &feeder));
    }

    #[test]
    fn non_admin_cannot_grant_roles() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let stranger = Address::generate(&env);
        let result = client.try_grant_role(&stranger, &Role::PriceFeeder, &Address::generate(&env));
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    }

    #[test]
    fn revoking_last_admin_is_rejected() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_revoke_role(&admin, &Role::Admin, &admin);
        assert_eq!(result, Err(Ok(Error::LastAdmin)));
    }

    #[test]
    fn admin_can_be_revoked_after_granting_second_admin() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let second = Address::generate(&env);
        client.grant_role(&admin, &Role::Admin, &second);
        client.revoke_role(&admin, &Role::Admin, &admin);

        assert!(!client.has_role(&Role::Admin, &admin));
        assert!(client.has_role(&Role::Admin, &second));
    }

    // ── Emergency pause tests ────────────────────────────────────────────

    #[test]
    fn admin_can_pause_and_unpause() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        assert!(!client.is_paused());
        client.pause(&admin);
        assert!(client.is_paused());
        client.unpause(&admin);
        assert!(!client.is_paused());
    }

    #[test]
    fn pause_blocks_submit_price() {
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

        client.pause(&admin);
        let result = client.try_submit_price(&admin, &100);
        assert_eq!(result, Err(Ok(Error::ContractPaused)));
    }

    #[test]
    fn submit_price_works_after_unpause() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        client.pause(&admin);
        client.unpause(&admin);

        let result = client.try_submit_price(&admin, &100);
        assert!(result.is_ok());
    }

    #[test]
    fn double_pause_is_rejected() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        client.pause(&admin);
        let result = client.try_pause(&admin);
        assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
    }

    #[test]
    fn unpause_when_not_paused_is_rejected() {
        let (_env, client, admin) = setup();
        client.initialize(&admin);

        let result = client.try_unpause(&admin);
        assert_eq!(result, Err(Ok(Error::NotPaused)));
    }

    #[test]
    fn get_twap_price_works_while_paused() {
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

        client.pause(&admin);
        // Read-only methods still work with the last committed price.
        let price = client.get_twap_price();
        assert_eq!(price, 50_000_000);
    }
}
