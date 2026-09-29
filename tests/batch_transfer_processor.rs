//! Integration tests for
//! `drip_batch_processor::BatchTransferProcessor::process_batch`.
//!
//! The processor pulls `sum(amounts)` from a single `funder` (one inbound
//! transfer, one auth) and fans the funds out to `recipients` in order. All
//! validation runs before any token movement.

use drip_batch_processor::{BatchTransferProcessor, BatchTransferProcessorClient, Error};
use soroban_sdk::{testutils::Address as _, token, Address, Env, Vec};

struct Fixture<'a> {
    env: Env,
    client: BatchTransferProcessorClient<'a>,
    token: token::Client<'a>,
    token_admin_client: token::StellarAssetClient<'a>,
    funder: Address,
}

fn setup<'a>() -> Fixture<'a> {
    let env = Env::default();
    env.mock_all_auths();

    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let id = env.register_contract(None, BatchTransferProcessor);
    let client = BatchTransferProcessorClient::new(&env, &id);

    let funder = Address::generate(&env);

    Fixture {
        token: token::Client::new(&env, &token_addr),
        token_admin_client: token::StellarAssetClient::new(&env, &token_addr),
        client,
        funder,
        env,
    }
}

fn addrs(env: &Env, n: u32) -> Vec<Address> {
    let mut v = Vec::new(env);
    for _ in 0..n {
        v.push_back(Address::generate(env));
    }
    v
}

#[test]
fn process_batch_fans_out_to_every_recipient() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 20, 30]);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 60);
    assert_eq!(f.token.balance(&recipients.get(0).unwrap()), 10);
    assert_eq!(f.token.balance(&recipients.get(1).unwrap()), 20);
    assert_eq!(f.token.balance(&recipients.get(2).unwrap()), 30);
    assert_eq!(f.token.balance(&f.funder), 40);
}

#[test]
fn process_batch_empty_input_is_a_noop() {
    let f = setup();
    let recipients: Vec<Address> = Vec::new(&f.env);
    let amounts: Vec<i128> = Vec::new(&f.env);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 0);
    assert_eq!(f.token.balance(&f.funder), 0);
}

#[test]
fn process_batch_accepts_exactly_100_entries() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 100);
    let mut amounts = Vec::new(&f.env);
    for _ in 0..100 {
        amounts.push_back(1i128);
    }

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 100);
    assert_eq!(f.token.balance(&f.funder), 0);
}

#[test]
fn process_batch_rejects_101_entries() {
    let f = setup();
    let recipients = addrs(&f.env, 101);
    let mut amounts = Vec::new(&f.env);
    for _ in 0..101 {
        amounts.push_back(1i128);
    }

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::BatchTooLarge)),
    );
}

#[test]
fn max_batch_size_matches_the_enforced_boundary() {
    let f = setup();

    // The advertised cap must be the constant the processor actually enforces,
    // otherwise a client that trusts it would still get BatchTooLarge.
    let cap = f.client.max_batch_size();
    assert_eq!(cap, 100);

    // Exactly `cap` entries are accepted...
    f.token_admin_client.mint(&f.funder, &(cap as i128));
    let recipients = addrs(&f.env, cap);
    let mut amounts = Vec::new(&f.env);
    for _ in 0..cap {
        amounts.push_back(1i128);
    }
    assert_eq!(
        f.client
            .process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        cap as i128,
    );

    // ...and one more is rejected, so the read-only value is the true boundary.
    let over_recipients = addrs(&f.env, cap + 1);
    let mut over_amounts = Vec::new(&f.env);
    for _ in 0..=cap {
        over_amounts.push_back(1i128);
    }
    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &over_recipients, &over_amounts),
        Err(Ok(Error::BatchTooLarge)),
    );
}

#[test]
fn version_reports_the_current_behaviour_version() {
    let f = setup();
    // v2: SEP-41 token probe before auth, instance-TTL keep-alive, and
    // `preview_batch`. Bumped whenever observable behaviour changes.
    assert_eq!(f.client.version(), 2);
}

