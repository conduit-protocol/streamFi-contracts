use soroban_sdk::Env;

use crate::storage::{DataKey, StreamInfo, FLAG_CANCELLED, FLAG_PAUSED};
use crate::Error;

/// Load the full stream state in a single storage read.
///
/// Tries the consolidated `Config` key first (written by all new
/// `initialize()` calls). Falls back to reading each field individually
/// for streams that were initialised before this optimisation landed —
/// this keeps old on-chain instances readable without a migration.
pub fn load(env: &Env) -> StreamInfo {
    let s = env.storage().instance();

    // Fast path: stream was initialised with the consolidated key.
    if let Some(info) = s.get::<_, StreamInfo>(&DataKey::Config) {
        return info;
    }

    // Legacy path: read each field individually (pre-optimisation streams).
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
        flags: env.storage().instance().get(&DataKey::Flags).unwrap_or(0),
        flags: env
            .storage()
            .instance()
            .get(&DataKey::Flags)
            .unwrap_or(0),
        sender: s.get(&DataKey::Sender).unwrap(),
        recipient: s.get(&DataKey::Recipient).unwrap(),
        token: s.get(&DataKey::Token).unwrap(),
        rate_per_second: s.get(&DataKey::RatePerSecond).unwrap(),
        start_time: s.get(&DataKey::StartTime).unwrap(),
        end_time: s.get(&DataKey::EndTime).unwrap(),
        withdrawn: s.get(&DataKey::Withdrawn).unwrap_or(0),
        paused: s.get(&DataKey::Paused).unwrap_or(false),
        paused_at: s.get(&DataKey::PausedAt).unwrap_or(0),
        clawback_enabled: s.get(&DataKey::ClawbackEnabled).unwrap_or(false),
        cancelled: s.get(&DataKey::Cancelled).unwrap_or(false),
    }
}

/// Persist the entire stream state in a single storage write.
pub fn save(env: &Env, info: &StreamInfo) {
    env.storage().instance().set(&DataKey::Config, info);
}

/// Update only the `withdrawn` counter without touching the other fields.
///
/// Uses load-modify-save so the single-struct layout is maintained.
pub fn save_withdrawn(env: &Env, amount: i128) {
    let mut info = load(env);
    info.withdrawn = amount;
    save(env, &info);
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
