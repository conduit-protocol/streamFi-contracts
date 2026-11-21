#![cfg(test)]

// The crate is `#![no_std]`, but this module only compiles under `cargo test`,
// where `std` is available as a linked dependency of the test harness anyway.
extern crate std;
use std::boxed::Box;

use soroban_sdk::{
    symbol_short,
    testutils::{storage::Instance as _, Address as _, Events as _, Ledger, LedgerInfo},
    token, Address, Env, IntoVal, String, Symbol, TryFromVal, TryIntoVal,
};

use crate::{
    storage::{DataKey, StreamInfo, FLAG_CANCELLED, FLAG_CLAWBACK_ENABLED, FLAG_PAUSED},
    DripStream, DripStreamClient, Error,
};

/// Deploy a mock token and a DripStream, returning both clients and
/// the sender/recipient addresses.
struct Setup {
    env: Env,
    client: DripStreamClient<'static>,
    token: token::Client<'static>,
    sender: Address,
    recipient: Address,
}

impl Setup {
    fn new(rate_per_second: i128, duration_secs: u64, clawback: bool) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Deploy a mock Stellar asset contract
        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let tok = token::Client::new(&env, &token_addr);
        let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

        let deposit = rate_per_second * duration_secs as i128;

        // Mint the deposit to the sender
        tok_admin.mint(&sender, &deposit);

        // Set ledger timestamp to a baseline
        let now: u64 = 1_000_000;
        env.ledger().set(LedgerInfo {
            timestamp: now,
            protocol_version: 21,
            sequence_number: 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 4096,
            max_entry_ttl: 6_312_000,
        });

        // Deploy stream
        let stream_id = env.register_contract(None, DripStream);
        let client = DripStreamClient::new(&env, &stream_id);

        // Transfer deposit into stream
        tok.transfer(&sender, &stream_id, &deposit);

        client.initialize(
            &sender,
            &recipient,
            &token_addr,
            &rate_per_second,
            &now,                   // start_time = now
            &(now + duration_secs), // end_time
            &clawback,
            &2_592_000_u64,
            &false,
            &None,
        );

        // Leak the env so we can return 'static references — acceptable in tests.
        let env: &'static Env = Box::leak(Box::new(env));
        let client = DripStreamClient::new(env, &stream_id);
        let token = token::Client::new(env, &token_addr);

        Self {
            env: unsafe { std::ptr::read(env) },
            client,
            token,
            sender,
            recipient,
        }
    }

    fn advance_secs(&self, secs: u64) {
        let ts = self.env.ledger().timestamp() + secs;
        self.env.ledger().set(LedgerInfo {
            timestamp: ts,
            ..self.env.ledger().get()
        });
    }
}

// ── Withdraw ─────────────────────────────────────────────────────────────────

#[test]
fn storage_version_set_on_initialize() {
    let s = Setup::new(100, 3600, false);
    assert_eq!(s.client.storage_version(), 1);
}

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
fn total_streamed_matches_streamed_total() {
    let s = Setup::new(1_000, 3_600, false);
    s.advance_secs(100);
    assert_eq!(s.client.total_streamed(), 100_000);
    assert_eq!(s.client.total_streamed(), s.client.streamed_total());
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
fn withdraw_before_any_elapsed_panics() {
    let s = Setup::new(100, 3600, false);
    let result = s.client.try_withdraw(&1);
    assert_eq!(result, Err(Ok(Error::NothingToWithdraw)));
}

#[test]
fn withdrawable_stops_at_end_time() {
    let s = Setup::new(100, 100, false); // 100s stream
    s.advance_secs(200); // advance past end_time
                         // Should be capped at 100s worth = 10_000
    assert_eq!(s.client.withdrawable(), 10_000);
}

#[test]
fn transfer_recipient_rejects_zero_address() {
    let s = Setup::new(100, 3600, false);
    let zero = Address::from_string(&soroban_sdk::String::from_str(
        &s.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    let result = s.client.try_transfer_recipient(&zero);
    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}

#[test]
fn transfer_recipient_rejects_same_recipient() {
    let s = Setup::new(100, 3600, false);

    let result = s.client.try_transfer_recipient(&s.recipient);
    assert_eq!(result, Err(Ok(Error::InvalidRecipient)));
}

#[test]
fn transfer_recipient_rejects_ended_stream() {
    let s = Setup::new(100, 100, false);
    s.advance_secs(200);

    let result = s.client.try_transfer_recipient(&Address::generate(&s.env));
    assert_eq!(result, Err(Ok(Error::StreamEnded)));
}

// ── Pause / Resume ────────────────────────────────────────────────────────────

#[test]
fn pause_freezes_withdrawable() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    let before_pause = s.client.withdrawable();
    s.client.pause(&s.sender);
    s.advance_secs(500); // time passes but stream is paused
    assert_eq!(s.client.withdrawable(), before_pause); // unchanged
}

/// Regression for #350: a stream that is left paused must keep freezing
/// accrual at `paused_at` even after ledger time passes `end_time`. Before
/// this fix the `end_time` clamp was evaluated before the pause clamp, so once
/// `now > end_time` a never-resumed pause reported the full contracted amount,
/// letting the recipient withdraw everything (pause fully defeated) and
/// understating the sender's refund on cancel.
#[test]
fn paused_stream_does_not_accrue_past_end_time() {
    // 200s stream: start = 1_000_000, end = 1_000_200.
    let s = Setup::new(100, 200, false);
    s.advance_secs(50); // 50s elapsed → 5_000 owed
    s.client.pause(&s.sender); // paused_at = 1_000_050

    // Advance well past end_time (1_000_200 → now 1_000_550) without resuming.
    s.advance_secs(500);
    assert!(s.env.ledger().timestamp() > s.client.info().end_time);
    assert!(s.client.info().is_paused());

    // Still frozen at the 5_000 owed at pause time — NOT the full 20_000.
    assert_eq!(s.client.withdrawable(), 5_000);
    assert_eq!(s.client.streamed_total(), 5_000);
}

#[test]
fn resume_continues_streaming() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100); // 100s elapsed → 10_000 owed
    s.client.pause(&s.sender);
    s.advance_secs(200); // 200s paused (should not count)
    s.client.resume(&s.sender);
    s.advance_secs(50); // 50s more elapsed → +5_000
                        // Total should be 150s of streaming = 15_000
    assert_eq!(s.client.withdrawable(), 15_000);
}

#[test]
fn double_pause_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.pause(&s.sender);
    let result = s.client.try_pause(&s.sender);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
}

#[test]
fn resume_unpaused_panics() {
    let s = Setup::new(100, 3600, false);
    let result = s.client.try_resume(&s.sender); // not paused
    assert_eq!(result, Err(Ok(Error::NotPaused)));
}

#[test]
fn pause_before_start_rejected() {
    let s = Setup::new(100, 3600, false);
    let mut ledger = s.env.ledger().get();
    ledger.timestamp -= 1;
    s.env.ledger().set(ledger);

    let result = s.client.try_pause(&s.sender);
    assert_eq!(result, Err(Ok(Error::StreamNotStarted)));
}

// ── Cancel ────────────────────────────────────────────────────────────────────

#[test]
fn pause_after_end_rejected() {
    let s = Setup::new(100, 3600, false);
    // Advance past the end time.
    let mut ledger = s.env.ledger().get();
    ledger.timestamp += 3601;
    s.env.ledger().set(ledger);

    let result = s.client.try_pause(&s.sender);
    assert_eq!(result, Err(Ok(Error::StreamEnded)));
}

#[test]
fn cancel_before_start_refunds_full_deposit() {
    let s = Setup::new(100, 3600, false);
    let deposit = 100 * 3600;
    let sender_before = s.token.balance(&s.sender);
    s.client.cancel(&s.sender);
    let sender_after = s.token.balance(&s.sender);
    assert_eq!(sender_after - sender_before, deposit);
    assert_eq!(s.token.balance(&s.recipient), 0);
}

#[test]
fn cancel_halfway_splits_correctly() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800); // halfway
    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);
    s.client.cancel(&s.sender);
    // Recipient gets 1800 × 100 = 180_000 (earned but not withdrawn)
    // Sender gets 180_000 refund
    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 180_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 180_000);
}

#[test]
fn cancel_advances_withdrawn_by_the_payout() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800); // halfway → 180_000 earned, none pulled via withdraw()
    s.client.cancel(&s.sender);

    // `cancel` paid the recipient 180_000 directly. Post-cancel `withdrawn`
    // must reflect what the recipient received, not just withdraw() pulls.
    assert_eq!(s.client.info().withdrawn, 180_000);
}

#[test]
fn cancel_advances_withdrawn_on_top_of_prior_withdrawals() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800); // 180_000 earned
    let pulled = s.client.withdraw(&50_000);
    assert_eq!(pulled, 50_000);
    s.client.cancel(&s.sender);

    // 50_000 via withdraw() + 130_000 paid out by cancel = 180_000 total.
    assert_eq!(s.client.info().withdrawn, 180_000);
}

#[test]
fn cancel_then_cancel_panics() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel(&s.sender);
    let result = s.client.try_cancel(&s.sender);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn withdraw_after_cancel_panics() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.cancel(&s.sender);
    // stream is fully settled; withdraw blocked
    let result = s.client.try_withdraw(&1);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

// ── Clawback ─────────────────────────────────────────────────────────────────

#[test]
fn clawback_reclaims_unstreamed() {
    let s = Setup::new(100, 3600, true); // clawback enabled
    s.advance_secs(600); // 600s streamed → 60_000 owed to recipient
    let sender_before = s.token.balance(&s.sender);
    let reclaimed = s.client.clawback(&s.sender);
    // reclaimed = total_balance − owed = (100×3600) − 60_000 = 300_000
    assert_eq!(reclaimed, 300_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 300_000);
}

#[test]
fn clawback_disabled_panics() {
    let s = Setup::new(100, 3600, false);
    let result = s.client.try_clawback(&s.sender);
    assert_eq!(result, Err(Ok(Error::ClawbackDisabled)));
}

/// Regression test for #458.
///
/// Open-ended streams (`end_time == 0`) accrue indefinitely while their
/// funded balance is finite. `clawback` must refund exactly the unstreamed
/// remainder (balance − accrued-but-unwithdrawn) and must never touch the
/// portion that has already accrued to the recipient.
#[test]
fn clawback_open_ended_refunds_unstreamed_remainder() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    // rate = 100 stroops/s; deposit = 10_000 stroops → funds 100 s of streaming.
    let rate: i128 = 100;
    let deposit: i128 = 10_000;
    let now: u64 = 1_000_000;

    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    tok_admin.mint(&sender, &deposit);
    tok.transfer(&sender, &stream_id, &deposit);

    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &0, // open-ended — no end_time
        &true,
        &2_592_000_u64,
        &false,
        &None,
    );

    // Advance 30 s → 3_000 stroops have accrued to the recipient.
    env.ledger().set(LedgerInfo {
        timestamp: now + 30,
        ..env.ledger().get()
    });

    let accrued: i128 = rate * 30; // 3_000
    let expected_refund = deposit - accrued; // 7_000

    let sender_before = tok.balance(&sender);
    let reclaimed = client.clawback(&sender);

    // Clawback returns exactly the unstreamed remainder.
    assert_eq!(reclaimed, expected_refund);
    // Sender's wallet grew by the same amount.
    assert_eq!(tok.balance(&sender) - sender_before, expected_refund);
    // Accrued balance is still in the contract, available to the recipient.
    assert_eq!(tok.balance(&stream_id), accrued);
}

/// Regression test for #458 — second invariant.
///
/// After a clawback on an open-ended stream the recipient can still
/// withdraw every stroop that had already accrued before the clawback,
/// but cannot withdraw more.
#[test]
fn clawback_open_ended_does_not_touch_accrued_funds() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    let rate: i128 = 100;
    let deposit: i128 = 10_000;
    let now: u64 = 1_000_000;

    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    tok_admin.mint(&sender, &deposit);
    tok.transfer(&sender, &stream_id, &deposit);

    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &0, // open-ended
        &true,
        &2_592_000_u64,
        &false,
        &None,
    );

    // Advance 50 s → 5_000 stroops accrued.
    env.ledger().set(LedgerInfo {
        timestamp: now + 50,
        ..env.ledger().get()
    });

    let accrued: i128 = rate * 50; // 5_000

    // Sender claws back the unstreamed half.
    client.clawback(&sender);

    // Time is frozen; withdrawable must equal the full accrued amount.
    assert_eq!(client.withdrawable(), accrued);

    // Recipient can withdraw every accrued stroop.
    let withdrawn = client.withdraw(&accrued);
    assert_eq!(withdrawn, accrued);
    assert_eq!(tok.balance(&recipient), accrued);

    // Nothing left to withdraw.
    assert_eq!(client.withdrawable(), 0);
}

// ── Top-up ────────────────────────────────────────────────────────────────────

#[test]
fn top_up_increases_contract_balance() {
    let s = Setup::new(100, 3600, false);
    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &50_000);

    let stream_before = s.token.balance(&s.client.address);
    s.client.top_up(&s.sender, &50_000);
    assert_eq!(s.token.balance(&s.client.address), stream_before + 50_000);
    assert_eq!(s.client.cumulative_deposited(), 410_000);
}

#[test]
fn auto_forward_address_receives_withdrawals_and_can_be_disabled() {
    let s = Setup::new(100, 3600, false);
    let forward = Address::generate(&s.env);
    s.client.set_auto_forward_address(&forward);
    assert_eq!(s.client.auto_forward_address(), Some(forward.clone()));

    s.advance_secs(50);
    assert_eq!(s.client.withdraw(&5_000), 5_000);
    assert_eq!(s.token.balance(&forward), 5_000);
    assert_eq!(s.token.balance(&s.recipient), 0);

    s.client.set_auto_forward_address(&s.recipient);
    assert_eq!(s.client.auto_forward_address(), None);
}

#[test]
fn top_up_on_cancelled_stream_is_rejected() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel(&s.sender);

    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &10_000);

    let result = s.client.try_top_up(&s.sender, &10_000);
    assert!(result.is_err());
}