#[test]
fn process_batch_rejects_zero_address_token() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    // The all-zero Stellar account address — the same precondition
    // `DripFactory::create_stream` applies before touching the token.
    let zero_token = Address::from_string(&soroban_sdk::String::from_str(
        &f.env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [10i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &zero_token, &recipients, &amounts),
        Err(Ok(Error::InvalidToken)),
    );
    assert_eq!(f.token.balance(&f.funder), 100);
}

#[test]
fn process_batch_rejects_length_mismatch() {
    let f = setup();
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [1i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::LengthMismatch)),
    );
}

#[test]
fn process_batch_rejects_zero_amount() {
    let f = setup();
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [0i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_rejects_zero_amount_in_mixed_batch() {
    let f = setup();
    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 0, 30]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_rejects_negative_amount() {
    let f = setup();
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [-5i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::InvalidAmount)),
    );
}

#[test]
fn process_batch_detects_total_overflow() {
    let f = setup();
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [i128::MAX, 1]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts),
        Err(Ok(Error::ArithmeticOverflow)),
    );
}

#[test]
fn process_batch_does_not_move_funds_when_validation_fails() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &1_000);

    // Length mismatch — must bail before any transfer.
    let recipients = addrs(&f.env, 2);
    let amounts = Vec::from_array(&f.env, [10i128]);
    let _ = f
        .client
        .try_process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(f.token.balance(&f.funder), 1_000);
}

#[test]
fn process_batch_pays_duplicate_recipients_separately() {
    // Documented behavior (#554): recipients are not deduplicated. The same
    // address listed twice receives one transfer per occurrence — not a
    // merged single transfer, and not a rejection.
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let dup = Address::generate(&f.env);
    let recipients = Vec::from_array(&f.env, [dup.clone(), dup.clone()]);
    let amounts = Vec::from_array(&f.env, [10i128, 20]);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 30);
    // Both entries paid out: 10 + 20, not merged or rejected.
    assert_eq!(f.token.balance(&dup), 30);
    assert_eq!(f.token.balance(&f.funder), 70);
}

#[test]
fn process_batch_accepts_a_single_entry_batch() {
    // Documented behavior (#556): size-1 batches are allowed (no
    // MIN_BATCH_SIZE guard) — callers are advised to use token.transfer
    // directly for single-recipient payouts, but the contract does not
    // reject them.
    let f = setup();
    f.token_admin_client.mint(&f.funder, &50);

    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [50i128]);

    let total = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(total, 50);
    assert_eq!(f.token.balance(&recipients.get(0).unwrap()), 50);
    assert_eq!(f.token.balance(&f.funder), 0);
}

#[test]
fn error_type_carries_required_traits() {
    fn assert_traits<T: Copy + Clone + core::fmt::Debug + Eq + PartialEq + PartialOrd + Ord>() {}
    assert_traits::<Error>();
    assert_eq!(Error::LengthMismatch as u32, 1);
    assert_eq!(Error::BatchTooLarge as u32, 2);
    assert_eq!(Error::InvalidAmount as u32, 3);
    assert_eq!(Error::ArithmeticOverflow as u32, 4);
    assert_eq!(Error::InvalidToken as u32, 5);
}

// ── preview_batch (issue #561) ─────────────────────────────────────────────

#[test]
fn preview_batch_returns_the_total_without_auth_or_moving_funds() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 20, 30]);

    // No funder, no token: the preview cannot pull funds or ask for auth.
    let previewed = f.client.preview_batch(&recipients, &amounts);

    assert_eq!(previewed, 60);
    assert_eq!(f.token.balance(&f.funder), 100);
    assert_eq!(f.token.balance(&f.client.address), 0);
}

#[test]
fn preview_batch_agrees_with_process_batch_on_the_same_inputs() {
    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 3);
    let amounts = Vec::from_array(&f.env, [10i128, 20, 30]);

    // What the user is shown before signing is exactly what they are charged.
    let previewed = f.client.preview_batch(&recipients, &amounts);
    let charged = f
        .client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    assert_eq!(previewed, charged);
    assert_eq!(f.token.balance(&f.funder), 100 - previewed);
}

