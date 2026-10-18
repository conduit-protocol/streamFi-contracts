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

use soroban_sdk::{contracttype, Env, TryFromVal, Val};

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

/// Detailed pause state for emergency pause.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseState {
    pub paused: bool,
    pub paused_at: u64,
    pub max_duration: Option<u64>,
    pub unpaused_at: Option<u64>,
    pub unpause_cooldown: Option<u64>,
}

/// Reads the emergency-pause flag stored under `paused_key` in `instance()`
/// storage.
///
/// Defaults to `false` when the key was never written, so a contract deployed
/// before the pause feature existed is treated as running normally rather than
/// halting every call.
///
/// If a pause duration limit (`max_duration`) was configured and the current
/// ledger timestamp exceeds `paused_at + max_duration`, the pause is considered
/// automatically expired and this returns `false`.
pub fn is_paused<K: StorageKey>(env: &Env, paused_key: &K) -> bool {
    let storage = env.storage().instance();
    let Some(raw_val) = storage.get::<K, Val>(paused_key) else {
        return false;
    };
    if let Ok(b) = bool::try_from_val(env, &raw_val) {
        return b;
    }
    if let Ok(state) = PauseState::try_from_val(env, &raw_val) {
        if !state.paused {
            return false;
        }
        if let Some(max_dur) = state.max_duration {
            let now = env.ledger().timestamp();
            if now > state.paused_at.saturating_add(max_dur) {
                return false;
            }
        }
        return true;
    }
    false
}

/// Writes the emergency-pause flag to `instance()` storage with optional duration and cooldown.
/// Returns `Ok(())` if the pause state was set successfully.
/// Returns `Err(PausedError::ContractPaused)` if unpause is in cooldown period.
pub fn set_paused_with_duration<K: StorageKey>(
    env: &Env,
    paused_key: &K,
    paused: bool,
    max_duration: Option<u64>,
) -> Result<(), PausedError> {
    set_paused_with_cooldown(env, paused_key, paused, max_duration, None)
}

/// Writes the emergency-pause flag with duration limit and unpause cooldown period.
///
/// When `unpause_cooldown` is set, rapidly re-pausing (toggling) is prevented by requiring
/// `now >= last_unpaused_at + cooldown_seconds` before pausing again.
pub fn set_paused_with_cooldown<K: StorageKey>(
    env: &Env,
    paused_key: &K,
    paused: bool,
    max_duration: Option<u64>,
    unpause_cooldown: Option<u64>,
) -> Result<(), PausedError> {
    let now = env.ledger().timestamp();
    let current_state: Option<PauseState> = env.storage().instance().get(paused_key);

    // Check unpause cooldown if pausing and previous unpaused_at is recent
    if paused {
        if let Some(current) = &current_state {
            if !current.paused {
                // We're currently unpaused; check if we're still in cooldown
                if let Some(last_unpaused) = current.unpaused_at {
                    if let Some(cooldown) = unpause_cooldown {
                        if now <= last_unpaused.saturating_add(cooldown) {
                            return Err(PausedError::ContractPaused);
                        }
                    }
                }
            }
        }
    }

    let state = PauseState {
        paused,
        paused_at: if paused { now } else { 0 },
        max_duration: if paused { max_duration } else { None },
        unpaused_at: if !paused && current_state.is_some() {
            Some(now)
        } else {
            None
        },
        unpause_cooldown,
    };
    env.storage().instance().set(paused_key, &state);
    Ok(())
}

/// Writes the emergency-pause flag to `instance()` storage without a duration limit.
///
/// Performs no authorization and no state-transition validation: the caller
/// (the contract's own `pause`/`unpause`) owns both, and is responsible for
/// emitting the matching event and bumping TTL.
pub fn set_paused<K: StorageKey>(
    env: &Env,
    paused_key: &K,
    paused: bool,
) -> Result<(), PausedError> {
    set_paused_with_duration(env, paused_key, paused, None)
}

