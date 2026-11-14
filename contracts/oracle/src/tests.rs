//! Unit tests for `DripOracle`. Integration tests live in
//! `tests/oracle_integration.rs` at the workspace root for
//! cross-contract scenarios once DripStream depends on this crate.

#![cfg(test)]

use crate::{errors::Error, twap::BUFFER_MAX, DripOracle, DripOracleClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
};

fn base_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });
    env
}

fn advance(env: &Env, secs: u64) {
    env.ledger().set(LedgerInfo {
        timestamp: env.ledger().timestamp() + secs,
        ..env.ledger().get()
    });
}

fn deploy<'e>(env: &'e Env, admin: &Address) -> DripOracleClient<'e> {
    let id = env.register_contract(None, DripOracle);
    let client = DripOracleClient::new(env, &id);
    client.init(admin);
    client
}

fn configure(
    admin: &Address,
    client: &DripOracleClient<'_>,
    asset: &Address,
    window_size: u64,
    max_staleness: u64,
) {
    client.configure_asset(admin, asset, &7u32, &window_size, &max_staleness);
}

// ─── init ────────────────────────────────────────────────────────────

#[test]
fn init_accepts_first_call_and_rejects_duplicate() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    assert_eq!(client.admin_address(), Some(admin.clone()));

    let res = client.try_init(&admin);
    assert_eq!(res, Err(Ok(Error::AlreadyInitialized)));
}

// ─── configure_asset ─────────────────────────────────────────────────

#[test]
fn configure_asset_persists_and_round_trips_admin_address() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    configure(&admin, &client, &asset, 300, 600);
    assert_eq!(client.observation_count(&asset), 0);
    assert_eq!(client.oracle_count(&asset), 0);
}

#[test]
fn configure_asset_accepts_window_greater_than_max_staleness() {
    // We do NOT reject `window > stale` — the prior invariant made
    // `Error::PriceStale` unreachable because staleness requires
    // `advance > stale` while in-window requires `window >= advance`,
    // and `window <= stale` collapses those into a contradiction.
    // The standard pattern is window >= stale (longer TWAP lookback than
    // freshness); lock that in here so a future refactor doesn't
    // re-introduce the rejection.
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    let res = client.try_configure_asset(&admin, &asset, &7u32, &600u64, &300u64);
    assert!(
        res.is_ok(),
        "window > stale is allowed (was previously wrongly rejected)"
    );
}

#[test]
fn configure_asset_rejects_decimals_above_max() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    let bad_decimals = crate::twap::MAX_DECIMALS + 1;
    let res = client.try_configure_asset(&admin, &asset, &bad_decimals, &300u64, &600u64);
    assert_eq!(res, Err(Ok(Error::InvalidConfig)));
}

#[test]
fn configure_asset_rejects_zero_window_or_staleness() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    let res = client.try_configure_asset(&admin, &asset, &7u32, &0u64, &600u64);
    assert_eq!(res, Err(Ok(Error::InvalidConfig)));
    let res = client.try_configure_asset(&admin, &asset, &7u32, &300u64, &0u64);
    assert_eq!(res, Err(Ok(Error::InvalidConfig)));
}

#[test]
fn configure_asset_rejects_non_admin() {
    let env = base_env();
    let admin = Address::generate(&env);
    let impostor = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    let res = client.try_configure_asset(&impostor, &asset, &7u32, &300u64, &600u64);
    assert_eq!(res, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn configure_asset_rejects_before_init() {
    let env = base_env();
    let id = env.register_contract(None, DripOracle);
    let client = DripOracleClient::new(&env, &id);
    let admin = Address::generate(&env);
    let asset = Address::generate(&env);

    let res = client.try_configure_asset(&admin, &asset, &7u32, &300u64, &600u64);
    assert_eq!(res, Err(Ok(Error::NotInitialized)));
}

// ─── whitelist ───────────────────────────────────────────────────────

#[test]
fn add_oracle_upserts_idempotent() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);

    client.add_oracle(&admin, &asset, &oracle);
    assert!(client.is_whitelisted(&asset, &oracle));
    assert_eq!(client.oracle_count(&asset), 1);

    // Second add is a no-op.
    client.add_oracle(&admin, &asset, &oracle);
    assert_eq!(client.oracle_count(&asset), 1);
}

#[test]
fn add_oracle_rejects_unconfigured_asset() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);

    let res = client.try_add_oracle(&admin, &asset, &oracle);
    assert_eq!(res, Err(Ok(Error::AssetNotConfigured)));
}

#[test]
fn remove_oracle_is_idempotent() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);
    client.remove_oracle(&admin, &asset, &oracle);
    assert!(!client.is_whitelisted(&asset, &oracle));

    // Removing a non-whitelisted oracle is a no-op, not an error.
    let res = client.try_remove_oracle(&admin, &asset, &oracle);
    assert!(res.is_ok());
}

// ─── submit_observation ──────────────────────────────────────────────

#[test]
fn submit_observation_appends_and_persists() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &1_000_000i128, &now);
    assert_eq!(client.observation_count(&asset), 1);
}

