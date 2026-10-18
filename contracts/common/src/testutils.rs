//! Test helpers, mock environments, and assertion utilities.
//! Gated behind the `testutils` feature flag (Issue #709).

use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    Address, Env,
};

/// Generates a test address for unit and integration testing.
pub fn create_test_address(env: &Env) -> Address {
    Address::generate(env)
}

/// Helper to configure a test environment with all authorizations mocked.
pub fn setup_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

/// Helper to advance ledger sequence and timestamp in tests.
pub fn advance_ledger(env: &Env, ledgers: u32, seconds: u64) {
    env.ledger().with_mut(|li| {
        li.sequence_number += ledgers;
        li.timestamp += seconds;
    });
}
