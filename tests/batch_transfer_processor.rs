//! Regression tests for
//! `conduit_integration_tests::batch_transfer_processor::BatchTransferProcessor`.
//!
//! Covers:
//! - Basic sum + lock-release contract (audit-round-2-v2)
//! - Issue #83: Defensive null/boundary guards against NPE edge cases
//! - Issue #84: State-version race-condition protection

#![cfg(test)]

use drip_batch_processor::{BatchTransferProcessor, BatchTransferProcessorClient, Error};
use soroban_sdk::{symbol_short, Env, Vec};

const LOCK_KEY: soroban_sdk::Symbol = symbol_short!("B_Lock");

fn deploy_processor(env: &Env) -> BatchTransferProcessorClient<'_> {
    let id = env.register_contract(None, BatchTransferProcessor);
    BatchTransferProcessorClient::new(env, &id)
}

/// Reads the processor's lock state from instance storage. Returns
/// `false` when the entry was never written, matching the contract's
/// own default of "unlocked".
fn lock_state(env: &Env, client: &BatchTransferProcessorClient<'_>) -> bool {
    env.as_contract(&client.address, || {
        env.storage()
            .instance()
            .get::<_, u32>(&LOCK_KEY)
            .unwrap_or(0)
            > 0
    })
}

#[test]
fn process_batch_sums_amounts_and_releases_lock() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts = Vec::from_array(&env, [10u64, 20, 30]);
    assert_eq!(client.try_process_batch(&amounts), Ok(Ok(60)));
    assert!(
        !lock_state(&env, &client),
        "lock must be released after a successful call",
    );
}

#[test]
fn process_batch_empty_input_returns_zero() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts: Vec<u64> = Vec::new(&env);
    assert_eq!(client.try_process_batch(&amounts), Ok(Ok(0)));
    assert!(
        !lock_state(&env, &client),
        "lock must be released after an empty-input call",
    );
}

#[test]
fn process_batch_accepts_exactly_100_entries() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts = Vec::from_array(&env, [1u64; 100]);
    assert_eq!(client.try_process_batch(&amounts), Ok(Ok(100)));
    assert!(
        !lock_state(&env, &client),
        "lock must be released after a max-sized successful call",
    );
}

#[test]
fn process_batch_rejects_101_entries_and_releases_lock() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts = Vec::from_array(&env, [1u64; 101]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::BatchTooLarge)),
    );
    assert!(
        !lock_state(&env, &client),
        "BatchTooLarge must release the lock so the next caller is not \
         fooled by a stale flag",
    );
}

#[test]
fn process_batch_detects_overflow_and_releases_lock() {
    let env = Env::default();
    let client = deploy_processor(&env);
    // `u64::MAX + 1` is the smallest pair that overflows `checked_add`.
    let amounts = Vec::from_array(&env, [u64::MAX, 1u64]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::CalculationOverflow)),
    );
    assert!(
        !lock_state(&env, &client),
        "CalculationOverflow must release the lock",
    );
}

#[test]
fn process_batch_rejects_when_lock_is_held() {
    let env = Env::default();
    let client = deploy_processor(&env);

    // Simulate a previous call that exited before clearing the lock
    // (e.g. a host panic). The contract must reject the next call
    // gracefully and not corrupt the externally-imposed lock state.
    env.as_contract(&client.address, || {
        env.storage().instance().set(&LOCK_KEY, &1_u32);
    });

    let amounts = Vec::from_array(&env, [42u64]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::ProcessorLocked)),
    );
    // The ProcessorLocked path is an early return BEFORE the contract
    // touches the lock. Lock in that invariant here: a future refactor
    // that adds a stray `set(lock_key, false)` (or any storage write)
    // adjacent to the early return would silently change a no-op-on-error
    // contract into a state-mutating one — this assertion catches it.
    assert!(
        lock_state(&env, &client),
        "ProcessorLocked must short-circuit without touching the lock",
    );
}

#[test]
fn error_type_carries_required_traits_and_named_discriminants() {
    fn assert_traits<T: Copy + Clone + core::fmt::Debug + Eq + PartialEq + PartialOrd + Ord>() {}
    assert_traits::<Error>();
    // Lock in the discriminant values so client integrators (and
    // downstream error handling in tests) cannot silently drift.
    assert_eq!(Error::ProcessorLocked as u32, 2001);
    assert_eq!(Error::CalculationOverflow as u32, 2002);
    assert_eq!(Error::BatchTooLarge as u32, 2003);
    assert_eq!(Error::StateVersionMismatch as u32, 2004);
    assert_eq!(Error::StaleCallbackCleaned as u32, 2005);
}

