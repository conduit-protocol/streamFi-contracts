#![cfg(test)]

// The crate is `#![no_std]`, but this module only compiles under `cargo test`,
// where `std` is available as a linked dependency of the test harness anyway.
extern crate std;

use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Events as _},
    token, Address, Env, IntoVal, Symbol, TryFromVal, TryIntoVal, Vec,
};

use crate::{BatchTransferProcessorClient, Error};

/// Mirrors `MAX_BATCH_SIZE` — the value `max_batch_size()` must expose and
/// the cap `process_batch` enforces.
const MAX_BATCH_SIZE: u32 = 100;

struct Setup {
    env: Env,
    client: BatchTransferProcessorClient<'static>,
    token: token::Client<'static>,
    token_addr: Address,
    funder: Address,
}

impl Setup {
    fn new() -> Self {
        Self::with_funder_balance(1_000_000)
    }

    fn with_funder_balance(balance: i128) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let funder = Address::generate(&env);
        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let tok_admin = token::StellarAssetClient::new(&env, &token_addr);
        tok_admin.mint(&funder, &balance);

        let contract_id = env.register_contract(None, super::BatchTransferProcessor);

        // Leak env for 'static lifetime convenience in tests
        let env: &'static Env = std::boxed::Box::leak(std::boxed::Box::new(env));
        Setup {
            env: env.clone(),
            client: BatchTransferProcessorClient::new(env, &contract_id),
            token: token::Client::new(env, &token_addr),
            token_addr,
            funder,
        }
    }

    fn recipients(&self, n: u32) -> Vec<Address> {
        let mut v = Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(Address::generate(&self.env));
        }
        v
    }

    fn amounts(&self, amounts: &[i128]) -> Vec<i128> {
        let mut v = Vec::new(&self.env);
        for a in amounts {
            v.push_back(*a);
        }
        v
    }

    /// `n` copies of `value` — builds boundary-size batches without the std
    /// `vec!` macro (the crate is `#![no_std]`).
    fn filled(&self, value: i128, n: u32) -> Vec<i128> {
        let mut v = Vec::new(&self.env);
        for _ in 0..n {
            v.push_back(value);
        }
        v
    }

    /// All events published by the batch-processor contract, oldest first.
    /// `env.events().all()` also captures the token's own transfer events, so
    /// filter down to this contract's address.
    fn processor_events(
        &self,
    ) -> std::vec::Vec<(
        Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )> {
        self.env
            .events()
            .all()
            .iter()
            .filter(|(contract, _, _)| contract == &self.client.address)
            .map(|(c, t, d)| (c.clone(), t.clone(), d))
            .collect()
    }
}

/// A non-zero Stellar *account* address (`G...` strkey) — the shape a real
/// funder wallet has in production, and what ends up in the `token` slot when
/// `process_batch`'s `funder` and `token` arguments are transposed.
///
/// `Address::generate` produces a contract-shaped address instead, so it
/// cannot stand in for a wallet here.
fn account_address(env: &Env) -> Address {
    use soroban_sdk::xdr::{AccountId, PublicKey, ScAddress, Uint256};
    Address::try_from_val(
        env,
        &ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
            [0x11u8; 32],
        )))),
    )
    .unwrap()
}

// ── Happy path ─────────────────────────────────────────────────────────────

#[test]
fn process_batch_transfers_each_amount_and_returns_total() {
    let s = Setup::new();
    let recipients = s.recipients(3);
    let amounts = s.amounts(&[100, 200, 300]);

    let total = s
        .client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(total, 600);
    assert_eq!(s.token.balance(&recipients.get(0).unwrap()), 100);
    assert_eq!(s.token.balance(&recipients.get(1).unwrap()), 200);
    assert_eq!(s.token.balance(&recipients.get(2).unwrap()), 300);

    // Funds pass through the contract in one lump sum and are fully fanned
    // out — nothing is stranded on the processor.
    assert_eq!(s.token.balance(&s.client.address), 0);
    assert_eq!(s.token.balance(&s.funder), 1_000_000 - 600);
}

#[test]
fn process_batch_accepts_a_batch_of_exactly_max_size() {
    let s = Setup::new();
    let recipients = s.recipients(MAX_BATCH_SIZE);
    let amounts = s.filled(1, MAX_BATCH_SIZE);

    let total = s
        .client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    // Exactly MAX_BATCH_SIZE entries are accepted; one more is rejected
    // (covered by the BatchTooLarge test below).
    assert_eq!(total, MAX_BATCH_SIZE as i128);
    assert_eq!(
        s.token
            .balance(&recipients.get(MAX_BATCH_SIZE - 1).unwrap()),
        1
    );
}

