use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    Token,
    MaxLimit,
    Balance,
    PendingCallback,
    /// Optional operator address delegated by the owner.
    ///
    /// When set, the operator can perform owner-level actions (`withdraw`,
    /// `set_limit`) on behalf of the owner — a hot-wallet / ops-key
    /// pattern matching `DripStream`'s `set_operator` design.
    /// Absent key means no operator has been delegated.
    Operator,
    /// Emergency-pause flag. When `true`, all state-mutating entry points
    /// (`deposit`, `withdraw`, `set_limit`) revert before touching state.
    Paused,
}

impl DataKey {
    pub fn owner_key() -> &'static str {
        "owner"
    }
}

pub fn set_owner(env: &Env, a: &Address) {
    env.storage().instance().set(&DataKey::Owner, a);
}

pub fn get_owner(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Owner)
}

pub fn set_token(env: &Env, t: &Address) {
    env.storage().instance().set(&DataKey::Token, t);
}

pub fn get_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Token)
}

pub fn set_max_limit(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::MaxLimit, v);
}

pub fn get_max_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxLimit)
}

pub fn set_balance(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::Balance, v);
}

pub fn get_balance(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::Balance)
}

pub fn set_pending(env: &Env, v: &Option<i128>) {
    env.storage().instance().set(&DataKey::PendingCallback, v);
}

pub fn get_pending(env: &Env) -> Option<Option<i128>> {
    env.storage().instance().get(&DataKey::PendingCallback)
}

pub fn set_operator(env: &Env, op: &Address) {
    env.storage().instance().set(&DataKey::Operator, op);
}

pub fn get_operator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Operator)
}

pub fn remove_operator(env: &Env) {
    env.storage().instance().remove(&DataKey::Operator);
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}