#[test]
fn top_up_rejects_zero_and_negative_amount() {
    let s = Setup::new(100, 3600, false);
    assert_eq!(
        s.client.try_top_up(&s.sender, &0),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        s.client.try_top_up(&s.sender, &-1),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn withdraw_rejects_zero_and_negative_amount() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    assert_eq!(s.client.try_withdraw(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(s.client.try_withdraw(&-1), Err(Ok(Error::InvalidAmount)));
}

// ── Empty-stream guard ───────────────────────────────────────────────────────

/// Deploy a bare DripStream (bypassing the factory — allowed per ADR-001,
/// one contract per stream) and attempt to initialize it with a zero rate.
/// Such a stream would escrow tokens but never release any ("empty
/// stream") and must be rejected at initialization time with
/// `InvalidAmount` (error #15).
#[test]
#[should_panic(expected = "Error(Contract, #15)")]
fn initialize_rejects_zero_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let now: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &0, // rate_per_second = 0 → empty stream
        &now,
        &(now + 3_600),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #15)")]
fn initialize_rejects_negative_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let now: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &-1, // negative rate → empty stream
        &now,
        &(now + 3_600),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

// ── Initialization guard ──────────────────────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn re_initializing_an_active_stream_panics() {
    let s = Setup::new(100, 3600, false);
    // An attacker calling initialize() again to hijack sender/recipient
    // must be rejected — otherwise they could redirect the escrowed balance
    // to themselves via cancel()/clawback().
    let attacker = Address::generate(&s.env);
    s.client.initialize(
        &attacker,
        &attacker,
        &s.token.address,
        &1,
        &0,
        &0,
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

// ── Time-range boundary guard (issue #81) ────────────────────────────────────

/// A bounded stream whose `end_time` is *before* `start_time` is malformed.
/// `initialize()` must reject it at the boundary with `InvalidTimeRange`
/// (error #8) before any state is persisted — otherwise the escrowed balance
/// gets permanently locked (see `malformed_time_range_would_lock_funds`).
#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn initialize_rejects_end_time_before_start() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let now: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,           // start_time
        &(now - 3_600), // end_time BEFORE start_time → malformed
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

/// A zero-duration bounded stream (`end_time == start_time`) releases nothing
/// yet still escrows tokens — the same "empty stream" class the zero-rate
/// guard already rejects. It must fail with `InvalidTimeRange` (error #8).
#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn initialize_rejects_end_time_equal_start() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let now: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now, // start_time
        &now, // end_time == start_time → zero-duration, malformed
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn initialize_rejects_overflowing_total_obligation() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let start_time: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &i128::MAX,
        &start_time,
        &(start_time + 60),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

/// The guard must NOT reject legitimate open-ended streams (`end_time == 0`).
/// This is the regression fence around the boundary check: `0` is a sentinel
/// for "no end", not a time that precedes `start_time`.
#[test]
fn initialize_accepts_open_ended_stream() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let now: u64 = 1_000_000;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,
        &0, // open-ended → valid
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    let inf = client.info();
    assert_eq!(inf.end_time, 0);
    assert_eq!(inf.start_time, now);
}

/// Documents the exact failure mode the boundary check prevents.
///
/// We inject malformed state (`end_time < start_time`) directly through the
/// legacy per-field storage keys, bypassing `initialize()`'s guard, then show
/// that once ledger time passes `start_time` the release math underflows and
/// surfaces `ArithmeticOverflow`. In a real deployment that same error fires

// ── TTL management ─────────────────────────────────────────────────────────────

#[test]
fn initialize_extends_instance_ttl() {
    let s = Setup::new(100, 3600, false);
    // Without an explicit extend_ttl call, instance storage TTL is left at
    // whatever the host assigns on creation, which is well under the
    // production-safe window. initialize() must bump it immediately.
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn withdraw_extends_instance_ttl() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.withdraw(&1);
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

// ── Cancelled stream state ────────────────────────────────────────────────────

#[test]
fn withdrawable_returns_zero_after_cancel() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(500);
    assert!(s.client.withdrawable() > 0);

    s.client.cancel(&s.sender);
    assert_eq!(s.client.withdrawable(), 0);
}

#[test]
fn pause_then_cancel_refunds_correctly() {
    let s = Setup::new(100, 3600, false);
    let deposit = 100 * 3600; // 360_000

    s.advance_secs(600); // 60_000 streamed
    s.client.pause(&s.sender);
    s.advance_secs(1_000); // time passes; not counted

    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);
    s.client.cancel(&s.sender);

    // Recipient should get 60_000 (earned before pause)
    // Sender should get 360_000 − 60_000 = 300_000
    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 60_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 300_000);
    let _ = deposit; // suppress unused warning
}

// ── Stream info ───────────────────────────────────────────────────────────────

#[test]
fn info_returns_correct_initial_state() {
    let s = Setup::new(250, 7_200, true);
    let inf = s.client.info();

    assert_eq!(inf.rate_per_second, 250);
    assert!(!inf.is_paused());
    assert!(!inf.is_cancelled());
    assert!(inf.is_clawback_enabled());
    assert_eq!(inf.withdrawn, 0);
}

#[test]
fn info_reflects_pause_state() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.pause(&s.sender);

    let inf = s.client.info();
    assert!(inf.is_paused());
    assert!(inf.paused_at > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn uninitialized_stream_info_returns_not_initialized() {
    let env = Env::default();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    client.info();
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn uninitialized_stream_withdrawable_returns_not_initialized() {
    let env = Env::default();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    client.withdrawable();
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn uninitialized_stream_streamed_total_returns_not_initialized() {
    let env = Env::default();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    client.streamed_total();
}

#[test]
fn uninitialized_stream_mutations_return_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    let caller = Address::generate(&env);

    assert_eq!(client.try_withdraw(&1), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_cancel(&caller), Err(Ok(Error::NotInitialized)));
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[test]
fn withdraw_exactly_full_balance_succeeds() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(3600); // end_time reached — full deposit earned
    let total = 100 * 3600; // 360_000

    let withdrawn = s.client.withdraw(&(total as i128));
    assert_eq!(withdrawn, total as i128);
    assert_eq!(s.token.balance(&s.recipient), total as i128);
}

#[test]
fn multiple_sequential_withdrawals_sum_correctly() {
    let s = Setup::new(1_000, 3_600, false);
    s.advance_secs(900); // 900_000 streamed

    let w1 = s.client.withdraw(&300_000);
    let w2 = s.client.withdraw(&300_000);
    let w3 = s.client.withdraw(&300_000);

    assert_eq!(w1 + w2 + w3, 900_000);
    assert_eq!(s.token.balance(&s.recipient), 900_000);
}

// ── Event delivery recovery ─────────────────────────────────────────────────

#[test]
fn delayed_consumer_retains_payloads_and_can_detect_sequence_gaps() {
    let s = Setup::new(100, 3_600, false);

    // Simulate rapid state changes while a consumer is disconnected. The
    // consumer reads the committed events only after all mutations complete.
    s.advance_secs(10);
    let paused_at = s.env.ledger().timestamp();
    s.client.pause(&s.sender);

    s.advance_secs(5);
    let resumed_at = s.env.ledger().timestamp();
    s.client.resume(&s.sender);

    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &500);
    s.client.top_up(&s.sender, &500);
    let balance_after_top_up = s.token.balance(&s.client.address);

    assert_eq!(s.client.event_sequence(), 4);

    let all_events = s.env.events().all();
    let stream_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .collect();

    assert_eq!(stream_events.len(), 4);

    // Event topics come back as `Vec<Val>` (which implements `PartialEq`) so
    // they can be compared directly. Event *data*, however, is a raw `Val`,
    // which deliberately has no `PartialEq` — comparing two `Val`s directly is
    // a compile error. Decode each data payload back into concrete Rust types
    // and compare those instead.
    //
    // `stream_events[0]` is the `created` event emitted by `initialize()` in
    // `Setup::new`, occupying sequence 1.
    assert_eq!(
        stream_events[1].1,
        (symbol_short!("paused"), s.sender.clone(), 2_u64).into_val(&s.env)
    );
    let paused_data: (u64, i128) = stream_events[1].2.try_into_val(&s.env).unwrap();
    assert_eq!(paused_data, (paused_at, 1_000_i128));

    assert_eq!(
        stream_events[2].1,
        (symbol_short!("resumed"), s.sender.clone(), 3_u64).into_val(&s.env)
    );
    let resumed_data: u64 = stream_events[2].2.try_into_val(&s.env).unwrap();
    assert_eq!(resumed_data, resumed_at);

    assert_eq!(
        stream_events[3].1,
        (symbol_short!("topped_up"), s.sender.clone(), 4_u64).into_val(&s.env)
    );
    let topped_up_data: (i128, i128) = stream_events[3].2.try_into_val(&s.env).unwrap();
    assert_eq!(topped_up_data, (500_i128, balance_after_top_up));
}

// ── Extend duration ─────────────────────────────────────────────────────────

#[test]
fn extend_duration_success() {
    let s = Setup::new(100, 3_600, false);
    let before_end = s.client.info().end_time;

    // Mint exact deposit needed to extend by 100s (100 × 100 = 10_000)
    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &10_000);

    let contract_before = s.token.balance(&s.client.address);
    s.client.extend_duration(&s.sender, &100);

    assert_eq!(s.client.info().end_time, before_end + 100);
    assert_eq!(s.token.balance(&s.client.address), contract_before + 10_000);
}

#[test]
fn extend_duration_rejected_for_open_ended() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let now: u64 = 1_000_000;
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,
        &0,
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    let result = client.try_extend_duration(&sender, &100);
    assert!(result.is_err(), "zero-duration stream must be rejected");
}

#[test]
fn extend_duration_rejects_on_arithmetic_overflow() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let now: u64 = 1_000_000;
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    // A minimum-duration stream at the largest rate whose 60s obligation
    // fits: multiplying it by u64::MAX seconds still overflows i128.
    let rate: i128 = i128::MAX / 60;
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &(now + 60),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    let result = client.try_extend_duration(&sender, &u64::MAX);
    assert_eq!(result, Err(Ok(Error::ArithmeticOverflow)));
}

// ── Cancellation CEI / settlement invariants (issue #78) ──────────────────────
//
// Issue #78 alleged a reentrancy drain in `cancel_batch_streams`. No such
// function exists in this workspace — the factory only exposes
// `create_batch_streams`, and cancellation lives entirely in this contract as
// `cancel()` / `force_cancel()`. Both already commit the cancelled flag before
// any `token::transfer`, which is the ordering the issue asked for.
//
// The specific attack described (a token contract re-entering the stream from
// a transfer callback) is not expressible on Soroban: the host refuses to
// re-enter a contract that is already on the call stack, and SEP-41 tokens
// have no receiver hooks to fire in the first place. So there is no
// `test_reentrancy_on_batch_cancel` to write — a test cannot construct the
// precondition.
//
// What IS worth locking in is the property that made the attack impossible:
// after cancellation, the contract holds no balance AND is durably marked
// cancelled, so there is no state in which a second settlement could pay out
// again. These tests pin that down so a future refactor cannot silently move a
// transfer ahead of the state write, or leave a re-drainable remainder behind.

/// `cancel()` must leave zero balance in the contract and a durably-set
/// cancelled flag. If a transfer were ever moved ahead of `state::save`, an
/// interleaved settlement would observe `is_cancelled() == false` while the
/// balance was still non-zero — this asserts that window is closed.
#[test]
fn cancel_commits_state_and_drains_balance() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800);

    assert!(!s.client.info().is_cancelled());
    assert!(s.token.balance(&s.client.address) > 0);

    s.client.cancel(&s.sender);

    assert!(s.client.info().is_cancelled());
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// Value conservation across `cancel()`: every stroop the contract held is
/// accounted for by exactly one payout, and nothing is left to drain twice.
#[test]
fn cancel_conserves_value_and_leaves_nothing_to_redrain() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(900);

    let escrowed = s.token.balance(&s.client.address);
    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);

    s.client.cancel(&s.sender);

    let paid_to_sender = s.token.balance(&s.sender) - sender_before;
    let paid_to_recipient = s.token.balance(&s.recipient) - recipient_before;

    assert_eq!(paid_to_sender + paid_to_recipient, escrowed);
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// Same invariant for the recipient-initiated escape hatch.
#[test]
fn force_cancel_commits_state_and_drains_balance() {
    // 60-day stream so the 30-day pause threshold is reached well before
    // end_time — otherwise `streamed_amount` clamps to end_time and the
    // pause branch never applies.
    let s = Setup::new(100, 5_184_000, false);
    s.advance_secs(1_000);
    s.client.pause(&s.sender);
    s.advance_secs(2_592_001); // 30 days + 1s

    let escrowed = s.token.balance(&s.client.address);
    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);

    s.client.force_cancel();

    assert!(s.client.info().is_cancelled());
    assert_eq!(s.token.balance(&s.client.address), 0);

    let paid_to_sender = s.token.balance(&s.sender) - sender_before;
    let paid_to_recipient = s.token.balance(&s.recipient) - recipient_before;
    // Only the 1_000s streamed before the pause is owed to the recipient.
    assert_eq!(paid_to_recipient, 100_000);
    assert_eq!(paid_to_sender + paid_to_recipient, escrowed);
    // `withdrawn` reflects the tokens the recipient received in force_cancel.
    assert_eq!(s.client.info().withdrawn, 100_000);
}

