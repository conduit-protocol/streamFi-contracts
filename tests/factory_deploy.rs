//! Integration tests: DripFactory validation and registry queries.
//!
//! Deployment tests (create_stream success path) require the stream WASM to be
//! built first (`cargo build --target wasm32-unknown-unknown --release`).
//! The tests in this file cover validation guards and registry read-paths that
//! do not require an actual stream deployment.

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, BytesN, Env,
};
use drip_factory::{DripFactory, DripFactoryClient};

fn base_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set(LedgerInfo {
        timestamp:                1_000_000,
        protocol_version:         21,
        sequence_number:          1,
        network_id:               Default::default(),
        base_reserve:             10,
        min_temp_entry_ttl:       16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl:            6_312_000,
    });
    env
}

fn deploy_factory(env: &Env) -> DripFactoryClient<'_> {
    let id     = env.register_contract(None, DripFactory);
    let client = DripFactoryClient::new(env, &id);
    let governor   = Address::generate(env);
    let dummy_hash = BytesN::from_array(env, &[0u8; 32]);
    client.initialize(&dummy_hash, &governor);
    client
}

// ── Fresh factory state ───────────────────────────────────────────────────────

#[test]
fn stream_count_starts_at_zero() {
    let env    = base_env();
    let client = deploy_factory(&env);
    assert_eq!(client.stream_count(), 0);
}

#[test]
fn stream_address_returns_none_for_nonexistent_id() {
    let env    = base_env();
    let client = deploy_factory(&env);
    assert!(client.stream_address(&0).is_none());
    assert!(client.stream_address(&999).is_none());
}

#[test]
fn streams_by_sender_returns_empty_for_unknown_address() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let result = client.streams_by_sender(&sender, &0, &10);
    assert_eq!(result.len(), 0);
}

#[test]
fn streams_by_recipient_returns_empty_for_unknown_address() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let recip  = Address::generate(&env);
    let result = client.streams_by_recipient(&recip, &0, &10);
    assert_eq!(result.len(), 0);
}

#[test]
fn protocol_fee_bps_returns_default_30() {
    let env    = base_env();
    let client = deploy_factory(&env);
    assert_eq!(client.protocol_fee_bps(), 30);
}

// ── Validation errors (all fail before deployment) ────────────────────────────

fn make_token(env: &Env, sender: &Address, amount: i128) -> Address {
    let admin = Address::generate(env);
    let addr  = env.register_stellar_asset_contract(admin.clone());
    token::StellarAssetClient::new(env, &addr).mint(sender, &amount);
    addr
}

#[test]
#[should_panic(expected = "InvalidDeposit")]
fn create_stream_rejects_zero_deposit() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 0);
    let now    = env.ledger().timestamp();
    // deposit = 0 → InvalidDeposit before any deployment
    client.create_stream(&sender, &recip, &token, &0, &100, &(now + 100), &(now + 3_700), &false);
}

#[test]
#[should_panic(expected = "InvalidRate")]
fn create_stream_rejects_zero_rate() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 10_000);
    let now    = env.ledger().timestamp();
    client.create_stream(&sender, &recip, &token, &10_000, &0, &(now + 100), &(now + 3_700), &false);
}

#[test]
#[should_panic(expected = "InsufficientDeposit")]
fn create_stream_rejects_deposit_less_than_one_second() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 10_000);
    let now    = env.ledger().timestamp();
    // deposit (50) < rate_per_sec (100) → InsufficientDeposit
    client.create_stream(&sender, &recip, &token, &50, &100, &(now + 100), &(now + 3_700), &false);
}

#[test]
#[should_panic(expected = "BackdatedStream")]
fn create_stream_rejects_start_time_in_past() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 100_000);
    let now    = env.ledger().timestamp();
    // start_time = now - 1 → BackdatedStream
    client.create_stream(&sender, &recip, &token, &100_000, &100, &(now - 1), &(now + 3_600), &false);
}

#[test]
#[should_panic(expected = "InvalidTimeRange")]
fn create_stream_rejects_end_before_start() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 100_000);
    let now    = env.ledger().timestamp();
    // end_time <= start_time → InvalidTimeRange
    client.create_stream(&sender, &recip, &token, &100_000, &100, &(now + 100), &(now + 50), &false);
}

#[test]
#[should_panic(expected = "InvalidTimeRange")]
fn create_stream_rejects_end_equal_to_start() {
    let env    = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);
    let recip  = Address::generate(&env);
    let token  = make_token(&env, &sender, 100_000);
    let now    = env.ledger().timestamp();
    let start  = now + 100;
    client.create_stream(&sender, &recip, &token, &100_000, &100, &start, &start, &false);
}
