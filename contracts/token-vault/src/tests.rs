#![cfg(test)]

extern crate std;
use std::boxed::Box;

use soroban_sdk::{testutils::LedgerInfo, token, Address, Env};

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

        let tok = token::Client::new(&env, &token_addr);
        let tok_admin = token::StellarAssetClient::new(&env, &token_addr);

        // Mint a large supply to the user so deposits succeed
        tok_admin.mint(&user, &(max_limit));

        let vault_id = env.register_contract(None, super::TokenVault);
        let client = TokenVaultClient::new(&env, &vault_id);

        client.initialize(&owner, &token_addr, &max_limit);

        // Leak env for static lifetime convenience in tests
        let env: &'static Env = Box::leak(Box::new(env));
        let client = TokenVaultClient::new(env, &vault_id);
        let token = token::Client::new(env, &token_addr);

        Setup {
            env: unsafe { std::ptr::read(env) },
            client,
            token,
            owner,
            user,
        }
    }
}

#[test]
fn deposit_respects_limits_and_avoids_overflow() {
    // Use a very large limit near i128::MAX to ensure arithmetic checks fire
    let max_limit: i128 = i128::MAX - 10;
    let s = Setup::new(max_limit);

    // First deposit up to the limit
    s.token.transfer(&s.user, &s.client.address, &max_limit);

    // depositing an extra amount that would overflow if unchecked
    let result = s.client.try_deposit(&s.user, &20);
    // We expect the contract to reject because it would exceed the max limit
    assert_eq!(result, Err(Ok(super::errors::Error::LimitExceeded)));
}