/// `force_cancel()` must succeed at exactly `pause_start + threshold`:
/// `_force_cancel` rejects with `PauseThresholdNotMet` only when
/// `now - paused_at < PAUSE_THRESHOLD_SECS`, so the boundary value itself is
/// accepted rather than one second short of it.
#[test]
fn force_cancel_succeeds_at_exact_pause_threshold() {
    // 60-day stream keeps end_time beyond the 30-day pause threshold so the
    // pause branch is the operative one.
    let s = Setup::new(100, 5_184_000, false);
    s.client.pause(&s.sender);
    s.advance_secs(2_592_000); // ledger time == pause_start + threshold

    s.client.force_cancel();

    assert!(s.client.info().is_cancelled());
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// `force_cancel()` must be rejected at `pause_start + threshold - 1`: one
/// second before the threshold the pause has not elapsed long enough, and the
/// stream must remain paused and unsettled.
#[test]
fn force_cancel_rejected_one_second_before_pause_threshold() {
    let s = Setup::new(100, 5_184_000, false);
    s.client.pause(&s.sender);
    s.advance_secs(2_591_999); // ledger time == pause_start + threshold - 1

    assert_eq!(
        s.client.try_force_cancel(),
        Err(Ok(Error::PauseThresholdNotMet))
    );
    assert!(!s.client.info().is_cancelled());
    assert!(s.client.info().is_paused());
}

/// Every value-moving entry point must reject an already-cancelled stream.
/// This is the guard that makes a second settlement impossible regardless of
/// how the caller reaches it.
#[test]
fn all_settlement_paths_rejected_after_cancel() {
    let s = Setup::new(100, 3600, true); // clawback enabled
    s.advance_secs(900);
    s.client.cancel(&s.sender);

    assert_eq!(
        s.client.try_cancel(&s.sender),
        Err(Ok(Error::StreamCancelled))
    );
    assert_eq!(s.client.try_force_cancel(), Err(Ok(Error::StreamCancelled)));
    assert_eq!(
        s.client.try_clawback(&s.sender),
        Err(Ok(Error::StreamCancelled))
    );
    assert_eq!(s.client.try_withdraw(&1), Err(Ok(Error::StreamCancelled)));
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// Mirror of the above, entered through `force_cancel()` instead.
#[test]
fn all_settlement_paths_rejected_after_force_cancel() {
    let s = Setup::new(100, 5_184_000, true); // clawback enabled
    s.advance_secs(1_000);
    s.client.pause(&s.sender);
    s.advance_secs(2_592_001);
    s.client.force_cancel();

    assert_eq!(
        s.client.try_cancel(&s.sender),
        Err(Ok(Error::StreamCancelled))
    );
    assert_eq!(s.client.try_force_cancel(), Err(Ok(Error::StreamCancelled)));
    assert_eq!(
        s.client.try_clawback(&s.sender),
        Err(Ok(Error::StreamCancelled))
    );
    assert_eq!(s.client.try_withdraw(&1), Err(Ok(Error::StreamCancelled)));
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// Partial withdrawals before cancellation must not let the recipient be paid
/// twice for the same streamed seconds.
#[test]
fn cancel_after_partial_withdrawal_does_not_double_pay() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(1800); // 180_000 streamed

    s.client.withdraw(&100_000);

    let escrowed = s.token.balance(&s.client.address);
    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);

    s.client.cancel(&s.sender);

    let paid_to_recipient = s.token.balance(&s.recipient) - recipient_before;
    let paid_to_sender = s.token.balance(&s.sender) - sender_before;

    // Recipient is owed only the 80_000 not already withdrawn.
    assert_eq!(paid_to_recipient, 80_000);
    assert_eq!(paid_to_sender, 180_000);
    assert_eq!(paid_to_sender + paid_to_recipient, escrowed);
    assert_eq!(s.token.balance(&s.client.address), 0);
}

/// The cancelled flag must survive as committed state, not just as a value in
/// the cancelling invocation's memory — a later, independent invocation has to
/// observe it.
#[test]
fn cancelled_flag_is_durable_across_invocations() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel(&s.sender);

    let persisted = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));
    assert!(persisted.is_cancelled());

    assert_eq!(s.client.withdrawable(), 0);
    assert_eq!(s.client.streamed_total(), 0);
}

// ── Issue #619: mutable prospective stream rate ──────────────────────────────

#[test]
fn change_rate_preserves_accrued_value_and_applies_new_rate_prospectively() {
    let s = Setup::new(100, 3_600, false);
    s.advance_secs(100);
    assert_eq!(s.client.streamed_total(), 10_000);

    s.client.change_rate(&s.sender, &200);
    assert_eq!(s.client.info().rate_per_second, 200);
    assert_eq!(
        s.client.streamed_total(),
        10_000,
        "changing the rate must not reprice value already accrued"
    );

    s.advance_secs(50);
    assert_eq!(s.client.streamed_total(), 20_000);
}

#[test]
fn change_rate_rejects_invalid_unauthorized_and_overflowing_rates() {
    let s = Setup::new(1, 60, false);

    assert_eq!(
        s.client.try_change_rate(&s.sender, &0),
        Err(Ok(Error::InvalidAmount))
    );

    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.client.try_change_rate(&stranger, &2),
        Err(Ok(Error::NotAuthorized))
    );

    assert_eq!(
        s.client.try_change_rate(&s.sender, &i128::MAX),
        Err(Ok(Error::ArithmeticOverflow))
    );
}

#[test]
fn change_rate_emits_rate_changed_event() {
    let s = Setup::new(100, 3_600, false);
    s.client.change_rate(&s.sender, &250);

    assert_eq!(s.client.event_sequence(), 2);
    let all_events = s.env.events().all();
    let stream_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .collect();
    let last = stream_events.last().unwrap();

    assert_eq!(
        last.1,
        (symbol_short!("rate_chg"), s.sender.clone(), 2_u64).into_val(&s.env)
    );
    let data: (i128, i128) = last.2.try_into_val(&s.env).unwrap();
    assert_eq!(data, (100, 250));
}

// ── Issue #205: top_up_and_extend convenience ────────────────────────────────

#[test]
fn top_up_and_extend_updates_balance_and_end_time() {
    let s = Setup::new(100, 3_600, false);
    let before_end = s.client.info().end_time;

    // Mint exact deposit needed: 100 rate × 200s = 20_000
    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &20_000);

    let contract_before = s.token.balance(&s.client.address);
    s.client.top_up_and_extend(&s.sender, &20_000, &200);

    assert_eq!(s.client.info().end_time, before_end + 200);
    let contract_after = s.token.balance(&s.client.address);
    assert_eq!(contract_after, contract_before + 20_000);

    // The atomic operation emits one dedicated event rather than a generic
    // top-up event plus a separate duration event that indexers must correlate,
    // followed by the explicit `StreamTopUpExtended` schedule event (#881).
    assert_eq!(s.client.event_sequence(), 3);
    let all_events = s.env.events().all();
    let stream_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .collect();
    assert_eq!(stream_events.len(), 3);
    let top_ext = &stream_events[1];
    assert_eq!(
        top_ext.1,
        (symbol_short!("top_ext"), s.sender.clone(), 2_u64).into_val(&s.env)
    );
    let data: (i128, i128, u64, u64) = top_ext.2.try_into_val(&s.env).unwrap();
    assert_eq!(
        data,
        (
            contract_before,
            contract_after,
            before_end,
            before_end + 200
        )
    );
}

#[test]
fn top_up_and_extend_rejects_zero_amount() {
    let s = Setup::new(100, 3_600, false);
    let result = s.client.try_top_up_and_extend(&s.sender, &0, &100);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn top_up_and_extend_rejects_zero_extra_time() {
    let s = Setup::new(100, 3_600, false);
    let result = s.client.try_top_up_and_extend(&s.sender, &10_000, &0);
    assert!(result.is_err(), "zero-duration stream must be rejected");
}

#[test]
fn top_up_and_extend_rejected_on_cancelled_stream() {
    let s = Setup::new(100, 3_600, false);
    s.client.cancel(&s.sender);

    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&s.sender, &10_000);

    let result = s.client.try_top_up_and_extend(&s.sender, &10_000, &100);
    assert!(result.is_err());
}

#[test]
fn top_up_and_extend_rejected_for_open_ended_stream() {
    let env = Env::default();
    env.mock_all_auths();

    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let now: u64 = 1_000_000;
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,
        &0,
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    let result = client.try_top_up_and_extend(&sender, &10_000, &100);
    assert!(result.is_err(), "zero-duration stream must be rejected");
}

// ── Operator access control ─────────────────────────────────────────────────

#[test]
fn operator_returns_none_initially() {
    let s = Setup::new(100, 3600, false);
    assert_eq!(s.client.operator(), None);
}

#[test]
fn set_operator_sets_address() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    assert_eq!(s.client.operator(), Some(operator));
}

#[test]
fn set_operator_rejects_non_sender() {
    let s = Setup::new(100, 3600, false);
    let non_sender = Address::generate(&s.env);
    let operator = Address::generate(&s.env);
    let result = s.client.try_set_operator(&non_sender, &operator);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn set_operator_rejects_on_cancelled_stream() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel(&s.sender);
    let operator = Address::generate(&s.env);
    let result = s.client.try_set_operator(&s.sender, &operator);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn set_operator_replaces_existing_operator_atomically() {
    // Issue #417: set_operator should replace an existing operator in one call,
    // allowing atomic key rotation without a no-operator gap.
    let s = Setup::new(100, 3600, false);
    let op1 = Address::generate(&s.env);
    let op2 = Address::generate(&s.env);

    // Set initial operator
    s.client.set_operator(&s.sender, &op1);
    assert_eq!(s.client.operator(), Some(op1.clone()));

    // Replace with a different operator - should succeed now
    s.client.set_operator(&s.sender, &op2);
    assert_eq!(s.client.operator(), Some(op2.clone()));

    // Verify we can replace again
    let op3 = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &op3);
    assert_eq!(s.client.operator(), Some(op3.clone()));
}

#[test]
fn set_operator_is_idempotent_for_same_address() {
    // Issue #417: Setting the same operator twice should be idempotent.
    // The early return keeps this operation idempotent.
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);

    s.client.set_operator(&s.sender, &operator);
    assert_eq!(s.client.operator(), Some(operator.clone()));

    // Setting the same operator again should succeed without error
    s.client.set_operator(&s.sender, &operator);
    assert_eq!(s.client.operator(), Some(operator));
}

#[test]
fn revoke_operator_removes_address() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    assert_eq!(s.client.operator(), Some(operator));
    s.client.revoke_operator(&s.sender);
    assert_eq!(s.client.operator(), None);
}

#[test]
fn revoke_operator_rejects_non_sender() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    let non_sender = Address::generate(&s.env);
    let result = s.client.try_revoke_operator(&non_sender);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoke_operator_rejects_on_cancelled_stream() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.cancel(&s.sender);
    let result = s.client.try_revoke_operator(&s.sender);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn operator_events_emit_correct_topic_shape_and_sequence() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);

    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    assert_eq!(s.client.event_sequence(), 3);

    let all_events = s.env.events().all();
    let stream_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .collect();

    assert_eq!(stream_events.len(), 3);

    // Event 1: set_op
    assert_eq!(
        stream_events[1].1,
        (
            symbol_short!("set_op"),
            s.sender.clone(),
            operator.clone(),
            2_u64
        )
            .into_val(&s.env)
    );
    let set_op_data: Address = stream_events[1].2.try_into_val(&s.env).unwrap();
    assert_eq!(set_op_data, operator);

    // Event 2: rm_op with ((rm_op, sender, 3), ())
    assert_eq!(
        stream_events[2].1,
        (symbol_short!("rm_op"), s.sender.clone(), 3_u64).into_val(&s.env)
    );
    let rm_op_data: () = stream_events[2].2.try_into_val(&s.env).unwrap();
    assert_eq!(rm_op_data, ());
}

// ── Operator exercises sender-gated functions ──────────────────────────────

#[test]
fn operator_can_cancel() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.advance_secs(1800);
    let sender_before = s.token.balance(&s.sender);
    let recipient_before = s.token.balance(&s.recipient);
    s.client.cancel(&operator);

    assert!(s.client.info().is_cancelled());
    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 180_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 180_000);
}

#[test]
fn operator_can_pause() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.advance_secs(100);
    let before_pause = s.client.withdrawable();
    s.client.pause(&operator);

    assert!(s.client.info().is_paused());
    s.advance_secs(500);
    assert_eq!(s.client.withdrawable(), before_pause);
}

#[test]
fn operator_can_resume() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.advance_secs(100);
    s.client.pause(&operator);
    s.advance_secs(200);
    s.client.resume(&operator);
    s.advance_secs(50);

    // 150s of streaming = 15_000
    assert_eq!(s.client.withdrawable(), 15_000);
}

