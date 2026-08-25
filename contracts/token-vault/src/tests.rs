#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, token, Address, Env};

use crate::errors::Error;
use crate::storage;
use crate::TokenVaultClient;

struct Setup {
    env: Env,
    client: TokenVaultClient<'static>,
    token: token::Client<'static>,
    owner: Address,
    user: Address,
}

impl Setup {
    fn new(max_limit: i128) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let owner = Address::generate(&env);
        let user = Address::generate(&env);

        let token_admin = Address::generate(&env);
        let token_addr = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();

        let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

        // Mint a large supply to the user so deposits succeed
        tok_admin.mint(&user, &(max_limit));

        let vault_id = env.register_contract(None, super::TokenVault);
        let client = TokenVaultClient::new(&env, &vault_id);

        client.initialize(&owner, &token_addr, &max_limit);

        // Leak env for 'static lifetime convenience in tests
        let env: &'static Env = std::boxed::Box::leak(std::boxed::Box::new(env));
        let client = TokenVaultClient::new(env, &vault_id);
        let token = token::Client::new(env, &token_addr);

        Setup {
            env: env.clone(),
            client,
            token,
            owner,
            user,
        }
    }
}

// ── Original deposit / withdraw / set_limit tests ──────────────────────────

#[test]
fn deposit_respects_max_limit() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    s.client.deposit(&s.user, &max_limit);

    let result = s.client.try_deposit(&s.user, &1);
    assert_eq!(result, Err(Ok(Error::LimitExceeded)));
}

#[test]
fn deposit_rejects_amount_that_would_overflow_i128() {
    let s = Setup::new(i128::MAX);

    s.client.deposit(&s.user, &(i128::MAX - 10));
    let result = s.client.try_deposit(&s.user, &20);
    assert_eq!(result, Err(Ok(Error::ArithmeticOverflow)));
}

#[test]
fn deposit_succeeds_with_real_sender_auth() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    s.client.deposit(&s.user, &999_900);
    assert_eq!(
        s.client.try_deposit(&s.user, &200),
        Err(Ok(Error::LimitExceeded))
    );
}

#[test]
fn set_limit_succeeds_for_owner_with_real_auth() {
    let s = Setup::new(1_000_000);
    s.client.set_limit(&s.owner, &2_000_000);
}

#[test]
fn withdraw_succeeds_with_real_owner_auth() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &1_000);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &400);

    assert_eq!(s.token.balance(&recipient), 400);
}

// ── Auth-bypass regressions ─────────────────────────────────────────────────

fn seed_vault(
    env: &Env,
    vault_id: &Address,
    owner: &Address,
    token_addr: &Address,
    balance: i128,
    max_limit: i128,
) {
    env.as_contract(vault_id, || {
        storage::set_owner(env, owner);
        storage::set_token(env, token_addr);
        storage::set_max_limit(env, &max_limit);
        storage::set_balance(env, &balance);
        storage::set_pending(env, &None);
    });
}

#[test]
#[should_panic]
fn withdraw_without_owner_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let stranger = Address::generate(&env);
    let to = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 1_000, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    // stranger is neither owner nor operator — must panic
    client.withdraw(&stranger, &to, &1);
}

#[test]
#[should_panic]
fn set_limit_with_owner_address_but_no_real_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 0, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    // Passing the real owner address without signed auth must still panic.
    client.set_limit(&owner, &2_000_000);
}

#[test]
#[should_panic]
fn deposit_without_sender_auth_panics() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let user = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 0, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    client.deposit(&user, &1);
}

// ── Operator delegation tests ───────────────────────────────────────────────

#[test]
fn owner_can_set_and_revoke_operator() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    assert_eq!(s.client.operator(), None);

    s.client.set_operator(&s.owner, &op);
    assert_eq!(s.client.operator(), Some(op.clone()));

    s.client.revoke_operator(&s.owner);
    assert_eq!(s.client.operator(), None);
}

#[test]
fn non_owner_cannot_set_operator() {
    let s = Setup::new(1_000_000);

    let stranger = Address::generate(&s.env);
    let op = Address::generate(&s.env);
    let result = s.client.try_set_operator(&stranger, &op);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn non_owner_cannot_revoke_operator() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let stranger = Address::generate(&s.env);
    let result = s.client.try_revoke_operator(&stranger);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn operator_can_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let recipient = Address::generate(&s.env);
    s.client.withdraw(&op, &recipient, &200);
    assert_eq!(s.token.balance(&recipient), 200);
}

#[test]
fn operator_can_set_limit() {
    let s = Setup::new(1_000_000);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    s.client.set_limit(&op, &2_000_000);
}

#[test]
fn stranger_cannot_withdraw_even_with_operator_set() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);

    let stranger = Address::generate(&s.env);
    let result = s.client.try_withdraw(&stranger, &stranger, &1);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn after_revoke_operator_can_no_longer_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.revoke_operator(&s.owner);

    let result = s.client.try_withdraw(&op, &op, &1);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn revoke_operator_when_none_set_is_ok() {
    let s = Setup::new(1_000_000);
    // No operator set — revoke should be a no-op, not an error.
    let result = s.client.try_revoke_operator(&s.owner);
    assert!(result.is_ok());
}

// ── Emergency pause tests ───────────────────────────────────────────────────

#[test]
fn owner_can_pause_and_unpause() {
    let s = Setup::new(1_000_000);

    assert!(!s.client.is_paused());
    s.client.pause(&s.owner);
    assert!(s.client.is_paused());
    s.client.unpause(&s.owner);
    assert!(!s.client.is_paused());
}

#[test]
fn non_owner_cannot_pause() {
    let s = Setup::new(1_000_000);
    let stranger = Address::generate(&s.env);
    let result = s.client.try_pause(&stranger);
    assert_eq!(result, Err(Ok(Error::NotAuthorized)));
}

#[test]
fn double_pause_is_rejected() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_pause(&s.owner);
    assert_eq!(result, Err(Ok(Error::AlreadyPaused)));
}

#[test]
fn unpause_when_not_paused_is_rejected() {
    let s = Setup::new(1_000_000);
    let result = s.client.try_unpause(&s.owner);
    assert_eq!(result, Err(Ok(Error::NotPaused)));
}

#[test]
fn pause_blocks_deposit() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_deposit(&s.user, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_withdraw() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &500);
    s.client.pause(&s.owner);

    let recipient = Address::generate(&s.env);
    let result = s.client.try_withdraw(&s.owner, &recipient, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn pause_blocks_set_limit() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    let result = s.client.try_set_limit(&s.owner, &2_000_000);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}

#[test]
fn operations_resume_after_unpause() {
    let s = Setup::new(1_000_000);
    s.client.pause(&s.owner);
    s.client.unpause(&s.owner);

    // All three should work again after unpause
    s.client.deposit(&s.user, &100);
    s.client.set_limit(&s.owner, &2_000_000);
    let recipient = Address::generate(&s.env);
    s.client.withdraw(&s.owner, &recipient, &50);
    assert_eq!(s.token.balance(&recipient), 50);
}

#[test]
fn operator_also_blocked_by_pause() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &500);

    let op = Address::generate(&s.env);
    s.client.set_operator(&s.owner, &op);
    s.client.pause(&s.owner);

    let recipient = Address::generate(&s.env);
    let result = s.client.try_withdraw(&op, &recipient, &100);
    assert_eq!(result, Err(Ok(Error::ContractPaused)));
}