#[test]
fn preview_batch_applies_the_same_validation_as_process_batch() {
    let f = setup();

    // Length mismatch
    assert_eq!(
        f.client
            .try_preview_batch(&addrs(&f.env, 2), &Vec::from_array(&f.env, [1i128])),
        Err(Ok(Error::LengthMismatch)),
    );

    // Batch cap
    assert_eq!(
        f.client
            .try_preview_batch(&addrs(&f.env, 101), &Vec::from_array(&f.env, [1i128; 101]),),
        Err(Ok(Error::BatchTooLarge)),
    );

    // Non-positive amount
    assert_eq!(
        f.client
            .try_preview_batch(&addrs(&f.env, 1), &Vec::from_array(&f.env, [0i128]),),
        Err(Ok(Error::InvalidAmount)),
    );

    // Checked accumulation
    assert_eq!(
        f.client.try_preview_batch(
            &addrs(&f.env, 2),
            &Vec::from_array(&f.env, [i128::MAX, i128::MAX]),
        ),
        Err(Ok(Error::ArithmeticOverflow)),
    );
}

#[test]
fn preview_batch_accepts_an_empty_batch_as_zero() {
    let f = setup();
    let recipients: Vec<Address> = Vec::new(&f.env);
    let amounts: Vec<i128> = Vec::new(&f.env);

    assert_eq!(f.client.preview_batch(&recipients, &amounts), 0);
}

// ── Argument order (issue #560) ─────────────────────────────────────────────

#[test]
fn transposed_funder_and_token_arguments_fail_with_invalid_token() {
    use soroban_sdk::xdr::{AccountId, PublicKey, ScAddress, Uint256};
    use soroban_sdk::TryFromVal;

    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    // A real `G...` account — what a funder's wallet actually is on-chain —
    // so the transposed case is exercised with an address of the same shape
    // a production caller would produce.
    let wallet = Address::try_from_val(
        &f.env,
        &ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
            [0x11u8; 32],
        )))),
    )
    .unwrap();

    // Wrong order: the token contract in the `funder` slot, the funder's
    // wallet in the `token` slot. Check 5 rejects the wallet, so the call
    // returns InvalidToken before `require_auth` against the token contract
    // and before any transfer.
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [10i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.token.address, &wallet, &recipients, &amounts),
        Err(Ok(Error::InvalidToken)),
    );
    assert_eq!(f.token.balance(&f.funder), 100);
}

#[test]
fn non_sep41_contract_as_token_is_rejected_with_invalid_token() {
    let f = setup();

    // A deployed contract that does not implement SEP-41 — previously this
    // died host-level at the first `transfer`, now the `balance` probe in
    // check 5 rejects it with a contract error before auth.
    let not_a_token = f.env.register_contract(None, BatchTransferProcessor);
    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [10i128]);

    assert_eq!(
        f.client
            .try_process_batch(&f.funder, &not_a_token, &recipients, &amounts),
        Err(Ok(Error::InvalidToken)),
    );
}

// ── Instance TTL keep-alive (issue #559) ───────────────────────────────────

#[test]
fn process_batch_extends_the_instance_ttl() {
    use soroban_sdk::testutils::storage::Instance as _;

    let f = setup();
    f.token_admin_client.mint(&f.funder, &100);

    let recipients = addrs(&f.env, 1);
    let amounts = Vec::from_array(&f.env, [10i128]);
    f.client
        .process_batch(&f.funder, &f.token.address, &recipients, &amounts);

    // Same TTL window the stream, factory, and governor contracts apply on
    // every state-mutating call (drip_common::TTL_EXTEND_TO).
    let ttl = f
        .env
        .as_contract(&f.client.address, || f.env.storage().instance().get_ttl());
    assert_eq!(ttl, 200_000);
}