#[test]
fn operator_passes_auth_gate_for_top_up() {
    // top_up transfers tokens FROM the sender via tk.transfer(&sender, …),
    // which requires the sender's auth in a nested cross-contract call.
    // mock_all_auths() only covers root-level auth, so the full happy-path
    // cannot be exercised without explicit non-root auth entries. Instead,
    // we verify the operator clears the require_sender_or_operator gate:
    // a cancelled stream fails with StreamCancelled (not NotAuthorized),
    // proving the operator was accepted as authorized.
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.cancel(&s.sender);

    let result = s.client.try_top_up(&operator, &1_000);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn operator_can_extend_duration() {
    let s = Setup::new(100, 3_600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    let before_end = s.client.info().end_time;
    // extend_duration transfers the required deposit (100s × rate 100 =
    // 10_000) from the *caller* — here the operator — so fund the operator
    // (see _extend_duration / #431).
    let token_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    token_admin.mint(&operator, &10_000);

    let contract_before = s.token.balance(&s.client.address);
    s.client.extend_duration(&operator, &100);

    assert_eq!(s.client.info().end_time, before_end + 100);
    assert_eq!(s.token.balance(&s.client.address), contract_before + 10_000);
}

#[test]
fn operator_passes_auth_gate_for_extend_duration() {
    // extend_duration transfers tokens FROM the sender via tk.transfer.
    // Same auth limitation as top_up — verify the operator clears the gate
    // by checking StreamCancelled (not NotAuthorized) on a cancelled stream.
    let s = Setup::new(100, 3_600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.cancel(&s.sender);

    let result = s.client.try_extend_duration(&operator, &100);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn operator_passes_auth_gate_for_top_up_and_extend() {
    // top_up_and_extend transfers tokens FROM the sender via tk.transfer.
    // Same auth limitation as top_up — verify the operator clears the gate
    // by checking StreamCancelled (not NotAuthorized) on a cancelled stream.
    let s = Setup::new(100, 3_600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.cancel(&s.sender);

    let result = s.client.try_top_up_and_extend(&operator, &1_000, &100);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
}

#[test]
fn operator_can_clawback() {
    let s = Setup::new(100, 3600, true);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.advance_secs(600); // 60_000 owed to recipient
    let sender_before = s.token.balance(&s.sender);
    let reclaimed = s.client.clawback(&operator);

    assert_eq!(reclaimed, 300_000);
    assert_eq!(s.token.balance(&s.sender) - sender_before, 300_000);
}

// ── Non-sender/non-operator rejected by sender-gated functions ─────────────

#[test]
fn non_sender_non_operator_rejected_by_cancel() {
    let s = Setup::new(100, 3600, false);
    let rando = Address::generate(&s.env);
    let result = s.client.try_cancel(&rando);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_sender_non_operator_rejected_by_pause() {
    let s = Setup::new(100, 3600, false);
    let rando = Address::generate(&s.env);
    let result = s.client.try_pause(&rando);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_sender_non_operator_rejected_by_resume() {
    let s = Setup::new(100, 3600, false);
    s.client.pause(&s.sender);
    let rando = Address::generate(&s.env);
    let result = s.client.try_resume(&rando);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_sender_can_top_up_gift_funding() {
    // Issue #690: top_up allows third-party gift funding with auth
    let s = Setup::new(100, 3600, false);
    let rando = Address::generate(&s.env);
    let tok_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    tok_admin.mint(&rando, &1_000);
    let result = s.client.try_top_up(&rando, &1_000);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn non_sender_non_operator_rejected_by_extend_duration() {
    let s = Setup::new(100, 3_600, false);
    let rando = Address::generate(&s.env);
    let result = s.client.try_extend_duration(&rando, &100);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_sender_non_operator_rejected_by_top_up_and_extend() {
    let s = Setup::new(100, 3_600, false);
    let rando = Address::generate(&s.env);
    let result = s.client.try_top_up_and_extend(&rando, &1_000, &100);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_sender_non_operator_rejected_by_clawback() {
    let s = Setup::new(100, 3600, true);
    let rando = Address::generate(&s.env);
    let result = s.client.try_clawback(&rando);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

// ── Revoked operator loses all delegated rights ────────────────────────────

#[test]
fn revoked_operator_rejected_by_cancel() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let result = s.client.try_cancel(&operator);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoked_operator_rejected_by_pause() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let result = s.client.try_pause(&operator);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoked_operator_can_still_top_up_gift_funding() {
    // Issue #690: any authenticated account can top up as a gift
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let tok_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    tok_admin.mint(&operator, &1_000);
    let result = s.client.try_top_up(&operator, &1_000);
    assert_eq!(result, Ok(Ok(())));
}

#[test]
fn revoked_operator_rejected_by_clawback() {
    let s = Setup::new(100, 3600, true);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let result = s.client.try_clawback(&operator);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoked_operator_rejected_by_extend_duration() {
    let s = Setup::new(100, 3_600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let result = s.client.try_extend_duration(&operator, &100);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoked_operator_rejected_by_top_up_and_extend() {
    let s = Setup::new(100, 3_600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);
    s.client.revoke_operator(&s.sender);

    let result = s.client.try_top_up_and_extend(&operator, &1_000, &100);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

// ── Sender retains access after setting operator ───────────────────────────

#[test]
fn sender_still_can_cancel_after_setting_operator() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.advance_secs(900);
    s.client.cancel(&s.sender);
    assert!(s.client.info().is_cancelled());
}

#[test]
fn sender_still_can_pause_after_setting_operator() {
    let s = Setup::new(100, 3600, false);
    let operator = Address::generate(&s.env);
    s.client.set_operator(&s.sender, &operator);

    s.client.pause(&s.sender);
    assert!(s.client.info().is_paused());
}

// ── Operator cannot access recipient-only functions ────────────────────────

// withdraw() and transfer_recipient() are gated by info.recipient.require_auth(),
// not by require_sender_or_operator. They don't take a caller parameter — the
// auth is checked against the stored recipient address. An operator cannot
// satisfy this auth because they are not the recipient. This is already covered
// by the existing withdraw tests (which always pass recipient auth) and would
// require disabling mock_all_auths() to test negative cases properly.
// ── State storage: save() writes only Config; legacy keys are cleaned ─────────
//
// `save()` used to mirror the 9 individual per-field keys alongside the
// consolidated `Config` on every mutation, but `load()` reads `Config` on its
// fast path (always true for post-consolidation streams). Those mirrors were
// never read — pure write/rent overhead on every state mutation. `save()` now
// writes only `Config` and, on the first `save()` of a pre-consolidation
// stream, removes the legacy keys one-time.

#[test]
fn initialize_writes_split_metadata_and_state() {
    let s = Setup::new(100, 3600, false);

    let (has_metadata, has_state, has_legacy_config) = s.env.as_contract(&s.client.address, || {
        let storage = s.env.storage().instance();
        (
            storage.has(&DataKey::Metadata),
            storage.has(&DataKey::State),
            storage.has(&DataKey::Config),
        )
    });

    assert!(has_metadata, "Metadata must be written on initialize");
    assert!(has_state, "State must be written on initialize");
    assert!(
        !has_legacy_config,
        "new streams must not write legacy Config"
    );

    // initialize() must expose the full state via load()/info().
    let info = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));
    assert_eq!(info.rate_per_second, 100);
    assert!(!info.is_clawback_enabled());
}

#[test]
fn event_sequence_is_persisted_in_config() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.pause(&s.sender);
    s.client.resume(&s.sender);

    let has_state = s.env.as_contract(&s.client.address, || {
        s.env.storage().instance().has(&DataKey::State)
    });

    assert!(
        has_state,
        "State must be present after event-driven updates"
    );

    let info = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));
    assert_eq!(
        info.event_sequence, 3,
        "pause/resume emits two events after init"
    );
    assert_eq!(s.client.event_sequence(), 3);
}

#[test]
fn state_mutation_keeps_metadata_separate_from_state() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(100);
    s.client.withdraw(&10_000); // drives state::save

    let (has_metadata, has_state, has_legacy_config) = s.env.as_contract(&s.client.address, || {
        let storage = s.env.storage().instance();
        (
            storage.has(&DataKey::Metadata),
            storage.has(&DataKey::State),
            storage.has(&DataKey::Config),
        )
    });
    assert!(has_metadata);
    assert!(has_state);
    assert!(!has_legacy_config);

    // The mutation's effect must be durable through State.
    let info = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));
    assert_eq!(info.withdrawn, 10_000);
}

#[test]
fn flag_getters_map_to_correct_bit() {
    // Regression test for the merged FLAG_ClAWBACK_ENABLED typo: a table-driven
    // check guarantees each getter masks exactly the documented bit.
    let env = Env::default();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token = Address::generate(&env);
    let base = StreamInfo {
        sender: sender.clone(),
        recipient: recipient.clone(),
        token: token.clone(),
        rate_per_second: 1,
        start_time: 0,
        end_time: 1,
        withdrawn: 0,
        paused_at: 0,
        flags: 0,
        event_sequence: 0,
        cumulative_fees: 0,
    };

    #[allow(clippy::type_complexity)]
    let cases: [(u32, fn(&StreamInfo) -> bool, &str); 3] = [
        (FLAG_PAUSED, StreamInfo::is_paused, "paused"),
        (
            FLAG_CLAWBACK_ENABLED,
            StreamInfo::is_clawback_enabled,
            "clawback_enabled",
        ),
        (FLAG_CANCELLED, StreamInfo::is_cancelled, "cancelled"),
    ];

    for (flag, getter, name) in cases {
        let mut info = base.clone();
        info.flags = flag;
        assert!(
            getter(&info),
            "is_{} must be true when flags={}",
            name,
            flag
        );
        info.flags = 0;
        assert!(!getter(&info), "is_{} must be false when flags=0", name);
    }
}

#[test]
fn withdraw_remaining_is_zero_when_draining_full_balance() {
    // Withdrawing the entire funded balance must not panic and must leave the
    // `withdrawn` event's `remaining` field at 0, matching the contract's real
    // post-withdrawal token balance (#415).
    let s = Setup::new(100, 3600, false);
    s.advance_secs(3600); // end reached; streamed == full deposit (360_000)
    s.client.withdraw(&360_000);
    let contract_after = s.token.balance(&s.client.address);
    assert_eq!(contract_after, 0);

    let all_events = s.env.events().all();
    let stream_events: std::vec::Vec<_> = all_events
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .collect();
    // The final event of the call is `completed` (a single i128); pick the
    // most recent event whose data is the withdrawn 3-tuple instead.
    let data: (i128, i128, i128) = stream_events
        .iter()
        .rev()
        .filter_map(|(_, _, value)| value.try_into_val(&s.env).ok())
        .next()
        .unwrap();
    let (_amount, _total, remaining) = data;
    assert_eq!(remaining, 0);
    assert_eq!(remaining, contract_after);
}

// ── streamed_amount boundary tests (issue #444) ────────────────────────────

#[test]
fn zero_duration_stream_is_rejected() {
    // end_time == start_time is malformed: it would stream nothing.
    // initialize must reject it with InvalidTimeRange before persisting.
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    let deposit = 100i128;
    tok_admin.mint(&sender, &deposit);

    let now: u64 = 1_000_000;
    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    tok.transfer(&sender, &stream_id, &deposit);

    let result = client.try_initialize(
        &sender,
        &recipient,
        &token_addr,
        &100i128,
        &now,
        &now, // end_time == start_time
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
    assert!(result.is_err(), "zero-duration stream must be rejected");
}

#[test]
fn open_ended_stream_accrues_indefinitely() {
    // end_time == 0: stream never ends.
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    let deposit = 10_000_000i128;
    tok_admin.mint(&sender, &deposit);

    let now: u64 = 1_000_000;
    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    tok.transfer(&sender, &stream_id, &deposit);

    // open-ended: end_time = 0
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,
        &0,
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    // Advance 1000s → 100_000 stroops accrued.
    env.ledger().set(LedgerInfo {
        timestamp: now + 1000,
        ..env.ledger().get()
    });
    assert_eq!(client.streamed_total(), 100_000);

    // Advance another 1000s → 200_000 stroops accrued.
    env.ledger().set(LedgerInfo {
        timestamp: now + 2000,
        ..env.ledger().get()
    });
    assert_eq!(client.streamed_total(), 200_000);
}

#[test]
fn cancelled_stream_returns_zero_streamed() {
    let s = Setup::new(100, 3600, false);
    s.client.cancel(&s.sender);
    // After cancellation, streamed_total should be 0.
    assert_eq!(s.client.streamed_total(), 0);
    assert_eq!(s.client.withdrawable(), 0);
}

#[test]
fn near_max_i128_rate_does_not_overflow() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    // rate = floor(i128::MAX / 60), elapsed = 60 → the largest product that
    // still fits i128 for the minimum stream duration.
    let rate = i128::MAX / 60;
    let deposit = rate * 60;
    tok_admin.mint(&sender, &deposit);

    let now: u64 = 1_000_000;
    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let tok = token::Client::new(&env, &token_addr);
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    tok.transfer(&sender, &stream_id, &deposit);

    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &(now + 60),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    env.ledger().set(LedgerInfo {
        timestamp: now + 60,
        ..env.ledger().get()
    });

    assert_eq!(client.streamed_total(), rate * 60);
}

#[test]
fn rate_elapsed_overflow_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    // Use an open-ended stream (end_time = 0) so effective_now is NOT clamped.
    // rate = i128::MAX / 2. After elapsed = 3, rate * 3 overflows.
    let rate = i128::MAX / 2;
    let deposit = rate.checked_mul(3).unwrap_or(i128::MAX);
    tok_admin.mint(&sender, &deposit);

    let now: u64 = 1_000_000;
    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    tok.transfer(&sender, &stream_id, &deposit);

    // open-ended: end_time = 0
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &now,
        &0,
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );

    // Advance so elapsed = 3. rate * 3 = i128::MAX / 2 * 3 > i128::MAX.
    env.ledger().set(LedgerInfo {
        timestamp: now + 3,
        ..env.ledger().get()
    });

    let result = client.try_streamed_total();
    assert!(result.is_err(), "rate * elapsed overflow must be rejected");
}

// ── Re-entrancy guard convention audit (issue #442) ────────────────────────

#[test]
fn every_state_mutating_entry_point_uses_with_guard() {
    // This test is a living checklist: when a new state-mutating pub fn is
    // added, it must be added here and must go through state::with_guard.
    // The list below was derived by grepping for `state::with_guard` and
    // `state::save` in lib.rs and cross-referencing every `pub fn` in
    // #[contractimpl].
    //
    // Allowed exceptions (documented in the module-level doc comment):
    // - initialize   — one-shot, no prior state
    // - set_operator — single slot, no external call
    // - revoke_operator — same as above

    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let tok = token::Client::new(&env, &token_addr);
    let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

    let deposit = 360_000i128;
    tok_admin.mint(&sender, &deposit);

    let now: u64 = 1_000_000;
    env.ledger().set(LedgerInfo {
        timestamp: now,
        protocol_version: 21,
        sequence_number: 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 4096,
        max_entry_ttl: 6_312_000,
    });

    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    tok.transfer(&sender, &stream_id, &deposit);

    // initialize with clawback enabled so clawback can be tested
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &100,
        &now,
        &(now + 3600),
        &true,
        &2_592_000_u64,
        &false,
        &None,
    );

    // Advance past start time so there is something to withdraw.
    env.ledger().set(LedgerInfo {
        timestamp: now + 10,
        ..env.ledger().get()
    });

    // These must all succeed — proving they are reachable and do not panic
    // on the guard path. If any future refactor removes with_guard from one
    // of these, the test still compiles but the convention is broken.
    client.withdraw(&100);
    client.pause(&sender);
    client.resume(&sender);

    // Mint more tokens for top_up (sender balance was reduced by deposit).
    tok_admin.mint(&sender, &500);
    client.top_up(&sender, &100);

    // Clawback last — returns remaining balance to sender.
    // Note: clawback does NOT cancel the stream; it only recovers
    // unstreamed funds. The stream remains active until end_time.
    client.clawback(&sender);

    // Verify stream is still active (not cancelled) and tracks streamed amount.
    assert_eq!(client.streamed_total(), 1000);
}

// ── Single guard API regression (issues #453 / #454) ───────────────────────

#[test]
fn stream_exposes_exactly_one_reentrancy_guard_api() {
    // Issues #453/#454: a merged PR (#373) left a second, unused guard
    // mechanism in `storage.rs` — `read_guard`/`write_guard` helpers plus the
    // `GUARD_NOT_ENTERED`/`GUARD_ENTERED` constants they used — coexisting
    // with the real depth-counter guard (`state::lock`/`unlock`/`with_guard`
    // at `DataKey::Guard`). The dead pair was deleted so there is exactly one
    // guard API; this test is the living regression check that keeps it that
    // way. `include_str!` embeds the source at compile time, so the assertion
    // is a pure string check with no I/O.
    let storage_src = include_str!("storage.rs");
    for dead in [
        "read_guard",
        "write_guard",
        "GUARD_NOT_ENTERED",
        "GUARD_ENTERED",
    ] {
        assert!(
            !storage_src.contains(dead),
            "dead reentrancy-guard helper `{dead}` reappeared in storage.rs — \
             the stream contract must expose a single guard API \
             (state::lock / state::unlock / state::with_guard)"
        );
    }

    // The live guard must remain the depth-counter API in `state.rs`, wired
    // to `DataKey::Guard`. The fn-pointer references below additionally fail
    // to compile if `lock`/`unlock` are ever removed or reshaped.
    let state_src = include_str!("state.rs");
    assert!(
        state_src.contains("pub fn with_guard"),
        "state::with_guard missing — the depth-counter guard is the one guard API"
    );
    assert!(
        state_src.contains("DataKey::Guard"),
        "state guard no longer stored at DataKey::Guard"
    );

    let _lock: fn(&Env) -> Result<(), Error> = crate::state::lock;
    let _unlock: fn(&Env) = crate::state::unlock;
}

// ── Split recipient tests ─────────────────────────────────────────────────

#[test]
fn set_split_recipient_and_withdraw_splits_proportionally() {
    let s = Setup::new(100, 3600, false);
    let secondary = Address::generate(&s.env);

    // Sender configures split of 2000 bps (20%)
    s.client.set_split_recipient(&s.sender, &secondary, &2000);

    let split = s.client.split_recipient().unwrap();
    assert_eq!(split.secondary_recipient, secondary);
    assert_eq!(split.split_bps, 2000);

    // Advance 10 seconds -> 1000 tokens accrued
    s.advance_secs(10);

    let recipient_before = s.token.balance(&s.recipient);
    let secondary_before = s.token.balance(&secondary);

    s.client.withdraw(&1000);

    let recipient_after = s.token.balance(&s.recipient);
    let secondary_after = s.token.balance(&secondary);

    // 80% to primary (800), 20% to secondary (200)
    assert_eq!(recipient_after - recipient_before, 800);
    assert_eq!(secondary_after - secondary_before, 200);
}

#[test]
fn recipient_can_configure_and_remove_split() {
    let s = Setup::new(100, 3600, false);
    let secondary = Address::generate(&s.env);

    // Recipient configures split of 3000 bps (30%)
    s.client
        .set_split_recipient(&s.recipient, &secondary, &3000);
    assert_eq!(s.client.split_recipient().unwrap().split_bps, 3000);

    // Recipient removes split
    s.client.remove_split_recipient(&s.recipient);
    assert_eq!(s.client.split_recipient(), None);

    s.advance_secs(10);
    let recipient_before = s.token.balance(&s.recipient);
    let secondary_before = s.token.balance(&secondary);

    s.client.withdraw(&1000);

    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 1000);
    assert_eq!(s.token.balance(&secondary) - secondary_before, 0);
}

#[test]
fn cancel_stream_splits_accrued_tokens_proportionally() {
    let s = Setup::new(100, 3600, false);
    let secondary = Address::generate(&s.env);

    s.client.set_split_recipient(&s.sender, &secondary, &2500); // 25%

    // Advance 20 seconds -> 2000 tokens accrued
    s.advance_secs(20);

    let recipient_before = s.token.balance(&s.recipient);
    let secondary_before = s.token.balance(&secondary);

    s.client.cancel(&s.sender);

    // 75% to primary (1500), 25% to secondary (500)
    assert_eq!(s.token.balance(&s.recipient) - recipient_before, 1500);
    assert_eq!(s.token.balance(&secondary) - secondary_before, 500);
}

#[test]
fn split_recipient_validation_rejects_invalid_inputs() {
    let s = Setup::new(100, 3600, false);
    let stranger = Address::generate(&s.env);
    let secondary = Address::generate(&s.env);

    // Unauthorized caller
    assert_eq!(
        s.client
            .try_set_split_recipient(&stranger, &secondary, &2000),
        Err(Ok(Error::NotAuthorized))
    );

    // Zero basis points
    assert_eq!(
        s.client.try_set_split_recipient(&s.sender, &secondary, &0),
        Err(Ok(Error::InvalidAmount))
    );

    // >= 10_000 basis points
    assert_eq!(
        s.client
            .try_set_split_recipient(&s.sender, &secondary, &10000),
        Err(Ok(Error::InvalidAmount))
    );

    // Secondary cannot be sender
    assert_eq!(
        s.client
            .try_set_split_recipient(&s.sender, &s.sender, &2000),
        Err(Ok(Error::InvalidRecipient))
    );

    // Secondary cannot be recipient
    assert_eq!(
        s.client
            .try_set_split_recipient(&s.sender, &s.recipient, &2000),
        Err(Ok(Error::InvalidRecipient))
    );
}

#[test]
fn test_get_summary_returns_consolidated_state() {
    let s = Setup::new(100, 3600, false);
    let now = s.env.ledger().timestamp();
    let summary = s.client.get_summary();
    assert_eq!(summary.sender, s.sender);
    assert_eq!(summary.recipient, s.recipient);
    assert_eq!(summary.token, s.token.address);
    assert_eq!(summary.total_amount, 360_000);
    assert_eq!(summary.streamed_amount, 0);
    assert_eq!(summary.status, crate::storage::StreamStatus::Active);
    assert_eq!(summary.start_time, now);
    assert_eq!(summary.stop_time, now + 3600);

    // Also verify get_stream_summary alias
    let alias_summary = s.client.get_stream_summary();
    assert_eq!(summary, alias_summary);

    // Advance 100 seconds
    s.advance_secs(100);
    let summary2 = s.client.get_summary();
    assert_eq!(summary2.streamed_amount, 10_000);
    assert_eq!(summary2.status, crate::storage::StreamStatus::Active);

    // Pause the stream
    s.client.pause(&s.sender);
    let paused_summary = s.client.get_summary();
    assert_eq!(paused_summary.status, crate::storage::StreamStatus::Paused);

    // Resume the stream
    s.client.resume(&s.sender);
    let active_summary = s.client.get_summary();
    assert_eq!(active_summary.status, crate::storage::StreamStatus::Active);

    // Advance past end time
    s.advance_secs(4000);
    let completed_summary = s.client.get_summary();
    assert_eq!(
        completed_summary.status,
        crate::storage::StreamStatus::Completed
    );
    assert_eq!(completed_summary.streamed_amount, 360_000);

    // Cancel test on new setup
    let s2 = Setup::new(100, 3600, false);
    s2.client.cancel(&s2.sender);
    let cancelled_summary = s2.client.get_summary();
    assert_eq!(
        cancelled_summary.status,
        crate::storage::StreamStatus::Cancelled
    );
}

#[test]
fn test_third_party_top_up_gift_funding() {
    let s = Setup::new(100, 3600, false);
    let third_party = Address::generate(&s.env);

    // Mint tokens to third party
    let tok_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    tok_admin.mint(&third_party, &50_000);

    // Third party tops up the stream
    assert!(s.client.try_top_up(&third_party, &50_000).is_ok());

    let balance = s.token.balance(&s.client.address);
    assert_eq!(balance, 360_000 + 50_000);
}

#[test]
fn test_cliff_unlock_percentage_and_linear_streaming() {
    let s = Setup::new(100, 3600, false);
    let start = s.env.ledger().timestamp();
    let cliff_time = start + 1000;
    let upfront_unlock = 72_000; // 20% of 360,000

    // Set cliff
    assert!(s
        .client
        .try_set_cliff(&s.sender, &cliff_time, &upfront_unlock)
        .is_ok());

    // Before cliff: 0 withdrawable
    s.advance_secs(500);
    assert_eq!(s.client.streamed_total(), 0);

    // At cliff: upfront 72,000 unlocked immediately
    s.advance_secs(500); // now at start + 1000
    assert_eq!(s.client.streamed_total(), 72_000);

    // 100 seconds after cliff: 72,000 + 100 * 100 = 82,000
    s.advance_secs(100);
    assert_eq!(s.client.streamed_total(), 82_000);
}

#[test]
fn cliff_at_end_time_unlocks_full_stream_without_arithmetic_panic() {
    let s = Setup::new(100, 100, false);
    let end_time = s.client.info().end_time;
    let full_deposit = 10_000;

    assert!(s
        .client
        .try_set_cliff(&s.sender, &end_time, &full_deposit)
        .is_ok());

    s.advance_secs(100);
    assert_eq!(s.env.ledger().timestamp(), end_time);
    assert_eq!(s.client.withdrawable(), full_deposit);
    assert_eq!(s.client.withdraw(&full_deposit), full_deposit);
    assert_eq!(s.token.balance(&s.recipient), full_deposit);
}

#[test]
fn test_min_withdrawal_interval_enforcement() {
    let s = Setup::new(100, 3600, false);
    // Set 60 seconds minimum interval
    assert!(s
        .client
        .try_set_min_withdrawal_interval(&s.sender, &60)
        .is_ok());
    assert_eq!(s.client.min_withdrawal_interval(), 60);

    // Advance 100s, withdraw
    s.advance_secs(100);
    assert_eq!(s.client.withdraw(&1000), 1000);
    assert_eq!(s.client.last_withdrawal_time(), s.env.ledger().timestamp());

    // Advance 30s (< 60s), second withdrawal should fail with WithdrawalTooFrequent
    s.advance_secs(30);
    let res = s.client.try_withdraw(&1000);
    assert_eq!(
        res.err().unwrap().unwrap(),
        crate::errors::Error::WithdrawalTooFrequent
    );

    // Advance another 30s (total 60s elapsed since last withdrawal), should succeed
    s.advance_secs(30);
    assert_eq!(s.client.withdraw(&1000), 1000);
}

#[test]
fn test_set_min_withdrawal_interval_auth() {
    let s = Setup::new(100, 3600, false);
    let unauthorized = Address::generate(&s.env);

    let res = s.client.try_set_min_withdrawal_interval(&unauthorized, &60);
    assert_eq!(
        res.err().unwrap().unwrap(),
        crate::errors::Error::NotAuthorized
    );
}

// ── Issue #621: validate_withdraw dry-run query ─────────────────────────────

#[test]
fn validate_withdraw_success_and_does_not_mutate_state() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(10);

    let withdrawable = s.client.withdrawable();
    assert_eq!(withdrawable, 1000);

    // Validate a partial withdrawal amount
    assert_eq!(s.client.try_validate_withdraw(&500), Ok(Ok(())));

    // Validate the exact withdrawable amount
    assert_eq!(s.client.try_validate_withdraw(&1000), Ok(Ok(())));

    // Verify dry-run did not mutate withdrawn amount or transfer tokens
    let info = s.client.info();
    assert_eq!(info.withdrawn, 0);
    assert_eq!(s.token.balance(&s.recipient), 0);

    // Actual withdraw succeeds afterwards
    assert_eq!(s.client.withdraw(&500), 500);
    assert_eq!(s.token.balance(&s.recipient), 500);
}

#[test]
fn validate_withdraw_rejects_zero_and_negative_amount() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(10);

    let res_zero = s.client.try_validate_withdraw(&0);
    assert_eq!(
        res_zero.err().unwrap().unwrap(),
        crate::errors::Error::InvalidAmount
    );

    let res_neg = s.client.try_validate_withdraw(&-100);
    assert_eq!(
        res_neg.err().unwrap().unwrap(),
        crate::errors::Error::InvalidAmount
    );
}

#[test]
fn validate_withdraw_rejects_before_any_elapsed() {
    let s = Setup::new(100, 3600, false);
    // At start timestamp 0, nothing has accrued yet
    let res = s.client.try_validate_withdraw(&100);
    assert_eq!(
        res.err().unwrap().unwrap(),
        crate::errors::Error::NothingToWithdraw
    );
}

#[test]
fn validate_withdraw_rejects_after_cancelled() {
    let s = Setup::new(100, 3600, false);
    s.advance_secs(10);
    s.client.cancel(&s.sender);

    let res = s.client.try_validate_withdraw(&100);
    assert_eq!(
        res.err().unwrap().unwrap(),
        crate::errors::Error::StreamCancelled
    );
}

#[test]
fn validate_withdraw_respects_min_interval() {
    let s = Setup::new(100, 3600, false);
    assert!(s
        .client
        .try_set_min_withdrawal_interval(&s.sender, &60)
        .is_ok());

    s.advance_secs(100);
    assert_eq!(s.client.withdraw(&1000), 1000);

    // 30s elapsed (< 60s min interval)
    s.advance_secs(30);
    let res = s.client.try_validate_withdraw(&500);
    assert_eq!(
        res.err().unwrap().unwrap(),
        crate::errors::Error::WithdrawalTooFrequent
    );

    // 30s more elapsed (total 60s)
    s.advance_secs(30);
    assert_eq!(s.client.try_validate_withdraw(&500), Ok(Ok(())));
}

#[test]
fn test_immutable_config_stored_in_instance_storage() {
    let s = Setup::new(100, 3600, false);
    let cfg = s.client.immutable_config();
    assert_eq!(cfg.rate_per_second, 100);
    assert_eq!(cfg.start_time, s.env.ledger().timestamp());
    assert_eq!(cfg.stop_time, s.env.ledger().timestamp() + 3600);
    assert_eq!(cfg.sender, s.sender);
    assert_eq!(cfg.recipient, s.recipient);
    assert_eq!(cfg.token, s.token.address);
}

#[test]
fn test_cliff_reached_and_stream_completed_events() {
    let s = Setup::new(100, 1000, false);
    let start = s.env.ledger().timestamp();
    let cliff_time = start + 500;
    let upfront_unlock = 10_000;

    // Configure cliff
    assert!(s
        .client
        .try_set_cliff(&s.sender, &cliff_time, &upfront_unlock)
        .is_ok());

    // Advance to cliff time
    s.advance_secs(500);

    // Initial withdrawal after cliff reaches the milestone and emits cliff_rch
    let withdrawn = s.client.withdraw(&10_000);
    assert_eq!(withdrawn, 10_000);

    // Second withdrawal after cliff should NOT emit cliff_rch again (one-shot milestone)
    s.advance_secs(100);
    let withdrawn2 = s.client.withdraw(&5_000);
    assert_eq!(withdrawn2, 5_000);

    // Advance to end of stream and withdraw remaining balance to trigger completion
    s.advance_secs(400); // at end of stream (1000s)
    let available = s.client.withdrawable();
    let final_withdrawn = s.client.withdraw(&available);
    assert!(final_withdrawn > 0);

    // A cliff stream only accrues from `cliff_time`; the pre-cliff portion of
    // the deposit remains unstreamed and recoverable via `cancel()`, so the
    // withdrawable amount (not the contract balance) is what reaches zero.
    assert_eq!(s.client.withdrawable(), 0);
}

#[test]
fn test_non_cancelable_stream_rejects_cancel() {
    let s = Setup::new(100, 3600, false);

    // By default stream is cancelable
    assert!(s.client.is_cancelable());

    // Sender marks stream as non-cancelable
    s.client.set_cancelable(&s.sender, &false);
    assert_eq!(s.client.is_cancelable(), false);

    let cfg = s.client.stream_config().unwrap();
    assert_eq!(cfg.is_cancelable, false);

    // Cancellation is rejected
    assert_eq!(
        s.client.try_cancel(&s.sender),
        Err(Ok(Error::StreamNotCancelable))
    );

    // If sender re-enables cancelability, cancellation succeeds
    s.client.set_cancelable(&s.sender, &true);
    assert_eq!(s.client.is_cancelable(), true);
    assert!(s.client.try_cancel(&s.sender).is_ok());
}

#[test]
fn test_stream_auto_completion_at_exact_duration() {
    let s = Setup::new(100, 1000, false);
    let _start = s.env.ledger().timestamp();

    s.advance_secs(999);
    let before_end = s.client.withdrawable();
    assert!(before_end > 0);

    s.advance_secs(1);

    let at_end = s.client.withdrawable();
    assert_eq!(at_end, 100_000);

    let withdrawn = s.client.withdraw(&at_end);
    assert_eq!(withdrawn, 100_000);

    assert_eq!(s.token.balance(&s.client.address), 0);
}

// ── Issue #847: minimum stream duration ─────────────────────────────────────

#[test]
#[should_panic(expected = "Error(Contract, #26)")]
fn initialize_rejects_a_one_second_stream() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    let now = env.ledger().timestamp();
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &1,
        &now,
        &(now + 1),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
}

