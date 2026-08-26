#![no_std]

mod errors;
mod events;
mod storage;
#[cfg(test)]
mod tests;

use errors::Error;
use soroban_sdk::{contract, contractimpl, panic_with_error, token, Address, Env};
use storage::{
    get_balance, get_max_limit, get_operator, get_owner, get_token, is_paused, remove_operator,
    set_balance, set_max_limit, set_operator, set_owner, set_paused, set_token,
};

#[contract]
pub struct TokenVault;

/// Checks that `caller` is either the vault owner or the currently delegated
/// operator, then consumes the caller's auth. Returns `NotAuthorized` if
/// `caller` matches neither role.
///
/// Mirrors `DripStream::require_sender_or_operator` — the owner can hand off
/// day-to-day withdrawal authority to a hot wallet / ops key without exposing
/// the cold owner key for routine operations.
fn require_owner_or_operator(env: &Env, caller: &Address, owner: &Address) -> Result<(), Error> {
    let operator = get_operator(env);
    let is_owner = caller == owner;
    let is_op = operator.as_ref().map(|op| caller == op).unwrap_or(false);
    if is_owner || is_op {
        caller.require_auth();
        Ok(())
    } else {
        Err(Error::NotAuthorized)
    }
}

/// Short-circuit helper: reject any state-mutating call while paused.
fn assert_not_paused(env: &Env) -> Result<(), Error> {
    if is_paused(env) {
        Err(Error::ContractPaused)
    } else {
        Ok(())
    }
}

#[contractimpl]
impl TokenVault {
    pub fn initialize(env: Env, owner: Address, token: Address, max_limit: i128) {
        if max_limit <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }

        if get_owner(&env).is_some() {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }

        set_owner(&env, &owner);
        set_token(&env, &token);
        set_max_limit(&env, &max_limit);
        set_balance(&env, &0_i128);

        events::initialized(&env, &owner, &token, max_limit);
    }

    pub fn deposit(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        from.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let _owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        // Check current balance and max_limit safely
        let balance = get_balance(&env).unwrap_or(0_i128);
        let max = get_max_limit(&env).ok_or(Error::NotInitialized)?;

        let new_balance = balance
            .checked_add(amount)
            .ok_or(Error::ArithmeticOverflow)?;
        if new_balance > max {
            return Err(Error::LimitExceeded);
        }

        // perform token transfer
        let tk = token::Client::new(&env, &get_token(&env).ok_or(Error::NotInitialized)?);
        tk.transfer(&from, &env.current_contract_address(), &amount);

        set_balance(&env, &new_balance);
        events::deposited(&env, &from, amount, new_balance);
        Ok(())
    }

    pub fn withdraw(env: Env, caller: Address, to: Address, amount: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        require_owner_or_operator(&env, &caller, &owner)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let balance = get_balance(&env).unwrap_or(0_i128);
        let new_balance = balance
            .checked_sub(amount)
            .ok_or(Error::ArithmeticOverflow)?;

        let tk = token::Client::new(&env, &get_token(&env).ok_or(Error::NotInitialized)?);
        tk.transfer(&env.current_contract_address(), &to, &amount);

        set_balance(&env, &new_balance);
        events::withdrawn(&env, &caller, &to, amount, new_balance);
        Ok(())
    }

    pub fn set_limit(env: Env, caller: Address, new_limit: i128) -> Result<(), Error> {
        assert_not_paused(&env)?;
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        require_owner_or_operator(&env, &caller, &owner)?;

        if new_limit <= 0 {
            return Err(Error::InvalidAmount);
        }
        let balance = get_balance(&env).unwrap_or(0_i128);
        if new_limit < balance {
            return Err(Error::LimitExceeded);
        }
        let old_limit = get_max_limit(&env).ok_or(Error::ArithmeticOverflow)?;
        set_max_limit(&env, &new_limit);
        events::limit_set(&env, &caller, old_limit, new_limit);
        Ok(())
    }

    // ── Operator delegation (owner-gated) ─────────────────────────────────

    /// Owner designates an operator who can perform owner-level actions
    /// (`withdraw`, `set_limit`) on this vault.
    ///
    /// Only the owner may call this. Matches `DripStream::set_operator` — the
    /// owner can delegate day-to-day operations to a hot wallet while keeping
    /// the owner key in cold storage.
    pub fn set_operator(env: Env, caller: Address, operator: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        set_operator(&env, &operator);
        events::operator_set(&env, &caller, &operator);
        Ok(())
    }

    /// Owner revokes the operator, removing all delegated authority.
    ///
    /// No-op (not an error) if no operator is currently set.
    pub fn revoke_operator(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        remove_operator(&env);
        events::operator_revoked(&env, &caller);
        Ok(())
    }

    /// Read-only: the current operator address, if any.
    pub fn operator(env: Env) -> Option<Address> {
        get_operator(&env)
    }

    // ── Emergency pause (owner-gated) ─────────────────────────────────────

    /// Emergency halt: freeze all state-mutating operations.
    ///
    /// While paused, `deposit`, `withdraw`, and `set_limit` all revert with
    /// `ContractPaused` before touching any state. Matches the
    /// `pause`/`unpause`/`is_paused` triple present on `DripFactory`,
    /// `DripGovernor`, and `TwapOracle`.
    pub fn pause(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        if is_paused(&env) {
            return Err(Error::AlreadyPaused);
        }
        set_paused(&env, true);
        events::paused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the emergency pause, re-enabling all state-mutating operations.
    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        let owner = get_owner(&env).ok_or(Error::NotInitialized)?;
        if caller != owner {
            return Err(Error::NotAuthorized);
        }
        caller.require_auth();
        if !is_paused(&env) {
            return Err(Error::NotPaused);
        }
        set_paused(&env, false);
        events::unpaused(&env, &caller, env.ledger().timestamp());
        Ok(())
    }

    /// Read-only: whether the vault is currently under an emergency pause.
    pub fn is_paused(env: Env) -> bool {
        is_paused(&env)
    }
}
