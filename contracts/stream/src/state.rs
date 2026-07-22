use soroban_sdk::Env;

use crate::storage::{DataKey, StreamInfo, FLAG_CANCELLED, FLAG_PAUSED};
use crate::Error;

pub fn load(env: &Env) -> StreamInfo {
    StreamInfo {
        sender: env.storage().instance().get(&DataKey::Sender).unwrap(),
        recipient: env.storage().instance().get(&DataKey::Recipient).unwrap(),
        token: env.storage().instance().get(&DataKey::Token).unwrap(),
        rate_per_second: env
            .storage()
            .instance()
            .get(&DataKey::RatePerSecond)
            .unwrap(),
        start_time: env.storage().instance().get(&DataKey::StartTime).unwrap(),
        end_time: env.storage().instance().get(&DataKey::EndTime).unwrap(),
        withdrawn: env
            .storage()
            .instance()
            .get(&DataKey::Withdrawn)
            .unwrap_or(0),
        paused_at: env
            .storage()
            .instance()
            .get(&DataKey::PausedAt)
            .unwrap_or(0),
        flags: env
            .storage()
            .instance()
            .get(&DataKey::Flags)
            .unwrap_or(0),
    }
}

pub fn save_withdrawn(env: &Env, amount: i128) {
    env.storage().instance().set(&DataKey::Withdrawn, &amount);
}

pub fn set_paused(env: &Env, paused: bool) {
    let mut flags: u32 = env.storage().instance().get(&DataKey::Flags).unwrap_or(0);
    if paused {
        flags |= FLAG_PAUSED;
    } else {
        flags &= !FLAG_PAUSED;
    }
    env.storage().instance().set(&DataKey::Flags, &flags);
}

pub fn set_cancelled(env: &Env) {
    let mut flags: u32 = env.storage().instance().get(&DataKey::Flags).unwrap_or(0);
    flags |= FLAG_CANCELLED;
    env.storage().instance().set(&DataKey::Flags, &flags);
}

pub fn assert_not_cancelled(info: &StreamInfo) -> Result<(), Error> {
    if info.is_cancelled() {
        Err(Error::StreamCancelled)
    } else {
        Ok(())
    }
}