// ─────────────────────────────────────────────────────────────────────────────
//  Issue #83 regression: null / boundary edge cases
// ────────────────────────────────────────────────────────────────────────��────

#[test]
fn process_batch_rejects_zero_amount() {
    let env = Env::default();
    let client = deploy_processor(&env);
    // Zero is a degenerate amount that could trigger NPE-like downstream
    // divide-by-zero or infinite-loop edge cases.
    let amounts = Vec::from_array(&env, [0u64]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::CalculationOverflow)),
    );
    assert!(
        !lock_state(&env, &client),
        "lock must be released after zero-amount rejection",
    );
}

#[test]
fn process_batch_rejects_zero_amount_in_mixed_batch() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts = Vec::from_array(&env, [10u64, 0, 30]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::CalculationOverflow)),
    );
    assert!(
        !lock_state(&env, &client),
        "lock must be released after detecting zero in batch",
    );
}

#[test]
fn process_batch_rejects_single_zero_entry() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let amounts = Vec::from_array(&env, [0u64]);
    assert_eq!(
        client.try_process_batch(&amounts),
        Err(Ok(Error::CalculationOverflow)),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Issue #84 regression: state-version race-condition guards
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn process_batch_succeeds_from_arbitrary_starting_version() {
    let env = Env::default();
    let client = deploy_processor(&env);

    // Pre-set the version to an arbitrary value (e.g. 99) to verify the
    // contract correctly increments from any starting point.
    let state_ver_key = soroban_sdk::symbol_short!("B_Ver");
    env.as_contract(&client.address, || {
        env.storage().instance().set(&state_ver_key, &99_u64);
    });

    let amounts = Vec::from_array(&env, [10u64, 20]);
    assert_eq!(client.try_process_batch(&amounts), Ok(Ok(30)));
    assert!(
        !lock_state(&env, &client),
        "lock must be released after a successful call",
    );

    // Version should have been bumped to 100.
    let v: u64 = env.as_contract(&client.address, || {
        env.storage().instance().get(&state_ver_key).unwrap_or(0)
    });
    assert_eq!(v, 100);
}

#[test]
fn process_batch_increments_version_on_success() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let state_ver_key = soroban_sdk::symbol_short!("B_Ver");

    // Before any call, version is 0.
    let v0: u64 = env.as_contract(&client.address, || {
        env.storage().instance().get(&state_ver_key).unwrap_or(0)
    });
    assert_eq!(v0, 0);

    // After a successful call, version must have been bumped.
    let amounts = Vec::from_array(&env, [5u64, 5]);
    let r = client.try_process_batch(&amounts);
    assert!(r.is_ok());

    let v1: u64 = env.as_contract(&client.address, || {
        env.storage().instance().get(&state_ver_key).unwrap_or(0)
    });
    assert_eq!(v1, 1);
}

#[test]
fn process_batch_succeeds_after_manual_version_set() {
    let env = Env::default();
    let client = deploy_processor(&env);
    let state_ver_key = soroban_sdk::symbol_short!("B_Ver");

    // First call succeeds, bumping version to 1.
    let amounts = Vec::from_array(&env, [100u64]);
    assert!(client.try_process_batch(&amounts).is_ok());

    // Manually set version to 42 to simulate external state change.
    // The contract should still succeed — it reads the current version,
    // bumps it, and verifies the bump was applied correctly.
    env.as_contract(&client.address, || {
        env.storage().instance().set(&state_ver_key, &42_u64);
    });

    let amounts = Vec::from_array(&env, [200u64]);
    assert_eq!(client.try_process_batch(&amounts), Ok(Ok(200)));

    // Version should now be 43.
    let v: u64 = env.as_contract(&client.address, || {
        env.storage().instance().get(&state_ver_key).unwrap_or(0)
    });
    assert_eq!(v, 43);
}

#[test]
fn process_batch_stale_callback_seq_does_not_block_new_calls() {
    let env = Env::default();
    let client = deploy_processor(&env);

    // Simulate an orphaned callback sequence from a previous interrupted call.
    let cb_seq_key = soroban_sdk::symbol_short!("B_CbSeq");
    env.as_contract(&client.address, || {
        env.storage().instance().set(&cb_seq_key, &999_u64);
    });

    // A new call must not be blocked by stale callback state.
    let amounts = Vec::from_array(&env, [7u64, 8, 9]);
    let result = client.try_process_batch(&amounts);
    assert!(result.is_ok());

    // The callback sequence should have advanced past the stale value.
    let seq: u64 = env.as_contract(&client.address, || {
        env.storage().instance().get(&cb_seq_key).unwrap_or(0)
    });
    assert_eq!(seq, 1000);
}