#[test]
fn initialize_accepts_a_stream_at_the_minimum_duration() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    let now = env.ledger().timestamp();
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &1,
        &now,
        &(now + crate::MIN_STREAM_DURATION),
        &false,
        &2_592_000_u64,
        &false,
        &None,
    );
    assert_eq!(client.info().end_time, now + crate::MIN_STREAM_DURATION);
}

// ── Issue #845: optional description ────────────────────────────────────────

#[test]
fn initialize_stores_and_exposes_the_description() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    let now = env.ledger().timestamp();
    let description = String::from_str(&env, "Q4 payroll - design team");
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &1,
        &now,
        &(now + 3_600),
        &false,
        &2_592_000_u64,
        &false,
        &Some(description.clone()),
    );
    assert_eq!(client.description(), Some(description));
}

#[test]
#[should_panic(expected = "Error(Contract, #27)")]
fn initialize_rejects_a_description_over_128_bytes() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);
    let now = env.ledger().timestamp();
    let too_long = String::from_str(&env, &"x".repeat(129));
    client.initialize(
        &sender,
        &recipient,
        &token_addr,
        &1,
        &now,
        &(now + 3_600),
        &false,
        &2_592_000_u64,
        &false,
        &Some(too_long),
    );
}

// ── Issue #846: duration-scaled instance TTL ────────────────────────────────

