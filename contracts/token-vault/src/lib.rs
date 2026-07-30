#![no_std]

mod errors;
mod storage;
#[cfg(test)]
mod tests;

use errors::Error;
use soroban_sdk::{contract, contractimpl, panic_with_error, token, Address, Env};
use storage::{get_balance, get_max_limit, get_pending, get_token, get_owner, set_balance, set_max_limit, set_owner, set_pending, set_token};

#[contract]
pub struct TokenVault;

#[contractimpl]
impl TokenVault {
    pub fn initialize(env: Env, owner: Address, token: Address, max_limit: i128) {
        if max_limit <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        if get_owner(&env).is_some() {
            panic_with_error!(&env, Error::NotAuthorized);
        }

        set_owner(&env, &owner);
        set_token(&env, &token);
        set_max_limit(&env, &max_limit);
        set_balance(&env, &0_i128);
        set_pending(&env, &None);
    }

    pub fn deposit(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        // Cleanup any pending callbacks before state mutation
        Self::cleanup_pending(&env);

        let owner = get_owner(&env).ok_or(Error::NotAuthorized)?;
        // Check current balance and max_limit safely
        let balance = get_balance(&env).unwrap_or(0_i128);
        let max = get_max_limit(&env).ok_or(Error::ArithmeticOverflow)?;

        let new_balance = balance.checked_add(amount).ok_or(Error::ArithmeticOverflow)?;
        if new_balance > max {
            return Err(Error::LimitExceeded);
        }

        // perform token transfer
        let tk = token::Client::new(&env, &get_token(&env).ok_or(Error::ArithmeticOverflow)?);
        tk.transfer(&from, &env.current_contract_address(), &amount);

        set_balance(&env, &new_balance);
        // clear pending after success
        set_pending(&env, &None);
        let _ = owner;
        Ok(())
    }

    pub fn withdraw(env: Env, to: Address, amount: i128) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotAuthorized)?;
        owner.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        Self::cleanup_pending(&env);

        let balance = get_balance(&env).unwrap_or(0_i128);
        let new_balance = balance.checked_sub(amount).ok_or(Error::ArithmeticOverflow)?;

        let tk = token::Client::new(&env, &get_token(&env).ok_or(Error::ArithmeticOverflow)?);
        tk.transfer(&env.current_contract_address(), &to, &amount);

        set_balance(&env, &new_balance);
        set_pending(&env, &None);
        Ok(())
    }

    pub fn set_limit(env: Env, caller: Address, new_limit: i128) -> Result<(), Error> {
        // Only owner may set limit
        let owner = get_owner(&env).ok_or(Error::NotAuthorized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        owner.require_auth();
        if new_limit <= 0 {
            return Err(Error::InvalidAmount);
        }
        let balance = get_balance(&env).unwrap_or(0_i128);
        if new_limit < balance {
            return Err(Error::LimitExceeded);
        }
        set_max_limit(&env, &new_limit);
        Ok(())
    }

    // Exposed for tests: ensures any pending async callbacks are cleared.
    pub fn cleanup_pending(env: &Env) {
        // If there was a pending callback payload, drop it instead of processing
        if let Some(Some(_v)) = get_pending(env) {
            set_pending(env, &None);
        }
    }
}
