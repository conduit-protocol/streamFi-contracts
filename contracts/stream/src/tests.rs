#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token, Address, Env,
};

use crate::{DripStream, DripStreamClient};

/// Deploy a mock token and a DripStream, returning both clients and
/// the sender/recipient addresses.
struct Setup {
    env:       Env,
    client:    DripStreamClient<'static>,
    token:     token::Client<'static>,
    sender:    Address,
    recipient: Address,
}

impl Setup {
    fn new(rate_per_second: i128, duration_secs: u64, clawback: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let sender    = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Deploy a mock Stellar asset contract
        let token_admin = Address::generate(&env);
        let token_addr  = env.register_stellar_asset_contract(token_admin.clone());
        let tok         = token::Client::new(&env, &token_addr);
        let tok_admin   = token::StellarAssetClient::new(&env, &token_addr);

        let deposit = rate_per_second * duration_secs as i128;

        // Mint the deposit to the sender
        tok_admin.mint(&sender, &deposit);

        // Set ledger timestamp to a baseline
        let now: u64 = 1_000_000;
        env.ledger().set(LedgerInfo {
            timestamp:          now,
            protocol_version:   21,
            sequence_number:    1,
            network_id:         Default::default(),
            base_reserve:       10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl:      6_312_000,
        });

        // Deploy stream
        let stream_id = env.register_contract(None, DripStream);
        let client    = DripStreamClient::new(&env, &stream_id);

        // Transfer deposit into stream
        tok.transfer(&sender, &stream_id, &deposit);

        client.initialize(
            &sender,
            &recipient,
            &token_addr,
            &rate_per_second,
            &now,               // start_time = now
            &(now + duration_secs), // end_time
            &clawback,
        );

        // Leak the env so we can return 'static references — acceptable in tests.
        let env: &'static Env = Box::leak(Box::new(env));
        let client            = DripStreamClient::new(env, &stream_id);
        let token             = token::Client::new(env, &token_addr);

        Self { env: unsafe { std::ptr::read(env) }, client, token, sender, recipient }
    }

    fn advance_secs(&self, secs: u64) {
        let ts = self.env.ledger().timestamp() + secs;
        self.env.ledger().set(LedgerInfo {
            timestamp:          ts,
            ..self.env.ledger().get()
        });
    }
}

// ── Withdraw ─────────────────────────────────────────────────────────────────

#[test]
fn withdraw_zero_at_start() {
    let s = Setup::new(100, 3600, false);
    // At exactly start_time, elapsed = 0
    assert_eq!(s.client.withdrawable(), 0);
}

#[test]
fn withdraw_correct_after_elapsed() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    // 100 seconds × 100 stroops/s = 10_000 stroops
    assert_eq!(s.client.withdrawable(), 10_000);
    let withdrawn = s.client.withdraw(&10_000);
    assert_eq!(withdrawn, 10_000);
    assert_eq!(s.token.balance(&s.recipient), 10_000);
}

#[test]
fn withdraw_capped_at_available() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(50);
    // Available = 5_000; requesting 99_999 should give back only 5_000
    let withdrawn = s.client.withdraw(&99_999);
    assert_eq!(withdrawn, 5_000);
}

#[test]
#[should_panic(expected = "NothingToWithdraw")]
fn withdraw_before_any_elapsed_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.withdraw(&1);
}

#[test]
fn withdrawable_stops_at_end_time() {
    let s = Setup::new(100, 100, false); // 100s stream
    s.advance_secs(200); // advance past end_time
    // Should be capped at 100s worth = 10_000
    assert_eq!(s.client.withdrawable(), 10_000);
}

// ── Pause / Resume ────────────────────────────────────────────────────────────

#[test]
fn pause_freezes_withdrawable() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    let before_pause = s.client.withdrawable();
    s.client.pause();
    s.advance_secs(500); // time passes but stream is paused
    assert_eq!(s.client.withdrawable(), before_pause); // unchanged
}

#[test]
fn resume_continues_streaming() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100); // 100s elapsed → 10_000 owed
    s.client.pause();
    s.advance_secs(200); // 200s paused (should not count)
    s.client.resume();
    s.advance_secs(50);  // 50s more elapsed → +5_000
    // Total should be 150s of streaming = 15_000
    assert_eq!(s.client.withdrawable(), 15_000);
}

#[test]
#[should_panic(expected = "AlreadyPaused")]
fn double_pause_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.pause();
    s.client.pause(); // should panic
}

#[test]
#[should_panic(expected = "NotPaused")]
fn resume_unpaused_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.resume(); // not paused
}

// ── Cancel ────────────────────────────────────────────────────────────────────

#[test]
fn cancel_before_start_refunds_full_deposit() {
    let s = Setup::new(100, 3600, false);
    let deposit = 100 * 3600;
    let sender_before = s.token.balance(&s.sender);
    s.client.cancel();
    let sender_after = s.token.balance(&s.sender);
    assert_eq!(sender_after - sender_before, deposit);
    assert_eq!(s.token.balance(&s.recipient), 0);
}

#[test]
fn cancel_halfway_splits_correctly() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800); // halfway
    let sender_before    = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);
    s.client.cancel();
    // Recipient gets 1800 × 100 = 180_000 (earned but not withdrawn)
    // Sender gets 180_000 refund
    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 180_000);
    assert_eq!(s.token.balance(&s.sender)    - sender_before,    180_000);
}

#[test]
#[should_panic(expected = "StreamCancelled")]
fn cancel_then_cancel_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel();
    s.client.cancel();
}

#[test]
#[should_panic(expected = "StreamCancelled")]
fn withdraw_after_cancel_panics() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.cancel();
    s.client.withdraw(&1); // stream is fully settled; withdraw blocked
}

// ── Clawback ─────────────────────────────────────────────────────────────────

#[test]
fn clawback_reclaims_unstreamed() {
    let s = Setup::new(100, 3600, true); // clawback enabled
    s.advance_secs(600); // 600s streamed → 60_000 owed to recipient
    let sender_before = s.token.balance(&s.sender);
    let reclaimed = s.client.clawback();
    // reclaimed = total_balance − owed = (100×3600) − 60_000 = 300_000
    assert_eq!(reclaimed, 300_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 300_000);
}

#[test]
#[should_panic(expected = "ClawbackDisabled")]
fn clawback_disabled_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.clawback();
}

// ── Top-up ────────────────────────────────────────────────────────────────────

#[test]
fn top_up_increases_contract_balance() {
    let s = Setup::new(100, 3600, false);
    // Mint extra tokens to sender
    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &50_000);

    let stream_before = s.token.balance(&s.client.address);
    s.client.top_up(&50_000);
    assert_eq!(s.token.balance(&s.client.address), stream_before + 50_000);
}
