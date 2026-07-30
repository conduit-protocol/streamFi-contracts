use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    Token,
    MaxLimit,
    Balance,
    PendingCallback,
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
