//! Shared emergency-pause helpers for the Drip protocol contracts.
//!
//! Every contract in the protocol can be halted by its authority
//! (`pause`/`unpause`/`is_paused`) and every state-mutating entry point is
//! expected to reject the call while halted. The gate itself is the same
//! everywhere — read one instance-storage flag, and refuse to proceed when it
//! is set — but it was previously open-coded per contract, with each copy
//! naming the flag key differently and mapping the failure to its own error
//! variant. A copy that silently loses its guard is a protocol-halt failure
//! mode, so the gate lives here once.
//!
//! Contracts keep their own `Error` enum (every code means something different
//! per contract), so [`PausedError`] is mapped at each call site:
//!
//! ```ignore
//! drip_common::pause::require_not_paused(env, &DataKey::Paused)
//!     .map_err(|_| Error::ContractPaused)?;
//! ```
//!
//! The flag is *not* gated here — reading and writing it is the contract's
//! business (only its pause authority may flip it), so [`is_paused`] and
//! [`set_paused`] are low-level storage helpers like the per-contract ones they
//! replace.

use soroban_sdk::Env;

use crate::rbac::StorageKey;

/// Errors the shared pause gate can return.
///
/// Each contract maps this onto its own `Error` variant (typically
/// `ContractPaused`), so no `#[contracterror]` attribute is needed here — the
/// same approach `drip_common::rbac::RbacError` takes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PausedError {
    /// The contract is under an emergency pause, so the call is refused.
    ContractPaused,
}

/// Reads the emergency-pause flag stored under `paused_key` in `instance()`
/// storage.
///
/// Defaults to `false` when the key was never written, so a contract deployed
/// before the pause feature existed is treated as running normally rather than
/// halting every call.
pub fn is_paused<K: StorageKey>(env: &Env, paused_key: &K) -> bool {
    env.storage().instance().get(paused_key).unwrap_or(false)
}

/// Writes the emergency-pause flag to `instance()` storage.
///
/// Performs no authorization and no state-transition validation: the caller
/// (the contract's own `pause`/`unpause`) owns both, and is responsible for
/// emitting the matching event and bumping TTL.
pub fn set_paused<K: StorageKey>(env: &Env, paused_key: &K, paused: bool) {
    env.storage().instance().set(paused_key, &paused);
}

/// The pause gate: `Ok(())` when the contract is running, `Err` while halted.
///
/// Place it at the top of a state-mutating entry point — before validation,
/// storage reads, and TTL payment — so a halted protocol rejects the call
/// immediately and cheaply, and so no state is touched on the rejected path.
pub fn require_not_paused<K: StorageKey>(env: &Env, paused_key: &K) -> Result<(), PausedError> {
    require_not_paused_flag(is_paused(env, paused_key))
}

/// [`require_not_paused`] for a flag the caller already has in memory — e.g. a
/// field inside a stored struct rather than a standalone instance key
/// (`DripStream` keeps its pause bit in the consolidated `StreamInfo`).
///
/// Accepting the flag instead of re-reading storage keeps the gate free to use
/// where the contract has already loaded the record it needs to gate on.
pub fn require_not_paused_flag(paused: bool) -> Result<(), PausedError> {
    if paused {
        Err(PausedError::ContractPaused)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, contracttype};

    // ── Contract frame ────────────────────────────────────────────────────────
    //
    // `pause` is storage-only shared code and owns no `#[contract]` of its own,
    // so the suite supplies the frame `soroban_sdk` requires before any
    // instance-storage access can run. The host is registered per test purely
    // so the frame has instance storage to open.

    #[contract]
    struct PauseTestHost;

    #[contractimpl]
    impl PauseTestHost {
        /// Never invoked. Present only because registering a contract requires
        /// at least one callable entry point.
        pub fn noop(_env: Env) {}
    }

    fn in_contract<T>(env: &Env, f: impl FnOnce() -> T) -> T {
        env.as_contract(&env.register_contract(None, PauseTestHost), f)
    }

    // ── Minimal key space ─────────────────────────────────────────────────────
    //
    // Mirrors the rbac suite: the helpers are generic over the flag key, so the
    // tests supply their own `#[contracttype]` key instead of reaching into a
    // consumer contract's `DataKey`.

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestKey {
        Paused,
        StreamAddr(u64),
    }

    const PAUSED: TestKey = TestKey::Paused;

    // ── The flag is readable and writable, defaulting to running ──────────────

    #[test]
    fn unset_flag_reads_as_not_paused() {
        let env = Env::default();

        in_contract(&env, || {
            // Absent key means "running normally" — a contract deployed before
            // the pause feature existed must not be halted by a missing entry.
            assert!(!is_paused(&env, &PAUSED));
        });
    }

    #[test]
    fn set_paused_round_trips_and_clears() {
        let env = Env::default();

        in_contract(&env, || {
            set_paused(&env, &PAUSED, true);
            assert!(is_paused(&env, &PAUSED));

            set_paused(&env, &PAUSED, false);
            assert!(!is_paused(&env, &PAUSED));
        });
    }

    // ── require_not_paused ────────────────────────────────────────────────────

    #[test]
    fn require_not_paused_accepts_a_running_contract() {
        let env = Env::default();

        in_contract(&env, || {
            assert_eq!(require_not_paused(&env, &PAUSED), Ok(()));
        });
    }

    #[test]
    fn require_not_paused_rejects_a_halted_contract() {
        let env = Env::default();

        in_contract(&env, || {
            set_paused(&env, &PAUSED, true);
            assert_eq!(
                require_not_paused(&env, &PAUSED),
                Err(PausedError::ContractPaused)
            );
        });
    }

    /// The two entry points must agree: the key-based gate is a thin wrapper
    /// over the flag-based one, and a consumer that already holds the flag
    /// (inside a loaded struct) has to reach the same verdict.
    #[test]
    fn flag_based_gate_matches_the_key_based_gate() {
        let env = Env::default();

        in_contract(&env, || {
            for paused in [false, true] {
                set_paused(&env, &PAUSED, paused);
                assert_eq!(
                    require_not_paused(&env, &PAUSED),
                    require_not_paused_flag(paused)
                );
            }
        });
    }

    // ── Generic over the flag key ─────────────────────────────────────────────

    /// `DripStream` gates on a pause bit held inside its loaded `StreamInfo`
    /// rather than a standalone instance key, and the factory/oracle gate on
    /// `DataKey::Paused`. Both key shapes must work through the same helper,
    /// which is what the generic bound buys.
    #[test]
    fn gate_is_generic_over_the_flag_key() {
        let env = Env::default();
        let stream_key = TestKey::StreamAddr(7);

        in_contract(&env, || {
            assert_eq!(require_not_paused(&env, &stream_key), Ok(()));

            set_paused(&env, &stream_key, true);
            assert_eq!(
                require_not_paused(&env, &stream_key),
                Err(PausedError::ContractPaused)
            );

            // Two distinct keys hold independent flags.
            assert!(!is_paused(&env, &PAUSED));
        });
    }
}
