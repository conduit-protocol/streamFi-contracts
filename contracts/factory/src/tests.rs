#![cfg(test)]

// The crate is `#![no_std]`, but this module only compiles under `cargo test`,
// where `std` is available as a linked dependency of the test harness anyway.
extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env, String,
};

use crate::{DripFactory, DripFactoryClient, Error};

/// Register a factory and initialize it with a dummy stream WASM hash and a
/// freshly generated governor. Auth is mocked, so the governor-gated
/// `pause`/`unpause` calls authorize automatically.
///
/// These tests exercise the pause/unpause state machine and its emitted
/// events in isolation — they never call `create_stream` (which would need a
/// real stream WASM to deploy and a live governor cross-contract call), so a
/// zero WASM hash is sufficient here.
struct Setup {
    env: Env,
    client: DripFactoryClient<'static>,
}

impl Setup {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let governor = Address::generate(&env);
        let wasm_hash = BytesN::from_array(&env, &[0u8; 32]);

        let contract_id = env.register_contract(None, DripFactory);
        let client = DripFactoryClient::new(&env, &contract_id);
        client.initialize(&wasm_hash, &governor);

        Setup { env, client }
    }

    /// Number of contract events emitted so far.
    fn event_count(&self) -> usize {
        self.env.events().all().len() as usize
    }
}

#[test]
fn pause_then_unpause_flips_state() {
    let s = Setup::new();
    assert!(!s.client.is_paused());

    s.client.pause();
    assert!(s.client.is_paused());

    s.client.unpause();
    assert!(!s.client.is_paused());
}

#[test]
fn factory_status_returns_combined_pause_and_fee_status() {
    let s = Setup::new();
    let status = s.client.factory_status();
    assert!(!status.is_paused);
    assert_eq!(status.protocol_fee_bps, 30);

    s.client.pause();
    let status_paused = s.client.factory_status();
    assert!(status_paused.is_paused);
    assert_eq!(status_paused.protocol_fee_bps, 30);
}


#[test]
fn pause_when_already_paused_errors_and_leaves_state_unchanged() {
    let s = Setup::new();
    s.client.pause();
    assert!(s.client.is_paused());

    let events_before = s.event_count();
    // A redundant pause must not silently succeed — it reverts, so a retrying
    // off-chain caller can distinguish "I changed state" from "already there".
    let result = s.client.try_pause();
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));

    // State is still paused, and no additional event was emitted for the no-op.
    assert!(s.client.is_paused());
    assert_eq!(s.event_count(), events_before);
}

#[test]
fn unpause_when_not_paused_errors_and_leaves_state_unchanged() {
    let s = Setup::new();
    assert!(!s.client.is_paused());

    let events_before = s.event_count();
    let result = s.client.try_unpause();
    assert_eq!(result, Err(Ok(Error::NotPaused)));

    assert!(!s.client.is_paused());
    assert_eq!(s.event_count(), events_before);
}

#[test]
fn each_successful_transition_emits_exactly_one_event() {
    let s = Setup::new();
    let base = s.event_count();

    s.client.pause();
    assert_eq!(s.event_count(), base + 1);

    s.client.unpause();
    assert_eq!(s.event_count(), base + 2);
}

#[test]
fn rapid_repeated_calls_never_diverge_from_the_invoked_sequence() {
    // Simulates the issue's "100+ rapid requests" as a long sequence of
    // repeated calls in one test. Every redundant call reverts; only genuine
    // transitions mutate state or emit events. At every point the observable
    // state and the emitted-event count agree with the calls that actually
    // succeeded — state can never silently diverge from what was invoked.
    let s = Setup::new();
    let base = s.event_count();
    let mut expected_paused = false;
    let mut successful_transitions = 0usize;

    for i in 0..120u32 {
        if i % 2 == 0 {
            // Attempt to pause.
            if expected_paused {
                assert_eq!(s.client.try_pause(), Err(Ok(Error::AlreadyPaused)));
            } else {
                s.client.pause();
                expected_paused = true;
                successful_transitions += 1;
            }
        } else {
            // Attempt to unpause.
            if expected_paused {
                s.client.unpause();
                expected_paused = false;
                successful_transitions += 1;
            } else {
                assert_eq!(s.client.try_unpause(), Err(Ok(Error::NotPaused)));
            }
        }

        // Invariant checked on every iteration.
        assert_eq!(s.client.is_paused(), expected_paused);
        assert_eq!(s.event_count(), base + successful_transitions);
    }
}

// ── Issue #86: upgrade_stream_wasm input validation ─────────────────────────

#[test]
fn upgrade_stream_wasm_rejects_zero_hash() {
    let s = Setup::new();
    let zero_hash = BytesN::from_array(&s.env, &[0u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&zero_hash);
    assert_eq!(result, Err(Ok(Error::InvalidWasmHash)));
}

#[test]
fn upgrade_stream_wasm_accepts_non_zero_hash() {
    let s = Setup::new();
    let valid_hash = BytesN::from_array(&s.env, &[1u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert!(result.is_ok());
}

#[test]
fn upgrade_stream_wasm_rejects_when_paused() {
    let s = Setup::new();
    s.client.pause();
    let valid_hash = BytesN::from_array(&s.env, &[2u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn upgrade_stream_wasm_accepts_after_unpause() {
    let s = Setup::new();
    s.client.pause();
    s.client.unpause();
    let valid_hash = BytesN::from_array(&s.env, &[3u8; 32]);
    let result = s.client.try_upgrade_stream_wasm(&valid_hash);
    assert!(result.is_ok());
}

#[test]
fn create_stream_rejects_zero_stellar_recipient() {
    let env = base_env();
    let client = deploy_factory(&env);
    let sender = Address::generate(&env);

    let zero_recipient = Address::from_string(&String::from_str(
        &env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    let token = make_token(&env, &sender, 100_000);
    let now = env.ledger().timestamp();

    let result = client.try_create_stream(
        &sender,
        &zero_recipient,
        &token,
        &100_000,
        &100,
        &(now + 100),
        &(now + 3_700),
        &false,
    );

    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}