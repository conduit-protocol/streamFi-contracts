use soroban_sdk::{Env, Symbol};

const STATE_VERSION_KEY: Symbol = Symbol::new(b"state_version");

pub fn process_batch(env: &Env) {
    let version = load_state_version(env);
    bump_state_version(env, version);
}

fn load_state_version(env: &Env) -> u64 {
    env.storage().instance().get(&STATE_VERSION_KEY).unwrap_or(0)
}

fn bump_state_version(env: &Env, version: u64) {
    env.storage().instance().set(&STATE_VERSION_KEY, &(version + 1));
}