#[test]
fn max_batch_size_reports_the_enforced_cap() {
    let s = Setup::new();
    assert_eq!(s.client.max_batch_size(), MAX_BATCH_SIZE);
}

// ── Behaviour version (issue #565) ─────────────────────────────────────────

#[test]
fn version_reports_the_current_behaviour_version() {
    let s = Setup::new();
    // v2: SEP-41 token probe before auth, instance-TTL keep-alive, and
    // `preview_batch` added. Bumped whenever observable behaviour changes.
    assert_eq!(s.client.version(), 2);
}

// ── Validation order (issue #562) ──────────────────────────────────────────

#[test]
fn documented_validation_order_reports_only_the_first_failing_check() {
    let s = Setup::new();

    // 1 beats 2: an oversized batch whose vectors disagree on length fails
    // the length check first.
    let over_recipients = s.recipients(MAX_BATCH_SIZE + 1);
    let short_amounts = s.filled(1, MAX_BATCH_SIZE);
    assert_eq!(
        s.client
            .try_process_batch(&s.funder, &s.token_addr, &over_recipients, &short_amounts),
        Err(Ok(Error::LengthMismatch)),
    );

    // 2 beats 3: an oversized batch containing a zero amount reports the
    // size failure, never the amount failure.
    let recipients = s.recipients(MAX_BATCH_SIZE + 1);
    let mut oversized_with_zero = s.filled(1, MAX_BATCH_SIZE);
    oversized_with_zero.push_back(0);
    assert_eq!(
        s.client
            .try_process_batch(&s.funder, &s.token_addr, &recipients, &oversized_with_zero),
        Err(Ok(Error::BatchTooLarge)),
    );

    // 3 beats 4: the first non-positive amount is reported before the
    // accumulation loop can overflow.
    let recipients = s.recipients(3);
    let amounts = s.amounts(&[0, i128::MAX, i128::MAX]);
    assert_eq!(
        s.client
            .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );

    // 4 beats 5: an overflowing total is reported even when the token
    // address is also invalid.
    let zero_token = Address::from_string(&soroban_sdk::String::from_str(
        &s.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    let recipients = s.recipients(2);
    let amounts = s.amounts(&[i128::MAX, i128::MAX]);
    assert_eq!(
        s.client
            .try_process_batch(&s.funder, &zero_token, &recipients, &amounts),
        Err(Ok(Error::ArithmeticOverflow)),
    );
}

// ── Token precondition (issue #564) ────────────────────────────────────────

#[test]
fn zero_address_token_is_rejected_before_auth_or_transfers() {
    let s = Setup::new();
    let zero_token = Address::from_string(&soroban_sdk::String::from_str(
        &s.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[100]);

    let result = s
        .client
        .try_process_batch(&s.funder, &zero_token, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidToken)));
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.processor_events().len(), 0);
}

/// The token check is no longer just the zero-address guard: a SEP-41
/// `balance` probe runs as check 5, before `funder.require_auth()`. An
/// address that is a real contract but does not implement SEP-41 is rejected
/// with `Error::InvalidToken` instead of blowing up at the first
/// `tk.transfer` with an opaque host-level error.
#[test]
fn non_sep41_contract_is_rejected_by_the_balance_probe_before_auth() {
    let s = Setup::new();
    // A real deployed contract that does not implement SEP-41 `transfer`/`balance`.
    let not_a_token = s.env.register_contract(None, super::BatchTransferProcessor);
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[100]);

    let result = s
        .client
        .try_process_batch(&s.funder, &not_a_token, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidToken)));
    // Rejected during validation — the funder's balance is untouched, no
    // event was published, and no auth was ever requested.
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.processor_events().len(), 0);
}

// ── Argument order (issue #560) ─────────────────────────────────────────────