/// Returns the current `PauseState` stored under `paused_key`, if any.
pub fn get_pause_state<K: StorageKey>(env: &Env, paused_key: &K) -> Option<PauseState> {
    let storage = env.storage().instance();
    let raw_val = storage.get::<K, Val>(paused_key)?;
    if let Ok(state) = PauseState::try_from_val(env, &raw_val) {
        Some(state)
    } else if let Ok(b) = bool::try_from_val(env, &raw_val) {
        Some(PauseState {
            paused: b,
            paused_at: 0,
            max_duration: None,
            unpaused_at: None,
            unpause_cooldown: None,
        })
    } else {
        None
    }
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
    use soroban_sdk::testutils::Ledger as _;
    use soroban_sdk::{contract, contractimpl, contracttype};

    #[contract]
    struct PauseTestHost;

    #[contractimpl]
    impl PauseTestHost {
        pub fn noop(_env: Env) {}
    }

    fn in_contract<T>(env: &Env, f: impl FnOnce() -> T) -> T {
        env.as_contract(&env.register_contract(None, PauseTestHost), f)
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestKey {
        Paused,
        StreamAddr(u64),
    }

    const PAUSED: TestKey = TestKey::Paused;

    #[test]
    fn unset_flag_reads_as_not_paused() {
        let env = Env::default();

        in_contract(&env, || {
            assert!(!is_paused(&env, &PAUSED));
        });
    }

    #[test]
    fn set_paused_round_trips_and_clears() {
        let env = Env::default();

        in_contract(&env, || {
            assert!(set_paused(&env, &PAUSED, true).is_ok());
            assert!(is_paused(&env, &PAUSED));

            assert!(set_paused(&env, &PAUSED, false).is_ok());
            assert!(!is_paused(&env, &PAUSED));
        });
    }

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
            set_paused(&env, &PAUSED, true).ok();
            assert_eq!(
                require_not_paused(&env, &PAUSED),
                Err(PausedError::ContractPaused)
            );
        });
    }

    #[test]
    fn flag_based_gate_matches_the_key_based_gate() {
        let env = Env::default();

        in_contract(&env, || {
            for paused in [false, true] {
                set_paused(&env, &PAUSED, paused).ok();
                assert_eq!(
                    require_not_paused(&env, &PAUSED),
                    require_not_paused_flag(paused)
                );
            }
        });
    }

    #[test]
    fn gate_is_generic_over_the_flag_key() {
        let env = Env::default();
        let stream_key = TestKey::StreamAddr(7);

        in_contract(&env, || {
            assert_eq!(require_not_paused(&env, &stream_key), Ok(()));

            set_paused(&env, &stream_key, true).ok();
            assert_eq!(
                require_not_paused(&env, &stream_key),
                Err(PausedError::ContractPaused)
            );

            assert!(!is_paused(&env, &PAUSED));
        });
    }

    #[test]
    fn pause_with_duration_automatically_unpauses_after_expiry() {
        let env = Env::default();
        let max_duration = 3600;

        in_contract(&env, || {
            env.ledger().set_timestamp(1000);

            assert!(set_paused_with_duration(&env, &PAUSED, true, Some(max_duration)).is_ok());
            assert!(is_paused(&env, &PAUSED));
            assert_eq!(
                require_not_paused(&env, &PAUSED),
                Err(PausedError::ContractPaused)
            );

            let state = get_pause_state(&env, &PAUSED).unwrap();
            assert!(state.paused);
            assert_eq!(state.paused_at, 1000);
            assert_eq!(state.max_duration, Some(max_duration));

            env.ledger().set_timestamp(4600);
            assert!(is_paused(&env, &PAUSED));

            env.ledger().set_timestamp(4601);
            assert!(!is_paused(&env, &PAUSED));
            assert_eq!(require_not_paused(&env, &PAUSED), Ok(()));
        });
    }

    #[test]
    fn unpause_clears_duration_and_paused_state() {
        let env = Env::default();

        in_contract(&env, || {
            env.ledger().set_timestamp(1000);
            assert!(set_paused_with_duration(&env, &PAUSED, true, Some(60)).is_ok());
            assert!(is_paused(&env, &PAUSED));

            assert!(set_paused(&env, &PAUSED, false).is_ok());
            assert!(!is_paused(&env, &PAUSED));
            assert_eq!(require_not_paused(&env, &PAUSED), Ok(()));

            let state = get_pause_state(&env, &PAUSED).unwrap();
            assert!(!state.paused);
            assert_eq!(state.paused_at, 0);
            assert_eq!(state.max_duration, None);
        });
    }

    #[test]
    fn unpause_cooldown_prevents_rapid_re_pause() {
        let env = Env::default();
        let cooldown = 300;

        in_contract(&env, || {
            env.ledger().set_timestamp(1000);

            assert!(set_paused_with_cooldown(&env, &PAUSED, true, None, Some(cooldown)).is_ok());
            assert!(is_paused(&env, &PAUSED));

            env.ledger().set_timestamp(2000);
            assert!(set_paused_with_cooldown(&env, &PAUSED, false, None, Some(cooldown)).is_ok());
            assert!(!is_paused(&env, &PAUSED));

            env.ledger().set_timestamp(2100);
            let result = set_paused_with_cooldown(&env, &PAUSED, true, None, Some(cooldown));
            assert_eq!(result, Err(PausedError::ContractPaused));
            assert!(!is_paused(&env, &PAUSED));

            env.ledger().set_timestamp(2300);
            let result = set_paused_with_cooldown(&env, &PAUSED, true, None, Some(cooldown));
            assert_eq!(result, Err(PausedError::ContractPaused));

            env.ledger().set_timestamp(2301);
            assert!(set_paused_with_cooldown(&env, &PAUSED, true, None, Some(cooldown)).is_ok());
            assert!(is_paused(&env, &PAUSED));
        });
    }

    #[test]
    fn legacy_boolean_storage_compatibility() {
        let env = Env::default();

        in_contract(&env, || {
            env.storage().instance().set(&PAUSED, &true);
            assert!(is_paused(&env, &PAUSED));
            assert_eq!(
                require_not_paused(&env, &PAUSED),
                Err(PausedError::ContractPaused)
            );

            env.storage().instance().set(&PAUSED, &false);
            assert!(!is_paused(&env, &PAUSED));
            assert_eq!(require_not_paused(&env, &PAUSED), Ok(()));
        });
    }
}
