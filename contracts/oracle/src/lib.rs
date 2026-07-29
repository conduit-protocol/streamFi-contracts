#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

/// TTL extension constants matching the convention used across sibling
/// contracts (factory, governor, stream). `bump_instance` is called from
/// every state-mutating entry point so instance storage entries
/// (`DataKey::Admin`, `DataKey::Config`, `DataKey::Price`, etc.) never
/// silently archive during idle periods.
const THRESHOLD: u32 = 100_000;
const EXTEND_TO: u32 = 200_000;

fn bump_instance(env: &Env) {
    env.storage().instance().extend_ttl(THRESHOLD, EXTEND_TO);
}

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
    /// Most recent submission from a single feeder, keyed by feeder address.
    /// Aggregated (median) across every address in `Submitters` by
    /// `get_twap_price`, so no single feeder's price is trusted alone.
    Submission(Address),
    /// Every address that has ever called `submit_price`, iterated by
    /// `get_twap_price` to build the aggregation set.
    Submitters,
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
        bump_instance(&env);
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
        bump_instance(&env);
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
        bump_instance(&env);
        if revoke_role_inner(&env, role, &account)? {
            events::revoke_role(&env, &caller, role, &account);
        }
        Ok(())
    }

    /// Moves the `Admin` role from `caller` to `new_admin` atomically, so
    /// oracle governance is never permanently stuck on a single key — see
    /// issue #192. Equivalent to `grant_role(Admin, new_admin)` followed by
    /// `revoke_role(Admin, caller)`, but as one call so the contract never
    /// passes through a state where `caller` has been revoked without
    /// `new_admin` already holding the role.
    pub fn transfer_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;
        bump_instance(&env);

        grant_role_inner(&env, Role::Admin, &new_admin);
        revoke_role_inner(&env, Role::Admin, &caller)?;

        events::admin_transferred(&env, &caller, &new_admin);
        Ok(())
    }

    // ── Reads ────────────────────────────────────────────────────────────

    /// Reconfigure the oracle parameters. Admin-gated.
    ///
    /// When `decimals` or `asset_peg` changes relative to the currently stored
    /// config, all existing price data (`DataKey::Price`, per-feeder
    /// `DataKey::Submission` entries, and the `DataKey::Submitters` list) is
    /// cleared. This prevents stale prices submitted under the old config from
    /// being silently misinterpreted under the new parameters — the next
    /// `get_twap_price` call will return `NoPriceAvailable` until a fresh
    /// `submit_price` is made. Changes to `max_staleness` or `oracle_address`
    /// alone do not clear price data, as those do not affect price magnitude
    /// interpretation.
    pub fn configure_oracle(env: Env, caller: Address, config: OracleConfig) -> Result<(), Error> {
        require_role_or_admin(&env, &caller, Role::Admin)?;

        if config.decimals > 38 {
            return Err(Error::InvalidDecimals);
        }

        bump_instance(&env);

        // Check if decimals or asset_peg changed relative to existing config.
        // If so, clear all stored price data to prevent magnitude misinterpretation.
        let existing: Option<OracleConfig> = env.storage().instance().get(&DataKey::Config);
        if let Some(old) = existing {
            if old.decimals != config.decimals || old.asset_peg != config.asset_peg {
                // Clear the legacy single-value price slot.
                env.storage().instance().remove(&DataKey::Price);

                // Clear every per-feeder submission and the submitter list itself.
                let submitters: Vec<Address> = env
                    .storage()
                    .instance()
                    .get(&DataKey::Submitters)
                    .unwrap_or(Vec::new(&env));
                for feeder in submitters.iter() {
                    env.storage()
                        .instance()
                        .remove(&DataKey::Submission(feeder));
                }
                env.storage().instance().remove(&DataKey::Submitters);
            }
        }

        env.storage().instance().set(&DataKey::Config, &config);
        events::oracle_configured(&env, &caller, config);
        Ok(())
    }

    /// Submit a price observation. Gated on `PriceFeeder` (or `Admin`).
    ///
    /// Blocked while the oracle is under an emergency pause. Each feeder's
    /// submission is tracked independently (`DataKey::Submission`) and
    /// aggregated by `get_twap_price` — no single feeder's price is trusted
    /// unconditionally.
    pub fn submit_price(env: Env, caller: Address, price: u64) -> Result<(), Error> {
        if is_paused(&env) {
            return Err(Error::ContractPaused);
        }
        require_role_or_admin(&env, &caller, Role::PriceFeeder)?;

        if price == 0 {
            return Err(Error::InvalidPrice);
        }

        let now = env.ledger().timestamp();
        let data = PriceData {
            price,
            updated_at: now,
        };

        bump_instance(&env);
        // Legacy single-value slot — kept so `price_age`/`is_price_stale`
        // and any external readers of the old scalar `Price` key continue
        // to see the most recent submission.
        env.storage().instance().set(&DataKey::Price, &data);

        env.storage()
            .instance()
            .set(&DataKey::Submission(caller.clone()), &data);
        add_submitter(&env, &caller);

        events::price_submitted(&env, &caller, price, now);
        Ok(())
    }

    /// Returns the current oracle price, guarded against re-entrancy.
    ///
    /// Aggregates every non-stale submission across all addresses that have
    /// ever called `submit_price` (median, or the average of the two
    /// middle values when there is an even count) rather than trusting the
    /// single most recent submitter unconditionally — see issue #194.
    ///
    /// Errors:
    /// - `OracleNotConfigured` if `configure_oracle` has not been called.
    /// - `NoPriceAvailable` if no price has been submitted yet.
    /// - `OracleStalePrice` if every submission on record is older than the
    ///   configured `max_staleness`.
    /// - `OracleLocked` if called while the re-entrancy guard is already held
    ///   (see the nested-lock warning on `calculate_fiat_stream_payout`).
    pub fn get_twap_price(env: Env) -> Result<u64, Error> {
        with_guard(&env, || {
            let config: OracleConfig = env
                .storage()
                .instance()
                .get(&DataKey::Config)
                .ok_or(Error::OracleNotConfigured)?;

            let submitters: Vec<Address> = env
                .storage()
                .instance()
                .get(&DataKey::Submitters)
                .unwrap_or(Vec::new(&env));

            let now = env.ledger().timestamp();
            let mut fresh_prices: Vec<u64> = Vec::new(&env);
            let mut saw_any_submission = false;

            for feeder in submitters.iter() {
                let submission: Option<PriceData> =
                    env.storage().instance().get(&DataKey::Submission(feeder));
                if let Some(data) = submission {
                    saw_any_submission = true;
                    let age = now.saturating_sub(data.updated_at);
                    if age <= config.max_staleness && data.price != 0 {
                        fresh_prices.push_back(data.price);
                    }
                }
            }

            if fresh_prices.is_empty() {
                if saw_any_submission {
                    return Err(Error::OracleStalePrice);
                }
                return Err(Error::NoPriceAvailable);
            }

            Ok(median(fresh_prices))
        })
    }

    /// Read-only: seconds elapsed since the most recent `submit_price` call
    /// by any feeder, without triggering `get_twap_price`'s error path.
    ///
    /// Errors:
    /// - `OracleNotConfigured` if `configure_oracle` has not been called.
    /// - `NoPriceAvailable` if no price has ever been submitted.
    pub fn price_age(env: Env) -> Result<u64, Error> {
        if !env.storage().instance().has(&DataKey::Config) {
            return Err(Error::OracleNotConfigured);
        }

        let data: PriceData = env
            .storage()
            .instance()
            .get(&DataKey::Price)
            .ok_or(Error::NoPriceAvailable)?;

        Ok(env.ledger().timestamp().saturating_sub(data.updated_at))
    }

    /// Read-only: whether the most recent submission is older than
    /// `configure_oracle`'s `max_staleness`, without triggering
    /// `get_twap_price`'s error path — see issue #195.
    ///
    /// Errors:
    /// - `OracleNotConfigured` if `configure_oracle` has not been called.
    /// - `NoPriceAvailable` if no price has ever been submitted.
    pub fn is_price_stale(env: Env) -> Result<bool, Error> {
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
        Ok(age > config.max_staleness)
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
        bump_instance(&env);
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
        bump_instance(&env);
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

/// Records `account` in the `Submitters` set the first time it submits a
/// price, so `get_twap_price` knows which `DataKey::Submission` entries to
/// aggregate. No-op if already recorded.
fn add_submitter(env: &Env, account: &Address) {
    let mut submitters: Vec<Address> = env
        .storage()
        .instance()
        .get(&DataKey::Submitters)
        .unwrap_or(Vec::new(env));

    for existing in submitters.iter() {
        if existing == *account {
            return;
        }
    }

    submitters.push_back(account.clone());
    env.storage()
        .instance()
        .set(&DataKey::Submitters, &submitters);
}

/// Median of `prices` (average of the two middle values for an even-length
/// input). Sorts a copy in place with insertion sort — the submitter count
/// is expected to stay small, so O(n^2) is not a concern.
///
/// Panics if `prices` is empty; callers must guarantee at least one value.
fn median(prices: Vec<u64>) -> u64 {
    let len = prices.len();
    let mut sorted = prices.clone();

    let mut i: u32 = 1;
    while i < len {
        let key = sorted.get(i).unwrap();
        let mut j = i;
        while j > 0 {
            let prev = sorted.get(j - 1).unwrap();
            if prev > key {
                sorted.set(j, prev);
                j -= 1;
            } else {
                break;
            }
        }
        sorted.set(j, key);
        i += 1;
    }

    let mid = len / 2;
    if len & 1 == 0 {
        let a = sorted.get(mid - 1).unwrap() as u128;
        let b = sorted.get(mid).unwrap() as u128;
        ((a + b) / 2) as u64
    } else {
        sorted.get(mid).unwrap()
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

    pub fn admin_transferred(env: &Env, old_admin: &Address, new_admin: &Address) {
        env.events().publish(
            (symbol_short!("adm_xfer"), old_admin.clone()),
            new_admin.clone(),
        );
    }

    pub fn price_submitted(env: &Env, caller: &Address, price: u64, timestamp: u64) {
        env.events().publish(
            (symbol_short!("priced"), caller.clone()),
            (price, timestamp),
        );
    }

    pub fn oracle_configured(env: &Env, caller: &Address, config: super::OracleConfig) {
        env.events()
            .publish((symbol_short!("ocfg"), caller.clone()), config);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use soroban_sdk::testutils::storage::Instance as _;
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
        let (_env, client, admin) = setup();
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
    fn submit_price_rejects_non_admin_non_feeder() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        // Create a caller with no roles (neither Admin nor PriceFeeder).
        let impostor = Address::generate(&env);

        // Caller with no authorization should be rejected explicitly.
        let result = client.try_submit_price(&impostor, &100);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));

        // Verify that granting PriceFeeder role permits submission.
        client.grant_role(&admin, &Role::PriceFeeder, &impostor);
        let result = client.try_submit_price(&impostor, &100);
        assert!(result.is_ok());
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

    // ── Admin rotation tests (#192) ───────────────────────────────────────

    #[test]
    fn transfer_admin_moves_role_and_revokes_old() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);

        assert!(client.has_role(&Role::Admin, &new_admin));
        assert!(!client.has_role(&Role::Admin, &admin));
    }

    #[test]
    fn transfer_admin_requires_admin_role() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let stranger = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let result = client.try_transfer_admin(&stranger, &new_admin);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    }

    #[test]
    fn new_admin_can_act_immediately_after_transfer() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let new_admin = Address::generate(&env);
        client.transfer_admin(&admin, &new_admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr,
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        // Old admin is no longer authorized...
        let result = client.try_configure_oracle(&admin, &config);
        assert_eq!(result, Err(Ok(Error::NotAuthorized)));
        // ...new admin is.
        client.configure_oracle(&new_admin, &config);
    }

    // ── Multi-feeder aggregation tests (#194) ─────────────────────────────

    #[test]
    fn get_twap_price_aggregates_median_across_feeders() {
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

        let feeder_b = Address::generate(&env);
        let feeder_c = Address::generate(&env);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder_b);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder_c);

        client.submit_price(&admin, &10);
        client.submit_price(&feeder_b, &20);
        client.submit_price(&feeder_c, &30);

        // Median of [10, 20, 30] = 20.
        let price = client.get_twap_price();
        assert_eq!(price, 20);
    }

    #[test]
    fn get_twap_price_averages_middle_two_on_even_count() {
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

        let feeder_b = Address::generate(&env);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder_b);

        client.submit_price(&admin, &10);
        client.submit_price(&feeder_b, &20);

        // Even count: average of the two middle (only) values = 15.
        let price = client.get_twap_price();
        assert_eq!(price, 15);
    }

    #[test]
    fn get_twap_price_ignores_stale_submitters_in_aggregate() {
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

        let feeder_b = Address::generate(&env);
        client.grant_role(&admin, &Role::PriceFeeder, &feeder_b);

        client.submit_price(&admin, &10);
        let submitted_at = env.ledger().timestamp();

        // Advance time so admin's submission goes stale.
        env.ledger().set(LedgerInfo {
            timestamp: submitted_at + 61,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });

        // feeder_b submits fresh after the jump.
        client.submit_price(&feeder_b, &20);

        // Only feeder_b's fresh submission counts toward the aggregate.
        let price = client.get_twap_price();
        assert_eq!(price, 20);
    }

    #[test]
    fn get_twap_price_errors_stale_when_all_submitters_stale() {
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

        client.submit_price(&admin, &10);
        let submitted_at = env.ledger().timestamp();

        env.ledger().set(LedgerInfo {
            timestamp: submitted_at + 61,
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

    // ── Staleness introspection tests (#195) ──────────────────────────────

    #[test]
    fn price_age_reports_seconds_since_last_submission() {
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
        let submitted_at = env.ledger().timestamp();

        assert_eq!(client.price_age(), 0);

        env.ledger().set(LedgerInfo {
            timestamp: submitted_at + 42,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });

        assert_eq!(client.price_age(), 42);
    }

    #[test]
    fn price_age_requires_config() {
        let (_env, client, _admin) = setup();
        let result = client.try_price_age();
        assert_eq!(result, Err(Ok(Error::OracleNotConfigured)));
    }

    #[test]
    fn price_age_requires_price_submission() {
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

        let result = client.try_price_age();
        assert_eq!(result, Err(Ok(Error::NoPriceAvailable)));
    }

    #[test]
    fn is_price_stale_reflects_max_staleness_without_erroring() {
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
        let submitted_at = env.ledger().timestamp();

        assert!(!client.is_price_stale());

        env.ledger().set(LedgerInfo {
            timestamp: submitted_at + 61,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });

        // Reports true instead of erroring like get_twap_price would.
        assert!(client.is_price_stale());
    }

    // ── TTL extension tests (#189) ──────────────────────────────────────

    #[test]
    fn initialize_extends_instance_ttl() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
        assert!(ttl >= 100_000, "instance TTL after initialize: {ttl}");
    }

    #[test]
    fn configure_oracle_extends_instance_ttl() {
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

        let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
        assert!(ttl >= 100_000, "instance TTL after configure_oracle: {ttl}");
    }

    #[test]
    fn submit_price_extends_instance_ttl() {
        let (env, client, admin) = setup();
        client.initialize(&admin);
        client.submit_price(&admin, &50_000_000);

        let ttl = env.as_contract(&client.address, || env.storage().instance().get_ttl());
        assert!(ttl >= 100_000, "instance TTL after submit_price: {ttl}");
    }

    // ── Issue #206: configure_oracle clears stale price on decimals change ────

    #[test]
    fn configure_oracle_clears_price_when_decimals_change() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);
        client.submit_price(&admin, &50_000_000);

        // Price exists and is fresh
        let price = client.get_twap_price();
        assert_eq!(price, 50_000_000);

        // Reconfigure with different decimals
        let new_config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 6,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &new_config);

        // After decimals change, old price data should be cleared
        let result = client.try_get_twap_price();
        assert_eq!(result, Err(Ok(Error::NoPriceAvailable)));
    }

    #[test]
    fn configure_oracle_clears_price_when_asset_peg_changes() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);
        client.submit_price(&admin, &50_000_000);

        let price = client.get_twap_price();
        assert_eq!(price, 50_000_000);

        // Reconfigure with different asset_peg
        let new_config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 8,
            asset_peg: 2,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &new_config);

        // After asset_peg change, old price data should be cleared
        let result = client.try_get_twap_price();
        assert_eq!(result, Err(Ok(Error::NoPriceAvailable)));
    }

    #[test]
    fn configure_oracle_preserves_price_when_only_staleness_changes() {
        let (env, client, admin) = setup();
        client.initialize(&admin);

        let oracle_addr = Address::generate(&env);
        let config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 8,
            asset_peg: 1,
            max_staleness: 300,
        };
        client.configure_oracle(&admin, &config);
        client.submit_price(&admin, &50_000_000);

        let price = client.get_twap_price();
        assert_eq!(price, 50_000_000);

        // Reconfigure with only max_staleness changed
        let new_config = OracleConfig {
            oracle_address: oracle_addr.clone(),
            decimals: 8,
            asset_peg: 1,
            max_staleness: 600,
        };
        client.configure_oracle(&admin, &new_config);

        // Price should still be available — only staleness window changed
        let price_after = client.get_twap_price();
        assert_eq!(price_after, 50_000_000);
    }
}