/// `funder` and `token` are adjacent bare `Address` parameters, so a
/// transposed call compiles. The realistic mistake — a wallet in the `token`
/// slot — must fail with a clear contract error, `InvalidToken`, *before*
/// `require_auth` runs against the token contract and before anything moves.
#[test]
fn transposed_funder_and_token_arguments_fail_with_invalid_token_before_auth() {
    let s = Setup::new();
    // A real `G...` account: what a funder's wallet actually is on-chain.
    let wallet = account_address(&s.env);
    let recipients = s.recipients(2);
    let amounts = s.amounts(&[100, 200]);

    // Wrong order: the token contract is passed as `funder`, the funder's
    // wallet as `token`.
    let result = s
        .client
        .try_process_batch(&s.token_addr, &wallet, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidToken)));
    // Nothing moved, nothing emitted, and no auth was requested (mock_all_auths
    // records every auth that *is* required — there are none).
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.token.balance(&s.client.address), 0);
    assert_eq!(s.processor_events().len(), 0);
    assert!(s.env.auths().is_empty());
}

/// The account guard that makes the test above possible: `G...` addresses are
/// wallets (rejected outright as `InvalidToken`), `C...` addresses are
/// contracts handed to the SEP-41 probe.
#[test]
fn is_stellar_account_distinguishes_wallets_from_contracts() {
    let s = Setup::new();

    assert!(super::is_stellar_account(&account_address(&s.env)));
    assert!(!super::is_stellar_account(&s.token_addr));
    // `Address::generate` yields a contract-shaped address, not a wallet.
    assert!(!super::is_stellar_account(&Address::generate(&s.env)));
}

// ── Input validation errors ────────────────────────────────────────────────

#[test]
fn length_mismatch_is_rejected() {
    let s = Setup::new();
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[100, 200]);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::LengthMismatch)));
    // Validation runs before any token movement.
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
}

#[test]
fn batch_larger_than_max_is_rejected() {
    let s = Setup::new();
    let recipients = s.recipients(MAX_BATCH_SIZE + 1);
    let amounts = s.filled(1, MAX_BATCH_SIZE + 1);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::BatchTooLarge)));
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
}

#[test]
fn zero_amount_is_rejected_as_invalid_amount() {
    let s = Setup::new();
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[0]);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
}

#[test]
fn negative_amount_is_rejected_as_invalid_amount() {
    let s = Setup::new();
    let recipients = s.recipients(2);
    let amounts = s.amounts(&[100, -1]);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
}

// ── Empty-batch short-circuit ──────────────────────────────────────────────

#[test]
fn empty_batch_short_circuits_with_zero() {
    let s = Setup::new();
    let recipients: Vec<Address> = Vec::new(&s.env);
    let amounts: Vec<i128> = Vec::new(&s.env);

    // Nothing to accumulate, so the loop leaves `total == 0` and the
    // function returns before auth or any token movement.
    let total = s
        .client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(total, 0);
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.token.balance(&s.client.address), 0);
    // No transfers happened, so no `batch_transferred` event is published.
    assert_eq!(s.processor_events().len(), 0);
}

// ── Overflow in the accumulation loop ──────────────────────────────────────

#[test]
fn accumulation_overflow_is_rejected_before_auth_or_transfers() {
    let s = Setup::new();
    let recipients = s.recipients(2);
    // Each amount is individually valid (> 0) but their sum overflows i128,
    // exercising `total.checked_add(amount)` returning `None`.
    let amounts = s.amounts(&[i128::MAX, i128::MAX]);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::ArithmeticOverflow)));
    // Rejected during validation — before require_auth and any transfer.
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.token.balance(&s.client.address), 0);
    assert_eq!(s.processor_events().len(), 0);
}

// ── Event emission (issue #553) ────────────────────────────────────────────

#[test]
fn process_batch_emits_batch_transferred_with_right_total() {
    let s = Setup::new();
    let recipients = s.recipients(3);
    let amounts = s.amounts(&[100, 200, 300]);

    s.client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    let events = s.processor_events();
    assert_eq!(events.len(), 1);

    let (_, topics, data) = &events[0];
    // Topics: ("batch_transferred", funder) — the event symbol is topic 0
    // (workspace convention), funder is the address topic.
    assert_eq!(
        topics.clone(),
        (Symbol::new(&s.env, "batch_transferred"), s.funder.clone()).into_val(&s.env)
    );
    // Data: { token, recipient_count, total }
    let payload: (Address, u32, i128) = data.clone().try_into_val(&s.env).unwrap();
    assert_eq!(payload, (s.token_addr.clone(), 3, 600));
}

#[test]
fn failed_process_batch_emits_no_event() {
    let s = Setup::new();
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[0]);

    let result = s
        .client
        .try_process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    assert_eq!(s.processor_events().len(), 0);
}

// ── preview_batch (issue #561) ─────────────────────────────────────────────