#[test]
fn long_streams_get_a_longer_instance_ttl_than_short_streams() {
    let short = Setup::new(1, 3_600, false);
    let long = Setup::new(1, 31_536_000, false); // one year

    let short_ttl = short.env.as_contract(&short.client.address, || {
        short.env.storage().instance().get_ttl()
    });
    let long_ttl = long.env.as_contract(&long.client.address, || {
        long.env.storage().instance().get_ttl()
    });

    assert!(
        long_ttl > short_ttl,
        "one-year stream TTL {long_ttl} must exceed one-hour stream TTL {short_ttl}"
    );
}

// ── Issue #810: secondary backup recovery address in stream state ───────────

#[test]
fn recovery_address_configuration_and_timelocked_claim() {
    let s = Setup::new(1, 3_600, false);
    let recovery = Address::generate(&s.env);
    let new_recipient = Address::generate(&s.env);

    // Recipient configures secondary backup recovery address with 300s timelock delay
    s.client.set_recovery_address(&recovery, &300);
    assert_eq!(s.client.recovery_address(), Some(recovery.clone()));
    let claimable_at = s.client.recovery_claimable_at().unwrap();
    assert_eq!(claimable_at, s.env.ledger().timestamp() + 300);

    // Premature claim fails
    assert_eq!(
        s.client.try_claim_recovery(&new_recipient),
        Err(Ok(Error::InvalidTimeRange))
    );

    // Advance time past the timelock
    s.env.ledger().with_mut(|l| l.timestamp += 301);

    // Recovery address successfully claims stream, updating recipient
    assert!(s.client.try_claim_recovery(&new_recipient).is_ok());
    assert_eq!(s.client.recipient(), new_recipient);

    // Recovery config is cleared after successful claim
    assert_eq!(s.client.recovery_address(), None);
    assert_eq!(s.client.recovery_claimable_at(), None);
}

// ── Issues #806-#809: cliff error, recipient transferred event, elapsed_seconds, auto-settle ──

#[test]
fn withdrawal_before_cliff_returns_stream_cliff_in_future() {
    let s = Setup::new(1, 3_600, false);
    let cliff_time = s.env.ledger().timestamp() + 1_000;
    s.client.set_cliff(&s.sender, &cliff_time, &100);

    // Attempting to withdraw before cliff returns StreamCliffInFuture (Issue #806)
    assert_eq!(
        s.client.try_withdraw(&50),
        Err(Ok(Error::StreamCliffInFuture))
    );

    // Advance time past the cliff
    s.env.ledger().with_mut(|l| l.timestamp = cliff_time + 10);
    assert!(s.client.try_withdraw(&50).is_ok());
}

#[test]
fn recipient_transfer_emits_event_with_old_and_new_recipients() {
    let s = Setup::new(1, 3_600, false);
    let new_recipient = Address::generate(&s.env);

    s.client.transfer_recipient(&new_recipient);

    let all_events = s.env.events().all();
    let expected_topics = (
        soroban_sdk::Symbol::new(&s.env, "StreamRecipientTransferred"),
        s.recipient.clone(),
        2_u64,
    )
        .into_val(&s.env);
    let matched = all_events
        .iter()
        .find(|(contract, topics, _)| contract == &s.client.address && topics == &expected_topics);
    assert!(
        matched.is_some(),
        "StreamRecipientTransferred event with old and new recipient must be emitted"
    );
}

#[test]
fn elapsed_seconds_calculates_stream_age_correctly() {
    let s = Setup::new(10, 1_000, false);
    let info = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));

    // Before start
    assert_eq!(info.elapsed_seconds(info.start_time.saturating_sub(10)), 0);
    // At start
    assert_eq!(info.elapsed_seconds(info.start_time), 0);
    // Mid stream
    assert_eq!(info.elapsed_seconds(info.start_time + 250), 250);
    // At end
    assert_eq!(info.elapsed_seconds(info.end_time), 1_000);
    // Past end (capped at stream duration)
    assert_eq!(info.elapsed_seconds(info.end_time + 500), 1_000);
}

#[test]
fn is_fully_vested_tracks_the_bounded_end_time() {
    let s = Setup::new(10, 1_000, false);
    let info = s
        .env
        .as_contract(&s.client.address, || crate::state::load(&s.env));

    assert!(!info.is_fully_vested(info.start_time));
    assert!(!info.is_fully_vested(info.end_time.saturating_sub(1)));
    assert!(info.is_fully_vested(info.end_time));
    assert!(info.is_fully_vested(info.end_time + 1));
    assert!(crate::state::is_fully_vested(&info, info.end_time));

    // Open-ended streams have no scheduled completion.
    let mut open_ended = info.clone();
    open_ended.end_time = 0;
    assert!(!open_ended.is_fully_vested(u64::MAX));
}

#[test]
fn auto_settle_full_remaining_balance_on_stream_end() {
    let s = Setup::new(1, 100, false);
    let now = s.env.ledger().timestamp();

    // Cannot auto-settle when disabled
    assert_eq!(s.client.try_auto_settle(), Err(Ok(Error::NotAuthorized)));

    // Enable auto-settle
    s.client.set_auto_settle(&s.recipient, &true);
    assert!(s.client.auto_settle_enabled());

    // Cannot auto-settle before stream end
    assert_eq!(s.client.try_auto_settle(), Err(Ok(Error::InvalidTimeRange)));

    // Advance to stream end
    s.env.ledger().with_mut(|l| l.timestamp = now + 101);

    // Keeper triggers auto-settle without recipient signature
    let settled = s.client.auto_settle();
    assert_eq!(settled, 100);
    assert_eq!(s.token.balance(&s.recipient), 100);
    assert_eq!(s.token.balance(&s.client.address), 0);
}

#[test]
fn test_enforce_minimum_cliff_amount_when_cliff_timestamp_configured() {
    let s = Setup::new(1, 3_600, false);
    let start_time = s.client.info().start_time;
    let cliff_time = start_time + 1_000;

    // Zero cliff amount with cliff_time > start_time is rejected with InvalidAmount (Issue #802)
    assert_eq!(
        s.client.try_set_cliff(&s.sender, &cliff_time, &0),
        Err(Ok(Error::InvalidAmount))
    );

    // Negative cliff amount is rejected with InvalidAmount
    assert_eq!(
        s.client.try_set_cliff(&s.sender, &cliff_time, &-10),
        Err(Ok(Error::InvalidAmount))
    );

    // Positive cliff amount succeeds
    assert!(s.client.try_set_cliff(&s.sender, &cliff_time, &100).is_ok());
    let cfg = s.client.cliff_config().unwrap();
    assert_eq!(cfg.cliff_unlock_amount, 100);
}

#[test]
fn test_stream_beneficiary_memo_field() {
    let s = Setup::new(1, 3_600, false);

    // Initially memo is None (Issue #803)
    assert_eq!(s.client.memo(), None);

    // Sender sets valid memo <= 64 bytes
    let memo_str = String::from_str(&s.env, "DEPT-ENG-402-EMP-883");
    s.client.set_memo(&s.sender, &Some(memo_str.clone()));
    assert_eq!(s.client.memo(), Some(memo_str.clone()));

    // Verify stream_config contains the memo
    let cfg = s.client.stream_config().unwrap();
    assert_eq!(cfg.memo, Some(memo_str));
    assert_eq!(cfg.is_cancelable, true);

    // Updating cancelable preserves existing memo
    s.client.set_cancelable(&s.sender, &false);
    assert_eq!(s.client.is_cancelable(), false);
    assert_eq!(s.client.memo().is_some(), true);

    // Updating memo preserves cancelable
    let new_memo = String::from_str(&s.env, "ACCOUNTING-RECON-OK");
    s.client.set_memo(&s.sender, &Some(new_memo.clone()));
    assert_eq!(s.client.is_cancelable(), false);
    assert_eq!(s.client.memo(), Some(new_memo));

    // Memo exceeding 64 bytes is rejected with DescriptionTooLong
    let long_memo = String::from_str(
        &s.env,
        "12345678901234567890123456789012345678901234567890123456789012345", // 65 bytes
    );
    assert_eq!(
        s.client.try_set_memo(&s.sender, &Some(long_memo)),
        Err(Ok(Error::DescriptionTooLong))
    );

    // Clearing memo to None succeeds
    s.client.set_memo(&s.sender, &None);
    assert_eq!(s.client.memo(), None);
}

#[test]
fn test_emergency_extend_stream_ttl_unauthenticated() {
    let s = Setup::new(1, 3_600, false);

    // Any caller can extend stream instance TTL without error (Issue #805)
    let res = s.client.try_extend_stream_ttl();
    assert!(res.is_ok());

    let res_alias = s.client.try_extend_ttl();
    assert!(res_alias.is_ok());

    // Balance and withdrawn state remain completely unchanged
    assert_eq!(s.token.balance(&s.client.address), 3_600);
    assert_eq!(s.client.info().withdrawn, 0);
}

// ── Issues #753-#755: is_depleted, pause_by_recipient, cancel_with_reason ──

#[test]
fn test_is_depleted_helper_method() {
    let s = Setup::new(10, 100, false);
    // At start, not depleted
    assert_eq!(s.client.is_depleted(), false);
    let mut info = s.client.info();
    assert_eq!(info.is_depleted(), false);
    assert_eq!(crate::state::is_depleted(&info), false);

    // Advance to end and withdraw everything
    s.advance_secs(100);
    s.client.withdraw(&1_000);
    assert_eq!(s.client.is_depleted(), true);
    info = s.client.info();
    assert_eq!(info.is_depleted(), true);
    assert_eq!(crate::state::is_depleted(&info), true);

    // Cancelled stream is also depleted
    let s2 = Setup::new(10, 100, false);
    s2.client.cancel(&s2.sender);
    assert_eq!(s2.client.is_depleted(), true);
    let info2 = s2.client.info();
    assert_eq!(info2.is_depleted(), true);
    assert_eq!(crate::state::is_depleted(&info2), true);
}

#[test]
fn test_recipient_initiated_pause() {
    let s = Setup::new(10, 100, false);
    s.advance_secs(20);

    // Non-recipient cannot call pause_by_recipient
    let stranger = Address::generate(&s.env);
    assert_eq!(
        s.client.try_pause_by_recipient(&stranger),
        Err(Ok(Error::NotAuthorized))
    );

    // Recipient can pause
    let res = s.client.try_pause_by_recipient(&s.recipient);
    assert!(res.is_ok());
    assert_eq!(s.client.info().is_paused(), true);

    // Calling again returns AlreadyPaused
    assert_eq!(
        s.client.try_pause_by_recipient(&s.recipient),
        Err(Ok(Error::AlreadyPaused))
    );
}

#[test]
fn test_cancel_with_reason_stored_in_persistent_storage() {
    let s = Setup::new(10, 100, false);
    s.advance_secs(40);

    // Cancel with reason 42
    let res = s.client.try_cancel_with_reason(&s.sender, &42);
    assert!(res.is_ok());

    let record = s.client.cancellation_record().expect("record must exist");
    assert_eq!(record.cancellation_reason, 42);
    assert_eq!(record.cancelled_by, s.sender);
    assert_eq!(record.cancelled_at, 1_000_000 + 40);
    assert_eq!(record.refund_amount, 600);
}

// ── Issues #750-#752: instance TTL extension, StreamAlreadyCompleted, StreamCliffReached ──

