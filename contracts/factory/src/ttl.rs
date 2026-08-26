use soroban_sdk::Env;

use crate::storage::DataKey;

// Exposed as `pub` so callers can reuse the same threshold/extend-to values
// for the persistent BySender/ByRecipient/StreamAddr registry entries.
pub use drip_common::TTL_EXTEND_TO as EXTEND_TO;
pub use drip_common::TTL_THRESHOLD as THRESHOLD;

/// How many persistent entries the bounded walker bumps per call. Sized so
/// the gas cost of any single `pause`/`unpause`/`upgrade_stream_wasm`
/// invocation remains bounded — the walker linearly scans at most this many
/// IDs. The walker wraps around modulo `StreamCount` so every live ID is
/// eventually covered across calls.
pub const BATCH_LIMIT: u32 = 8;

pub fn bump_instance(env: &Env) {
    env.storage().instance().extend_ttl(THRESHOLD, EXTEND_TO);
}

/// Bounded TTL walker.
///
/// Advances `DataKey::LastBumpedId` by `BATCH_LIMIT`, wrapping around modulo
/// `DataKey::StreamCount`, and bumps the persistent `StreamAddr(id)` TTLs in
/// that window. Bumps only live (existing) entries — the `has` check is
/// defensive against any drift between `StreamCount` and the actual
/// persistent set (e.g. after a future migration or partial archival). The
/// return value is the number of live entries touched (informational;
///
/// Idempotent and side-effect-bounded: reads at most `BATCH_LIMIT` entries
/// from persistent storage, writes exactly one instance entry
/// (`DataKey::LastBumpedId`). Safe to call from every maintenance
/// invocation regardless of factory state. `extend_ttl` is gated by `has`
/// in the public read paths for the same reason — it is required to skip
/// missing persistent entries.
pub fn bump_persistent_bucket(env: &Env) -> u32 {
    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::StreamCount)
        .unwrap_or(0);
    if count == 0 {
        return 0;
    }

    let cursor: u64 = env
        .storage()
        .instance()
        .get(&DataKey::LastBumpedId)
        .unwrap_or(0);

    let mut new_last: u64 = cursor;
    let mut touched: u32 = 0;

    // Walk the next `BATCH_LIMIT` IDs starting AFTER the cursor, wrapping
    // around modulo `count`. We `saturating_add` rather than plain `+` so a
    // runaway/crafted cursor (or `BATCH_LIMIT` large enough to exceed u64)
    // cannot produce a panic from integer overflow; the modulo wrap still
    // behaves correctly because `count > 0` here (early-return above).
    for i in 0..BATCH_LIMIT {
        let next: u64 = cursor
            .saturating_add(1)
            .saturating_add(u32_into_u64_saturating(i));
        let id: u64 = next % count;
        let key = DataKey::StreamAddr(id);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, THRESHOLD, EXTEND_TO);
            touched += 1;
        }
        new_last = id;
    }

    env.storage()
        .instance()
        .set(&DataKey::LastBumpedId, &new_last);
    touched
}

// Helper to avoid the `as` casting lint in the saturating_add chain.
fn u32_into_u64_saturating(v: u32) -> u64 {
    v as u64
}