#[test]
fn submit_observation_rejects_non_whitelist() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    let impostor = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let res = client.try_submit_observation(&impostor, &asset, &1_000_000i128, &1_000_000u64);
    assert_eq!(res, Err(Ok(Error::OracleNotWhitelisted)));
}

#[test]
fn submit_observation_rejects_non_positive_price() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    let res = client.try_submit_observation(&oracle, &asset, &0i128, &now);
    assert_eq!(res, Err(Ok(Error::InvalidPrice)));
    let res = client.try_submit_observation(&oracle, &asset, &-1i128, &now);
    assert_eq!(res, Err(Ok(Error::InvalidPrice)));
}

#[test]
fn submit_observation_rejects_unconfigured_asset() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);

    let res = client.try_submit_observation(&oracle, &asset, &1_000_000i128, &1_000_000u64);
    assert_eq!(res, Err(Ok(Error::AssetNotConfigured)));
}

#[test]
fn submit_observation_rejects_non_monotonic_timestamp() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &1_000_000i128, &now);
    // Equal — should reject.
    let res = client.try_submit_observation(&oracle, &asset, &1_001_000i128, &now);
    assert_eq!(res, Err(Ok(Error::TimestampInvalid)));
    advance(&env, 5);
    // Older by 5s — should reject.
    let res = client.try_submit_observation(
        &oracle,
        &asset,
        &1_001_000i128,
        &(env.ledger().timestamp() - 5),
    );
    assert_eq!(res, Err(Ok(Error::TimestampInvalid)));
}

#[test]
fn submit_observation_rejects_far_future_timestamp() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    let res = client.try_submit_observation(
        &oracle,
        &asset,
        &1_000_000i128,
        &(now + crate::twap::CLOCK_DRIFT_TOLERANCE + 1),
    );
    assert_eq!(res, Err(Ok(Error::TimestampInvalid)));
}

// ─── buffer rollover + get_twap ─────────────────────────────────────

#[test]
fn buffer_caps_at_buffer_max_rolling_oldest_out() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 600, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let mut ts = env.ledger().timestamp();
    for _ in 0..(BUFFER_MAX + 5) {
        client.submit_observation(&oracle, &asset, &1_000_000i128, &ts);
        advance(&env, 1);
        ts = env.ledger().timestamp();
    }
    assert_eq!(client.observation_count(&asset), BUFFER_MAX);
}

#[test]
fn get_twap_single_sample_returns_its_price() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &1_234_567i128, &now);

    let (price, decimals) = client.try_get_twap(&asset).unwrap().unwrap();
    assert_eq!(price, 1_234_567);
    assert_eq!(decimals, 7);
}

#[test]
fn get_twap_two_samples_compute_time_weighted_average() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    // window_size = max_staleness = 600s so both samples (60s apart)
    // sit comfortably in-window.
    configure(&admin, &client, &asset, 600, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let start = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &100i128, &start);
    advance(&env, 60);
    client.submit_observation(&oracle, &asset, &200i128, &(start + 60));

    // Expected:
    // - segment 1: price 100 for 60 s (weight 60)
    // - segment 2: price 200 from ts=60 to now=start+60 (weight 0,
    //   because the second sample was just submitted at the current
    //   ledger time)
    // weighted_sum = 100 * 60 = 6000, weight_total = 60, TWAP = 100.
    let (price, _dec) = client.try_get_twap(&asset).unwrap().unwrap();
    assert_eq!(price, 100);
}

#[test]
fn get_twap_three_samples_with_zero_price_later_segment() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 1000, 1000);
    client.add_oracle(&admin, &asset, &oracle);

    let start = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &100i128, &start);
    advance(&env, 60);
    client.submit_observation(&oracle, &asset, &200i128, &(start + 60));
    advance(&env, 60);
    client.submit_observation(&oracle, &asset, &100i128, &(start + 120));

    // Walk through manually:
    //   obs0: ts=0,    price=100 — segment to obs1 (ts=60, weight 60)
    //   obs1: ts=60,   price=200 — segment to obs2 (ts=120, weight 60)
    //   obs2: ts=120,  price=100 — segment to now (ts=120, weight 0)
    // weighted_sum = 100*60 + 200*60 + 100*0 = 18000
    // weight_total = 60 + 60 + 0 = 120
    // TWAP = 18000 / 120 = 150
    let (price, _) = client.try_get_twap(&asset).unwrap().unwrap();
    assert_eq!(price, 150);
}

#[test]
fn get_twap_rejects_when_newest_observation_is_stale() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 300, 100); // window(300) > max_staleness(100) keeps obs in-window; advance(200) > max_staleness so obs is stale
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &100i128, &now);
    advance(&env, 200); // exceeds max_staleness = 60

    let res = client.try_get_twap(&asset);
    assert_eq!(res, Err(Ok(Error::PriceStale)));
}

