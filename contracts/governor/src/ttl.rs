use soroban_sdk::Env;

use drip_common::{TTL_EXTEND_TO, TTL_THRESHOLD};

pub fn bump(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}
