//! Emergency-pause flag storage and gating for `DripFactory`.
//!
//! The flag read/write and the "reject while halted" gate are protocol-wide
//! policy and live in [`drip_common::pause`]; this module only keeps the
//! crate-local call-site names (`pause::is_paused`, `pause::set_paused`,
//! `pause::require_not_paused`) unchanged — it must not restate the gate
//! (issue #650).

use soroban_sdk::Env;

use crate::errors::Error;
use crate::storage::DataKey;

/// Reads the emergency-pause flag.
///
/// Defaults to `false` (unpaused) when the key has never been set — e.g. a
/// factory that was initialized before this feature existed. This keeps the
/// flag backward-compatible: an absent entry means "running normally".
pub fn is_paused(env: &Env) -> bool {
    drip_common::pause::is_paused(env, &DataKey::Paused)
}

/// Writes the emergency-pause flag directly to instance storage.
///
/// # Module Boundary & Access Control
/// This is a low-level internal storage helper. It does **not** perform caller
/// authorization (such as checking governor gating via `require_auth`) or
/// enforce state invariants (such as checking if the factory is already paused
/// or unpaused).
///
/// Callers (e.g., [`DripFactory::pause`](crate::DripFactory::pause) and
/// [`DripFactory::unpause`](crate::DripFactory::unpause) in `lib.rs`) are
/// responsible for verifying caller authorization, checking current pause
/// state transitions, updating TTL, and emitting public events.
pub fn set_paused(env: &Env, paused: bool) {
    drip_common::pause::set_paused(env, &DataKey::Paused, paused);
}

/// The pause gate: `Err(ContractPaused)` while the factory is halted.
///
/// Delegates to [`drip_common::pause::require_not_paused`] and maps the shared
/// error onto the factory's own `Error`, so the halt check is byte-for-byte the
/// same gate every sibling contract runs.
pub fn require_not_paused(env: &Env) -> Result<(), Error> {
    drip_common::pause::require_not_paused(env, &DataKey::Paused).map_err(|_| Error::ContractPaused)
}