#[test]
fn preview_batch_returns_the_total_without_auth_or_transfers() {
    let s = Setup::new();
    let recipients = s.recipients(3);
    let amounts = s.amounts(&[100, 200, 300]);

    // Read-only: no funder, no token, no auth, no funds moved, no event.
    let total = s.client.preview_batch(&recipients, &amounts);

    assert_eq!(total, 600);
    assert_eq!(s.token.balance(&s.funder), 1_000_000);
    assert_eq!(s.token.balance(&s.client.address), 0);
    assert_eq!(s.processor_events().len(), 0);
    assert!(s.env.auths().is_empty());
}

#[test]
fn preview_batch_agrees_with_process_batch_on_the_same_inputs() {
    let s = Setup::new();
    let recipients = s.recipients(4);
    let amounts = s.amounts(&[7, 13, 21, 35]);

    // The preview must be the exact figure process_batch charges — a client
    // that shows it to a user before signing can never be off by a unit.
    let previewed = s.client.preview_batch(&recipients, &amounts);
    let charged = s
        .client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    assert_eq!(previewed, 76);
    assert_eq!(charged, previewed);
}

#[test]
fn preview_batch_applies_the_same_validation_in_the_same_order() {
    let s = Setup::new();

    // 1 — length mismatch
    assert_eq!(
        s.client
            .try_preview_batch(&s.recipients(2), &s.filled(1, 1)),
        Err(Ok(Error::LengthMismatch)),
    );

    // 2 — batch size cap (beats a zero amount in the same batch)
    let mut oversized = s.filled(1, MAX_BATCH_SIZE);
    oversized.push_back(0);
    assert_eq!(
        s.client
            .try_preview_batch(&s.recipients(MAX_BATCH_SIZE + 1), &oversized),
        Err(Ok(Error::BatchTooLarge)),
    );

    // 3 — non-positive amount
    assert_eq!(
        s.client
            .try_preview_batch(&s.recipients(1), &s.amounts(&[0])),
        Err(Ok(Error::InvalidAmount)),
    );
    assert_eq!(
        s.client
            .try_preview_batch(&s.recipients(1), &s.amounts(&[-5])),
        Err(Ok(Error::InvalidAmount)),
    );

    // 4 — checked accumulation overflows the same way
    assert_eq!(
        s.client
            .try_preview_batch(&s.recipients(2), &s.amounts(&[i128::MAX, i128::MAX]),),
        Err(Ok(Error::ArithmeticOverflow)),
    );
}

#[test]
fn preview_batch_accepts_an_empty_batch_as_zero() {
    let s = Setup::new();
    let recipients: Vec<Address> = Vec::new(&s.env);
    let amounts: Vec<i128> = Vec::new(&s.env);

    assert_eq!(s.client.preview_batch(&recipients, &amounts), 0);
    assert_eq!(s.processor_events().len(), 0);
}

// ── Instance TTL keep-alive (issue #559) ───────────────────────────────────

#[test]
fn process_batch_extends_the_instance_ttl() {
    let s = Setup::new();
    let recipients = s.recipients(1);
    let amounts = s.amounts(&[100]);

    s.client
        .process_batch(&s.funder, &s.token_addr, &recipients, &amounts);

    // Same threshold/extend-to pair the other three contracts apply on every
    // state-mutating call (`drip_common::TTL_EXTEND_TO`).
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}

#[test]
fn read_only_and_validation_failure_paths_do_not_extend_the_instance_ttl() {
    let s = Setup::new();

    // `preview_batch` is read-only — it must leave the instance TTL alone.
    s.client.preview_batch(&s.recipients(1), &s.amounts(&[1]));
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_ne!(ttl, 200_000);

    // A rejected batch fails early, before the keep-alive runs.
    let result =
        s.client
            .try_process_batch(&s.funder, &s.token_addr, &s.recipients(1), &s.amounts(&[0]));
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_ne!(ttl, 200_000);

    // The empty-batch no-op returns before auth and before the keep-alive.
    let empty: Vec<Address> = Vec::new(&s.env);
    let no_amounts: Vec<i128> = Vec::new(&s.env);
    assert_eq!(
        s.client
            .process_batch(&s.funder, &s.token_addr, &empty, &no_amounts),
        0
    );
    let ttl = s
        .env
        .as_contract(&s.client.address, || s.env.storage().instance().get_ttl());
    assert_ne!(ttl, 200_000);
}