#[test]
fn test_top_up_and_withdraw_extend_instance_ttl() {
    let s = Setup::new(10, 3600, false);
    // Mint more tokens to sender for top-up
    let tok_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    tok_admin.mint(&s.sender, &500);

    // Initial TTL
    s.advance_secs(100);
    let res = s.client.try_top_up(&s.sender, &100);
    assert!(res.is_ok());

    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);

    // Withdraw also extends instance TTL
    s.advance_secs(50);
    let w_res = s.client.try_withdraw(&50);
    assert!(w_res.is_ok());
    let ttl_w = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl_w, 200_000);
}

#[test]
fn test_stream_already_completed_error() {
    let s = Setup::new(10, 100, false);
    s.advance_secs(100);

    // Draining full balance completes the stream
    s.client.withdraw(&1_000);
    assert!(s.client.is_depleted());

    // Subsequent withdrawal attempts return StreamAlreadyCompleted
    let res = s.client.try_withdraw(&1);
    assert_eq!(res, Err(Ok(Error::StreamAlreadyCompleted)));

    // validate_withdraw also returns StreamAlreadyCompleted
    let val_res = s.client.try_validate_withdraw(&1);
    assert_eq!(val_res, Err(Ok(Error::StreamAlreadyCompleted)));

    // Top up on a completed stream returns StreamAlreadyCompleted
    let tok_admin = token::StellarAssetClient::new(&s.env, &s.token.address);
    tok_admin.mint(&s.sender, &100);
    let top_res = s.client.try_top_up(&s.sender, &100);
    assert_eq!(top_res, Err(Ok(Error::StreamAlreadyCompleted)));
}

#[test]
fn test_stream_cliff_reached_event_and_cliff_claimed() {
    let s = Setup::new(10, 100, false);
    let cliff_time = s.client.info().start_time + 40;
    let cliff_amount = 200;
    s.client.set_cliff(&s.sender, &cliff_time, &cliff_amount);

    assert_eq!(s.client.is_cliff_claimed(), false);

    // Advance past cliff
    s.advance_secs(50);

    // First withdrawal triggers cliff emission and marks cliff as claimed
    let w = s.client.withdraw(&100);
    assert_eq!(w, 100);
    assert_eq!(s.client.is_cliff_claimed(), true);
    assert_eq!(s.client.cliff_claimed(), true);

    // Second withdrawal does not fail and cliff remains claimed
    let w2 = s.client.withdraw(&50);
    assert_eq!(w2, 50);
    assert_eq!(s.client.is_cliff_claimed(), true);
}

// ── Issue #923: creation-time metadata URI ──────────────────────────────────

fn init_with_metadata(
    env: &Env,
    metadata_uri: Option<String>,
) -> (DripStreamClient<'_>, Address, Address) {
    env.mock_all_auths();
    let sender = Address::generate(env);
    let recipient = Address::generate(env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(env))
        .address();
    let rate = 10i128;
    let start = env.ledger().timestamp();
    let end = start + 3_600;
    let deposit = rate * 3_600;
    token::StellarAssetClient::new(env, &token_addr).mint(&sender, &deposit);

    let stream_id = env.register_contract(None, DripStream);
    token::Client::new(env, &token_addr).transfer(&sender, &stream_id, &deposit);
    let client = DripStreamClient::new(env, &stream_id);

    client.initialize_with_metadata(
        &sender,
        &recipient,
        &token_addr,
        &rate,
        &start,
        &end,
        &false,
        &86_400u64,
        &false,
        &crate::StreamInitMetadata {
            description: None,
            metadata_uri,
        },
    );

    (client, sender, token_addr)
}

#[test]
fn initialize_with_metadata_stores_the_uri() {
    let env = Env::default();
    let uri = Some(String::from_str(&env, "ipfs://bafy/invoice-42"));
    let (client, _sender, _token) = init_with_metadata(&env, uri.clone());

    assert_eq!(client.metadata_uri(), uri);
}

#[test]
fn initialize_without_metadata_reads_none() {
    let env = Env::default();
    let (client, _sender, _token) = init_with_metadata(&env, None);

    assert_eq!(client.metadata_uri(), None);
}

#[test]
fn initialize_with_metadata_rejects_a_uri_over_128_bytes() {
    let env = Env::default();
    env.mock_all_auths();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let start = env.ledger().timestamp();
    let stream_id = env.register_contract(None, DripStream);
    let client = DripStreamClient::new(&env, &stream_id);

    let result = client.try_initialize_with_metadata(
        &sender,
        &recipient,
        &token_addr,
        &10i128,
        &start,
        &(start + 3_600),
        &false,
        &86_400u64,
        &false,
        &crate::StreamInitMetadata {
            description: None,
            metadata_uri: Some(String::from_str(&env, &"x".repeat(129))),
        },
    );

    assert!(result.is_err());
}

// ---- #933: sender-approved third-party funder top-up ----

fn mint_to_new_funder(s: &Setup, amount: i128) -> Address {
    let who = Address::generate(&s.env);
    token::StellarAssetClient::new(&s.env, &s.token.address).mint(&who, &amount);
    who
}

#[test]
fn approved_funder_top_up_increases_stream_balance() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    s.client.approve_funder(&s.sender, &funder);
    assert!(s.client.is_approved_funder(&funder));

    let before = s.token.balance(&s.client.address);
    s.client.top_up_by_funder(&funder, &1_000);

    assert_eq!(s.token.balance(&s.client.address), before + 1_000);
    assert_eq!(s.token.balance(&funder), 0);
}

#[test]
fn unapproved_funder_top_up_is_rejected() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    let result = s.client.try_top_up_by_funder(&funder, &1_000);
    assert_eq!(result, Err(Ok(Error::FunderNotApproved)));
    assert_eq!(s.token.balance(&funder), 1_000);
}

#[test]
fn non_sender_cannot_approve_funder() {
    let s = Setup::new(100, 3_600, false);
    let rando = Address::generate(&s.env);
    let funder = mint_to_new_funder(&s, 1_000);
    let result = s.client.try_approve_funder(&rando, &funder);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    assert!(!s.client.is_approved_funder(&funder));
}

#[test]
fn top_up_by_funder_rejects_zero_amount() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    s.client.approve_funder(&s.sender, &funder);
    let result = s.client.try_top_up_by_funder(&funder, &0);
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

// ---- #935: StreamPaused / StreamResumed events ----

fn has_event_named(env: &Env, name: &str) -> bool {
    let want = soroban_sdk::Symbol::new(env, name);
    for (_addr, topics, _data) in env.events().all().iter() {
        let first: Option<soroban_sdk::Symbol> =
            topics.get(0).and_then(|v| v.try_into_val(env).ok());
        if first == Some(want.clone()) {
            return true;
        }
    }
    false
}

#[test]
fn pause_emits_stream_paused_event() {
    let s = Setup::new(100, 3_600, false);
    s.client.pause(&s.sender);
    assert!(has_event_named(&s.env, "StreamPaused"));
}

#[test]
fn resume_emits_stream_resumed_event() {
    let s = Setup::new(100, 3_600, false);
    s.client.pause(&s.sender);
    s.client.resume(&s.sender);
    assert!(has_event_named(&s.env, "StreamResumed"));
}


#[test]
fn rejected_double_pause_emits_no_stream_paused_event() {
    let s = Setup::new(100, 3_600, false);
    s.client.pause(&s.sender);
    let result = s.client.try_pause(&s.sender);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
    assert!(!has_event_named(&s.env, "StreamPaused"));
}

#[test]
fn rejected_resume_when_active_emits_no_stream_resumed_event() {
    let s = Setup::new(100, 3_600, false);
    let result = s.client.try_resume(&s.sender);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
    assert!(!has_event_named(&s.env, "StreamResumed"));
}

// ---- #946: new error codes ----

#[test]
fn new_error_codes_match_issue_values() {
    assert_eq!(Error::StreamAlreadyPaused as u32, 120);
    assert_eq!(Error::StreamNotPaused as u32, 121);
    assert_eq!(Error::FunderNotApproved as u32, 122);
}

// ---- #933: revoke, retry, authorization and cancelled-stream edge cases ----

#[test]
fn revoked_funder_is_rejected_and_retry_succeeds_after_reapproval() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 2_000);
    s.client.approve_funder(&s.sender, &funder);
    s.client.revoke_funder(&s.sender, &funder);
    assert!(!s.client.is_approved_funder(&funder));

    let rejected = s.client.try_top_up_by_funder(&funder, &500);
    assert_eq!(rejected, Err(Ok(Error::FunderNotApproved)));
    assert_eq!(s.token.balance(&funder), 2_000);

    s.client.approve_funder(&s.sender, &funder);
    let retried = s.client.try_top_up_by_funder(&funder, &500);
    assert_eq!(retried, Ok(Ok(())));
    assert_eq!(s.token.balance(&funder), 1_500);
}

#[test]
fn approve_funder_is_idempotent() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    s.client.approve_funder(&s.sender, &funder);
    s.client.approve_funder(&s.sender, &funder);
    assert!(s.client.is_approved_funder(&funder));
    assert_eq!(s.client.try_top_up_by_funder(&funder, &1_000), Ok(Ok(())));
}

#[test]
fn non_sender_cannot_revoke_funder() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    s.client.approve_funder(&s.sender, &funder);
    let rando = Address::generate(&s.env);
    let result = s.client.try_revoke_funder(&rando, &funder);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
    assert!(s.client.is_approved_funder(&funder));
}

#[test]
fn top_up_by_funder_rejected_on_cancelled_stream() {
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    s.client.approve_funder(&s.sender, &funder);
    s.client.cancel(&s.sender);
    let result = s.client.try_top_up_by_funder(&funder, &1_000);
    assert_eq!(result, Err(Ok(Error::StreamCancelled)));
    assert_eq!(s.token.balance(&funder), 1_000);
}

#[test]
fn open_top_up_still_accepts_unapproved_third_party() {
    // Regression for #690: the existing open top_up path is not gated by approval.
    let s = Setup::new(100, 3_600, false);
    let funder = mint_to_new_funder(&s, 1_000);
    assert_eq!(s.client.try_top_up(&funder, &1_000), Ok(Ok(())));
}

#[test]
fn rejected_resume_then_pause_retry_emits_events_once_each() {
    let s = Setup::new(100, 3_600, false);
    assert_eq!(s.client.try_resume(&s.sender), Err(Ok(Error::NotPaused)));
    s.client.pause(&s.sender);
    assert!(has_event_named(&s.env, "StreamPaused"));
    s.client.resume(&s.sender);
    assert!(has_event_named(&s.env, "StreamResumed"));
}

// ---- #887: custom TTL thresholds on emergency keeper renewal ----

#[test]
fn emergency_extend_ttl_custom_validates_bounds_and_extends() {
    let s = Setup::new(100, 3_600, false);

    // Zero min or extend rejected
    assert_eq!(
        s.client.try_emergency_extend_ttl_custom(&0, &100_000),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        s.client.try_emergency_extend_ttl_custom(&10_000, &0),
        Err(Ok(Error::InvalidAmount))
    );
    // min > extend rejected
    assert_eq!(
        s.client.try_emergency_extend_ttl_custom(&200_000, &100_000),
        Err(Ok(Error::InvalidAmount))
    );
    // extend > MAX_TTL_LEDGERS rejected
    assert_eq!(
        s.client
            .try_emergency_extend_ttl_custom(&50_000, &(crate::ttl::MAX_TTL_LEDGERS + 1)),
        Err(Ok(Error::InvalidAmount))
    );

    // Valid custom bounds (threshold above current 200_000 TTL) extend instance TTL to 350_000
    assert_eq!(
        s.client.try_emergency_extend_ttl_custom(&250_000, &350_000),
        Ok(Ok(()))
    );
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl, 350_000);
}

// ---- #886: time-bounded emergency cancellation delegate ----

#[test]
fn cancel_delegate_allows_cancellation_within_window_only() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    let stranger = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    // Non-sender cannot configure delegate
    assert_eq!(
        s.client
            .try_set_cancel_delegate(&stranger, &delegate, &(now + 500)),
        Err(Ok(Error::NotAuthorized))
    );
    // Expired timestamp rejected
    assert_eq!(
        s.client.try_set_cancel_delegate(&s.sender, &delegate, &now),
        Err(Ok(Error::InvalidTimeRange))
    );

    // Sender configures delegate valid until now + 500
    s.client
        .set_cancel_delegate(&s.sender, &delegate, &(now + 500));
    assert!(s.client.can_cancel_as_delegate(&delegate));
    assert!(!s.client.can_cancel_as_delegate(&stranger));

    // Cancel delegate CANNOT invoke other privileged sender actions (e.g. pause)
    assert_eq!(
        s.client.try_pause(&delegate),
        Err(Ok(Error::NotAuthorized))
    );

    // Delegate CAN cancel while within window
    s.client.cancel(&delegate);
    assert!(s.client.info().is_cancelled());
}

#[test]
fn expired_or_revoked_cancel_delegate_cannot_cancel() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    let now = s.env.ledger().timestamp();

    s.client
        .set_cancel_delegate(&s.sender, &delegate, &(now + 100));
    assert!(s.client.can_cancel_as_delegate(&delegate));

    // Advance past expiration -> delegate is no longer authorized
    s.advance_secs(101);
    assert!(!s.client.can_cancel_as_delegate(&delegate));
    assert_eq!(
        s.client.try_cancel(&delegate),
        Err(Ok(Error::NotAuthorized))
    );

    // Re-grant and then explicitly revoke -> also rejected
    let now2 = s.env.ledger().timestamp();
    s.client
        .set_cancel_delegate(&s.sender, &delegate, &(now2 + 500));
    assert!(s.client.can_cancel_as_delegate(&delegate));
    s.client.revoke_cancel_delegate(&s.sender);
    assert!(!s.client.can_cancel_as_delegate(&delegate));
    assert_eq!(
        s.client.try_cancel(&delegate),
        Err(Ok(Error::NotAuthorized))
    );
}

// ── Issue #881: StreamTopUpExtended event ───────────────────────────────────

fn stream_events(
    s: &Setup,
) -> std::vec::Vec<(soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val)> {
    s.env
        .events()
        .all()
        .iter()
        .filter(|(contract, _, _)| contract == &s.client.address)
        .map(|(_, topics, data)| (topics, data))
        .collect()
}