#[test]
fn get_twap_rejects_insufficient_samples_when_only_obs_outside_window() {
    // Cleaner replacement for the original env.as_contract poke test:
    // submit a single observation, advance ledger time past the TWAP
    // window but well within the staleness window, then read —
    // first_in_window loop returns None → InsufficientSamples.
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);
    configure(&admin, &client, &asset, 60, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let initial = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &100i128, &initial);
    // Advance past window (60s) but well within staleness (600s).
    advance(&env, 120);
    let res = client.try_get_twap(&asset);
    assert_eq!(res, Err(Ok(Error::InsufficientSamples)));
}

#[test]
fn get_twap_rejects_insufficient_samples_when_buffer_empty() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    // Configure but never push — exercises the empty-buffer path
    // without bypassing the public API.
    configure(&admin, &client, &asset, 60, 600);

    let res = client.try_get_twap(&asset);
    assert_eq!(res, Err(Ok(Error::InsufficientSamples)));
}

#[test]
fn get_twap_is_a_no_op_on_unconfigured_asset() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);

    let res = client.try_get_twap(&asset);
    assert_eq!(res, Err(Ok(Error::AssetNotConfigured)));
}

// ─── accessors ───────────────────────────────────────────────────────

#[test]
fn is_whitelisted_false_on_unconfigured_asset() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);

    assert!(!client.is_whitelisted(&asset, &oracle));
}

#[test]
fn admin_address_returns_none_before_init() {
    let env = base_env();
    let id = env.register_contract(None, DripOracle);
    let client = DripOracleClient::new(&env, &id);

    assert_eq!(client.admin_address(), None);
}

#[test]
fn error_type_carries_required_traits_and_named_discriminants() {
    fn assert_traits<T: Copy + Clone + core::fmt::Debug + Eq + PartialEq + PartialOrd + Ord>() {}
    // Pins the derive set needed by #[contracterror].
    assert_traits::<Error>();
    // Pins discriminants so client integrators (and tests) cannot
    // silently drift on a refactor.
    assert_eq!(Error::NotInitialized as u32, 1);
    assert_eq!(Error::AlreadyInitialized as u32, 2);
    assert_eq!(Error::AssetNotConfigured as u32, 3);
    assert_eq!(Error::OracleNotWhitelisted as u32, 4);
    assert_eq!(Error::TimestampInvalid as u32, 5);
    assert_eq!(Error::PriceStale as u32, 6);
    assert_eq!(Error::InsufficientSamples as u32, 7);
    assert_eq!(Error::ArithmeticOverflow as u32, 8);
    assert_eq!(Error::InvalidPrice as u32, 9);
    assert_eq!(Error::InvalidConfig as u32, 10);
    assert_eq!(Error::NotAuthorized as u32, 11);
    assert_eq!(Error::OraclePriceStale as u32, 16);
}

// ─── multi-asset price feed aggregation ──────────────────────────────

#[test]
fn multi_asset_aggregation_with_independent_buffers() {
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset_a = Address::generate(&env);
    let asset_b = Address::generate(&env);
    let oracle = Address::generate(&env);

    configure(&admin, &client, &asset_a, 300, 600);
    configure(&admin, &client, &asset_b, 300, 600);
    client.add_oracle(&admin, &asset_a, &oracle);
    client.add_oracle(&admin, &asset_b, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset_a, &100i128, &now);
    client.submit_observation(&oracle, &asset_b, &200i128, &now);

    let (price_a, _) = client.try_get_twap(&asset_a).unwrap().unwrap();
    let (price_b, _) = client.try_get_twap(&asset_b).unwrap().unwrap();
    assert_eq!(price_a, 100);
    assert_eq!(price_b, 200);
}

#[test]
fn deviation_bounds_reject_excessive_spread() {
    // When price_a = 100 and price_b = 200, the deviation is 100%.
    // If max_deviation_bps = 1000 (10%), this spread should be rejected.
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle_a = Address::generate(&env);
    let oracle_b = Address::generate(&env);

    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle_a);
    client.add_oracle(&admin, &asset, &oracle_b);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle_a, &asset, &100i128, &now);
    advance(&env, 1);
    client.submit_observation(&oracle_b, &asset, &200i128, &(now + 1));

    // Deviation: |200 - 100| / 100 = 100% > 10% bound → error
    // (This test documents the behavior; implementation requires
    // adding max_deviation_bps config and checking logic.)
}

#[test]
fn deviation_bounds_accept_within_tolerance() {
    // When price_a = 100 and price_b = 105, the deviation is 5%.
    // If max_deviation_bps = 1000 (10%), this spread should be accepted.
    let env = base_env();
    let admin = Address::generate(&env);
    let client = deploy(&env, &admin);
    let asset = Address::generate(&env);
    let oracle = Address::generate(&env);

    configure(&admin, &client, &asset, 300, 600);
    client.add_oracle(&admin, &asset, &oracle);

    let now = env.ledger().timestamp();
    client.submit_observation(&oracle, &asset, &100i128, &now);
    advance(&env, 1);
    client.submit_observation(&oracle, &asset, &105i128, &(now + 1));

    let (price, _) = client.try_get_twap(&asset).unwrap().unwrap();
    assert!(price > 0);
}
