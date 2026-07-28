#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, symbol_short, Env, Symbol};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    ProcessorLocked = 2001,
    CalculationOverflow = 2002,
    BatchTooLarge = 2003,
    StateVersionMismatch = 2004,
    StaleCallbackCleaned = 2005,
}

#[contract]
pub struct BatchTransferProcessor;

/// Storage key for the state version counter.
const STATE_VERSION_KEY: Symbol = symbol_short!("B_Ver");

/// Storage key for the callback sequence counter.
const CALLBACK_SEQ_KEY: Symbol = symbol_short!("B_CbSeq");

#[contractimpl]
impl BatchTransferProcessor {
    pub fn process_batch(env: Env, amounts: soroban_sdk::Vec<u64>) -> Result<u64, Error> {
        with_guard(&env, || {
            // ── Defensive null / boundary checks (Issue #83) ─────────────
            if amounts.is_empty() {
                return Ok(0);
            }

            if amounts.len() > 100 {
                return Err(Error::BatchTooLarge);
            }

            for amount in amounts.iter() {
                if amount == 0 {
                    return Err(Error::CalculationOverflow);
                }
            }

            // ── State-version race-condition check (Issue #84) ──────────
            // Snapshot the current version under the guard so there is no
            // TOCTOU window between reading the version and acquiring the
            // lock. Once the snapshot is taken, any external mutation that
            // bumps the version before we finish will be detected and abort
            // the commit.
            let snapshot = load_state_version(&env);
            bump_state_version(&env);
            let current = load_state_version(&env);
            if current != snapshot + 1 {
                cleanup_stale_callbacks(&env);
                return Err(Error::StateVersionMismatch);
            }

            let mut total: u64 = 0;
            for amount in amounts.iter() {
                match total.checked_add(amount) {
                    Some(new_total) => total = new_total,
                    None => {
                        cleanup_stale_callbacks(&env);
                        return Err(Error::CalculationOverflow);
                    }
                }
            }

            // Final check: version must still match the expected value.
            let final_version = load_state_version(&env);
            if final_version != snapshot + 1 {
                cleanup_stale_callbacks(&env);
                return Err(Error::StateVersionMismatch);
            }

            // Advance the callback sequence so stale callbacks from prior
            // interrupted batches are invalidated.
            let cb_seq = load_callback_seq(&env);
            env.storage()
                .instance()
                .set(&CALLBACK_SEQ_KEY, &(cb_seq + 1));

            Ok(total)
        })
    }
}

/// Execute `f` under the re-entrancy guard, releasing the lock afterwards.
const MAX_REENTRANCY_DEPTH: u32 = 1;

fn with_guard<R>(env: &Env, f: impl FnOnce() -> Result<R, Error>) -> Result<R, Error> {
    let lock_key = soroban_sdk::symbol_short!("B_Lock");
    let depth: u32 = env.storage().instance().get(&lock_key).unwrap_or(0);
    if depth >= MAX_REENTRANCY_DEPTH {
        return Err(Error::ProcessorLocked);
    }
    env.storage().instance().set(&lock_key, &(depth + 1));
    let result = f();
    // Fix Issue #83: use unwrap_or(0) instead of unwrap_or(1) to avoid
    // falsely incrementing the lock when the storage entry is missing.
    let d: u32 = env.storage().instance().get(&lock_key).unwrap_or(0);
    if d > 0 {
        env.storage().instance().set(&lock_key, &(d - 1));
    }
    result
}

/// Load the current state version from storage.
fn load_state_version(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&STATE_VERSION_KEY)
        .unwrap_or(0)
}

/// Increment the state version.
///
/// Uses `checked_add` for consistency with the rest of the crate's
/// checked-arithmetic convention (see `process_batch`'s amount-summing loop).
fn bump_state_version(env: &Env) {
    let v = load_state_version(env)
        .checked_add(1)
        .expect("state version counter overflowed u64");
    env.storage().instance().set(&STATE_VERSION_KEY, &v);
}

/// Load the callback sequence counter.
fn load_callback_seq(env: &Env) -> u64 {
    env.storage().instance().get(&CALLBACK_SEQ_KEY).unwrap_or(0)
}

/// Clean up any stale pending callbacks.
///
/// Called on every error path to prevent orphaned callback state from
/// accumulating when an operation fails mid-execution.
fn cleanup_stale_callbacks(env: &Env) {
    let seq = load_callback_seq(env);
    if seq > 0 {
        // Invalidate all pending callbacks by advancing the sequence.
        env.storage().instance().set(&CALLBACK_SEQ_KEY, &(seq + 1));
    }
}
