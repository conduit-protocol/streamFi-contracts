#![cfg(test)]

extern crate std;

use soroban_sdk::{testutils::Address as _, token, Address, Env};

use crate::TokenVaultClient;
use crate::storage;

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

        // Leak env for 'static lifetime convenience in tests -- Env is cheap
        // to clone (it's a handle around shared Rc-like internals), so we
        // just clone the leaked reference back out instead of the previous
        // `unsafe { std::ptr::read(env) }`, which created two values that
        // both believed they owned the same underlying Env.
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

#[test]
fn deposit_respects_max_limit() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    // Deposit up to the limit through the contract itself (not a raw token
    // transfer), so the internal `balance` counter and the real on-chain
    // balance move together the way `deposit`'s own bookkeeping assumes.
    s.client.deposit(&s.user, &max_limit);

    // Any additional deposit must be rejected for exceeding max_limit.
    let result = s.client.try_deposit(&s.user, &1);
    assert_eq!(result, Err(Ok(super::errors::Error::LimitExceeded)));
}

#[test]
fn deposit_rejects_amount_that_would_overflow_i128() {
    // max_limit is i128::MAX, so the LimitExceeded check can never fire --
    // this isolates the checked_add overflow guard specifically.
    let s = Setup::new(i128::MAX);

    s.client.deposit(&s.user, &(i128::MAX - 10));
    let result = s.client.try_deposit(&s.user, &20);
    assert_eq!(result, Err(Ok(super::errors::Error::ArithmeticOverflow)));
}

#[test]
fn deposit_succeeds_with_real_sender_auth() {
    let max_limit: i128 = 1_000_000;
    let s = Setup::new(max_limit);

    // mock_all_auths() satisfies `from.require_auth()` here -- this is the
    // control case proving the happy path still works once auth is enforced.
    s.client.deposit(&s.user, &999_900);
    assert_eq!(s.client.try_deposit(&s.user, &200), Err(Ok(super::errors::Error::LimitExceeded)));
}

#[test]
fn set_limit_succeeds_for_owner_with_real_auth() {
    let s = Setup::new(1_000_000);
    // mock_all_auths() satisfies owner.require_auth() here -- proves the
    // happy path still works once set_limit() enforces real authorization.
    s.client.set_limit(&s.owner, &2_000_000);
}

#[test]
fn withdraw_succeeds_with_real_owner_auth() {
    let s = Setup::new(1_000_000);
    s.client.deposit(&s.user, &1_000);

    let recipient = Address::generate(&s.env);
    // mock_all_auths() satisfies owner.require_auth() here -- proves the
    // happy path still works once withdraw() enforces real authorization.
    s.client.withdraw(&recipient, &400);

    assert_eq!(s.token.balance(&recipient), 400);
}

// ── Auth-bypass regressions ─────────────────────────────────────────────────
//
// These deliberately do NOT call `env.mock_all_auths()` (which would make
// every `require_auth()` call succeed regardless of who's actually calling,
// hiding exactly the bugs being tested for). Storage is seeded directly via
// `env.as_contract(...)` so setup itself never needs to satisfy an auth
// check -- each test isolates only the one `require_auth()` call it cares
// about, which must panic since no matching authorization was ever provided.

fn seed_vault(env: &Env, vault_id: &Address, owner: &Address, token_addr: &Address, balance: i128, max_limit: i128) {
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
    let to = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let token_addr = env
        .register_stellar_asset_contract_v2(token_admin)
        .address();
    let vault_id = env.register_contract(None, super::TokenVault);

    seed_vault(&env, &vault_id, &owner, &token_addr, 1_000, 1_000_000);

    let client = TokenVaultClient::new(&env, &vault_id);
    // No auth was ever provided for `owner` -- withdraw() must panic
    // rather than let an arbitrary caller drain the vault.
    client.withdraw(&to, &1);
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
    // Passing the real owner's (public) address as `caller` is not proof of
    // identity -- without a real signed authorization, this must still panic.
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
    // `user` never authorized this deposit -- must panic rather than
    // silently pulling funds via the underlying token's own auth check.
    client.deposit(&user, &1);
}