fn has_topic(s: &Setup, name: &str) -> bool {
    let wanted = Symbol::new(&s.env, name);
    stream_events(s).iter().any(|(topics, _)| {
        topics
            .get(0)
            .and_then(|t| Symbol::try_from_val(&s.env, &t).ok())
            .is_some_and(|t| t == wanted)
    })
}

#[test]
fn top_up_and_extend_emits_stream_top_up_extended() {
    let s = Setup::new(100, 3_600, false);
    let before_end = s.client.info().end_time;
    let deposited_before = s.client.cumulative_deposited();
    token::StellarAssetClient::new(&s.env, &s.token.address).mint(&s.sender, &20_000);

    s.client.top_up_and_extend(&s.sender, &20_000, &200);

    let (topics, data) = stream_events(&s).pop().unwrap();
    assert_eq!(
        topics,
        (
            Symbol::new(&s.env, "StreamTopUpExtended"),
            s.sender.clone(),
            3_u64
        )
            .into_val(&s.env)
    );
    let payload: (u64, u64, i128, i128) = data.try_into_val(&s.env).unwrap();
    assert_eq!(
        payload,
        (
            before_end,
            before_end + 200,
            20_000,
            deposited_before + 20_000
        )
    );
}

#[test]
fn plain_top_up_does_not_emit_stream_top_up_extended() {
    let s = Setup::new(100, 3_600, false);
    token::StellarAssetClient::new(&s.env, &s.token.address).mint(&s.sender, &5_000);

    s.client.top_up(&s.sender, &5_000);

    assert!(!has_topic(&s, "StreamTopUpExtended"));
}

// ── Issue #880: long-term stream TTL heartbeat ──────────────────────────────

const YEAR_SECS: u64 = 31_536_000;

fn instance_ttl(s: &Setup) -> u32 {
    s.env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl())
}

#[test]
fn heartbeat_extends_ttl_of_multi_year_stream() {
    let s = Setup::new(1, 4 * YEAR_SECS, false);
    s.env.ledger().with_mut(|l| l.sequence_number += 1_000_000);
    let before = instance_ttl(&s);
    assert!(before < 6_000_000);

    s.client.bump_long_term_stream_ttl(&6_000_000, &6_000_000);

    let after = instance_ttl(&s);
    assert!(after > before);
    assert!(after >= 6_000_000);
}

#[test]
fn heartbeat_rejects_short_streams() {
    let s = Setup::new(10, 3_600, false);
    assert_eq!(
        s.client.try_bump_long_term_stream_ttl(&1_000, &100_000),
        Err(Ok(Error::StreamNotLongTerm))
    );
}

#[test]
fn heartbeat_rejects_ended_streams() {
    let s = Setup::new(1, 2 * YEAR_SECS, false);
    s.advance_secs(2 * YEAR_SECS);
    assert_eq!(
        s.client.try_bump_long_term_stream_ttl(&1_000, &100_000),
        Err(Ok(Error::StreamNotLongTerm))
    );
}

#[test]
fn heartbeat_rejects_invalid_bounds() {
    let s = Setup::new(1, 2 * YEAR_SECS, false);
    for (threshold, bump) in [(0, 100), (100, 0), (200, 100), (1, 6_312_001)] {
        assert_eq!(
            s.client.try_bump_long_term_stream_ttl(&threshold, &bump),
            Err(Ok(Error::InvalidAmount))
        );
    }
}

// ── Issue #878: delegated claims ────────────────────────────────────────────

#[test]
fn delegate_claims_for_recipient_without_taking_custody() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    s.client.set_claim_delegate(&delegate);
    assert_eq!(s.client.claim_delegate(), Some(delegate.clone()));

    s.advance_secs(100);
    let claimed = s.client.claim_for(&delegate, &s.recipient, &500);

    assert_eq!(claimed, 500);
    assert_eq!(s.env.auths()[0].0, delegate);
    assert_eq!(s.token.balance(&s.recipient), 500);
    assert_eq!(s.token.balance(&delegate), 0);
    assert_eq!(s.client.info().withdrawn, 500);
    assert!(has_topic(&s, "claim_for"));
}

#[test]
fn claim_for_requires_a_grant_from_the_recipient() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    s.advance_secs(100);

    assert_eq!(
        s.client.try_claim_for(&delegate, &s.recipient, &100),
        Err(Ok(Error::NotAuthorized))
    );
}

#[test]
fn claim_for_rejects_a_recipient_that_does_not_match() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    let other = Address::generate(&s.env);
    s.client.set_claim_delegate(&delegate);
    s.advance_secs(100);

    assert_eq!(
        s.client.try_claim_for(&delegate, &other, &100),
        Err(Ok(Error::NotAuthorized))
    );
}

#[test]
fn revoked_delegate_cannot_claim() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    s.client.set_claim_delegate(&delegate);
    s.client.revoke_claim_delegate();
    s.advance_secs(100);

    assert_eq!(s.client.claim_delegate(), None);
    assert_eq!(
        s.client.try_claim_for(&delegate, &s.recipient, &100),
        Err(Ok(Error::NotAuthorized))
    );
}

#[test]
fn claim_grant_does_not_carry_over_to_a_new_recipient() {
    let s = Setup::new(10, 3_600, false);
    let delegate = Address::generate(&s.env);
    let new_recipient = Address::generate(&s.env);
    s.client.set_claim_delegate(&delegate);
    s.client.transfer_recipient(&new_recipient);
    s.advance_secs(100);

    assert_eq!(s.client.claim_delegate(), None);
    assert_eq!(
        s.client.try_claim_for(&delegate, &new_recipient, &100),
        Err(Ok(Error::NotAuthorized))
    );
}

#[test]
fn recipient_cannot_be_its_own_claim_delegate() {
    let s = Setup::new(10, 3_600, false);
    assert_eq!(
        s.client.try_set_claim_delegate(&s.recipient),
        Err(Ok(Error::InvalidRecipient))
    );

// ---------------------------------------------------------------------------
// Compact StreamFlags bitfield (#882)
// ---------------------------------------------------------------------------

#[test]
fn stream_flags_bitfield_getters_setters_and_persistent_roundtrip() {
    use crate::storage::{get_status_flags, set_status_flags, StreamFlags};

    let mut flags = StreamFlags::new();
    assert_eq!(flags.bits(), 0);
    assert!(!flags.is_paused());
    assert!(!flags.is_cliff_claimed());
    assert!(!flags.is_cancelled());
    assert!(!flags.is_depleted());

    flags.set_paused(true);
    flags.set_cliff_claimed(true);
    assert!(flags.is_paused());
    assert!(flags.is_cliff_claimed());
    assert!(!flags.is_cancelled());
    assert!(!flags.is_depleted());

    flags.set_cancelled(true);
    flags.set_depleted(true);
    flags.set_paused(false);
    assert!(!flags.is_paused());
    assert!(flags.is_cliff_claimed());
    assert!(flags.is_cancelled());
    assert!(flags.is_depleted());
    assert_eq!(
        flags.bits(),
        StreamFlags::CLIFF_CLAIMED | StreamFlags::CANCELLED | StreamFlags::DEPLETED
    );

    let s = Setup::new(100, 3600, false);
    s.env.as_contract(&s.client.address, || {
        assert_eq!(get_status_flags(&s.env), StreamFlags::new());
        set_status_flags(&s.env, flags);
        assert_eq!(get_status_flags(&s.env), flags);
    });
}

// ---------------------------------------------------------------------------
// Non-linear vesting curves (#883)
// ---------------------------------------------------------------------------

#[test]
fn vesting_curve_linear_exponential_decay_and_stepped_invariants() {
    use crate::state::{get_vesting_curve, set_vesting_curve, vested_amount_for_curve, VestingCurve};

    let total: i128 = 1_000_000;
    let duration: u64 = 1_000;

    let lin_mid = vested_amount_for_curve(&VestingCurve::Linear, total, 500, duration).unwrap();
    assert_eq!(lin_mid, 500_000);

    // Exponential decay front-loads more value by the midpoint than linear vesting.
    let exp_curve = VestingCurve::ExponentialDecay(5_000);
    let exp_start = vested_amount_for_curve(&exp_curve, total, 0, duration).unwrap();
    let exp_mid = vested_amount_for_curve(&exp_curve, total, 500, duration).unwrap();
    let exp_end = vested_amount_for_curve(&exp_curve, total, duration, duration).unwrap();
    assert_eq!(exp_start, 0);
    assert!(exp_mid > lin_mid && exp_mid < total);
    assert_eq!(exp_end, total);

    // Stepped vesting unlocks in discrete 250-second chunks.
    let stepped = VestingCurve::Stepped(250);
    assert_eq!(vested_amount_for_curve(&stepped, total, 249, duration).unwrap(), 0);
    assert_eq!(vested_amount_for_curve(&stepped, total, 250, duration).unwrap(), 250_000);
    assert_eq!(vested_amount_for_curve(&stepped, total, 749, duration).unwrap(), 500_000);
    assert_eq!(vested_amount_for_curve(&stepped, total, 750, duration).unwrap(), 750_000);
    assert_eq!(vested_amount_for_curve(&stepped, total, duration, duration).unwrap(), total);

    // Storage roundtrip defaults to Linear and persists configured curve.
    let s = Setup::new(100, 3600, false);
    s.env.as_contract(&s.client.address, || {
        assert_eq!(get_vesting_curve(&s.env), VestingCurve::Linear);
        set_vesting_curve(&s.env, &exp_curve);
        assert_eq!(get_vesting_curve(&s.env), exp_curve);
    });
}

// ---------------------------------------------------------------------------
// Distinct InvalidCliffDuration error (#884)
// ---------------------------------------------------------------------------

#[test]
fn set_cliff_returns_invalid_cliff_duration_outside_stream_bounds() {
    let s = Setup::new(100, 1_000, false);
    let info = s.client.info();

    // Cliff before start_time -> InvalidCliffDuration
    assert_eq!(
        s.client.try_set_cliff(&s.sender, &(info.start_time - 1), &100),
        Err(Ok(Error::InvalidCliffDuration))
    );

    // Cliff after end_time -> InvalidCliffDuration
    assert_eq!(
        s.client.try_set_cliff(&s.sender, &(info.end_time + 1), &100),
        Err(Ok(Error::InvalidCliffDuration))
    );

    // Cliff within [start_time, end_time] succeeds
    assert!(s
        .client
        .try_set_cliff(&s.sender, &(info.start_time + 100), &100)
        .is_ok());
}

// ---------------------------------------------------------------------------
// Fuzz & property-based extreme duration edge cases (#885)
// ---------------------------------------------------------------------------

#[test]
fn fuzz_extreme_stream_durations_and_arbitrary_tuples() {
    use crate::state::{vested_amount_for_curve, VestingCurve};
    use crate::storage::StreamInfo;

    const ONE_SECOND: u64 = 1;
    const HUNDRED_YEARS: u64 = 100 * 365 * 86_400;

    let env = Env::default();
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let token = Address::generate(&env);

    // 1. Explicit boundary duration cases: 1s, 100 years, and near u64::MAX
    let boundary_cases: &[(u64, u64, i128)] = &[
        (0, ONE_SECOND, 1),
        (1_000, 1_000 + ONE_SECOND, 1_000_000_000_000),
        (1_700_000_000, 1_700_000_000 + HUNDRED_YEARS, 10_000_000_000_000_000_000),
        (u64::MAX - 10, u64::MAX, 500_000_000),
        (u64::MAX - HUNDRED_YEARS, u64::MAX, i128::MAX / 4),
    ];

    let curves = [
        VestingCurve::Linear,
        VestingCurve::ExponentialDecay(2_500),
        VestingCurve::ExponentialDecay(10_000),
        VestingCurve::Stepped(1),
        VestingCurve::Stepped(86_400),
    ];

    for &(start, stop, deposit) in boundary_cases {
        let duration = stop.saturating_sub(start);
        let info = StreamInfo {
            sender: sender.clone(),
            recipient: recipient.clone(),
            token: token.clone(),
            rate_per_second: 1,
            start_time: start,
            end_time: stop,
            withdrawn: 0,
            paused_at: 0,
            flags: 0,
            event_sequence: 0,
            cumulative_fees: 0,
        };

        assert_eq!(info.elapsed_seconds(start), 0);
        assert_eq!(info.elapsed_seconds(stop), duration);
        assert_eq!(info.elapsed_seconds(u64::MAX), duration);

        for curve in &curves {
            let v0 = vested_amount_for_curve(curve, deposit, 0, duration).unwrap();
            let v_mid = vested_amount_for_curve(curve, deposit, duration / 2, duration).unwrap();
            let v_end = vested_amount_for_curve(curve, deposit, duration, duration).unwrap();
            let v_over = vested_amount_for_curve(curve, deposit, u64::MAX, duration).unwrap();
            assert_eq!(v0, 0);
            assert!(v_mid >= 0 && v_mid <= deposit);
            assert_eq!(v_end, deposit);
            assert_eq!(v_over, deposit);
        }
    }

    // 2. Deterministic PRNG fuzz over 500 arbitrary (start, stop, cliff, deposit) tuples
    let mut seed: u64 = 0x5EED_CAFE_BABE_1337;
    let mut next_u64 = || -> u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for _ in 0..500 {
        let start = next_u64() % (u64::MAX - HUNDRED_YEARS - 1);
        let duration = match next_u64() % 4 {
            0 => ONE_SECOND,
            1 => 1 + (next_u64() % 86_400),
            2 => 1 + (next_u64() % HUNDRED_YEARS),
            _ => (u64::MAX - start).max(1),
        };
        let stop = start.saturating_add(duration);
        let effective_duration = stop.saturating_sub(start);
        let cliff = start.saturating_add(next_u64() % effective_duration.saturating_add(1));
        assert!(cliff >= start && cliff <= stop);

        let deposit = ((next_u64() as i128) % 10_000_000_000_000).max(1);
        let t1 = next_u64() % effective_duration.saturating_add(1);
        let t2 = t1.saturating_add(next_u64() % (effective_duration - t1).saturating_add(1));

        for curve in &curves {
            let v1 = vested_amount_for_curve(curve, deposit, t1, effective_duration).unwrap();
            let v2 = vested_amount_for_curve(curve, deposit, t2, effective_duration).unwrap();
            assert!(v1 >= 0 && v1 <= deposit, "v1 out of bounds");
            assert!(v2 >= 0 && v2 <= deposit, "v2 out of bounds");
            assert!(v1 <= v2, "monotonicity violated for {curve:?}: v({t1})={v1} > v({t2})={v2}");
        }
    }
}
