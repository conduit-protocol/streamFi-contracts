use soroban_sdk::Env;

use crate::storage::DataKey;

/// Reads the emergency-pause flag.
///
/// Defaults to `false` (unpaused) when the key has never been set — e.g. a
/// factory that was initialized before this feature existed. This keeps the
/// flag backward-compatible: an absent entry means "running normally".
pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false)
}

/// Writes the emergency-pause flag directly to instance storage.
///
/// # Module Boundary & Access Control
/// This is a low-level internal storage helper. It does **not** perform caller
/// authorization (such as checking governor gating via `require_auth`) or
/// enforce state invariants (such as checking if the factory is already paused
/// or unpaused).
///
/// Callers (e.g., [`DripFactory::pause`] and [`DripFactory::unpause`] in `lib.rs`)
/// are responsible for verifying caller authorization, checking current pause
/// state transitions, updating TTL, and emitting public events.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&DataKey::Paused, &paused);
}
