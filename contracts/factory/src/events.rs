use soroban_sdk::{symbol_short, Address, Env};

/// Emergency-pause observability events.
///
/// The factory's `pause`/`unpause` are already idempotent — a redundant call
/// reverts with `AlreadyPaused`/`NotPaused` rather than silently no-op'ing.
/// These events close the remaining gap: they let off-chain infrastructure
/// (indexers, relayers) positively confirm that a state transition committed,
/// rather than having to infer it from a bare "ok" that an ambiguous or
/// rate-limited RPC response may have dropped or duplicated. A relayer that
/// retried after a lost response can reconcile against the emitted event
/// instead of re-issuing a call it can no longer distinguish as a no-op.
///
/// Publication and the `set_paused` storage write are part of the same Soroban
/// transaction, so either both commit or both roll back — an event is never
/// emitted for a transition that did not actually persist.
///
/// Emitted when the factory transitions from unpaused to paused.
///
/// Topics: `("paused", governor)` — the governor that authorized the halt.
/// Data:   `paused_at` — the ledger timestamp at which the halt took effect.
pub fn paused(env: &Env, governor: &Address, paused_at: u64) {
    env.events()
        .publish((symbol_short!("paused"), governor.clone()), paused_at);
}

/// Emitted when the factory transitions from paused back to unpaused.
///
/// Topics: `("unpaused", governor)` — the governor that lifted the halt.
/// Data:   `resumed_at` — the ledger timestamp at which creation resumed.
pub fn unpaused(env: &Env, governor: &Address, resumed_at: u64) {
    env.events()
        .publish((symbol_short!("unpaused"), governor.clone()), resumed_at);
}
