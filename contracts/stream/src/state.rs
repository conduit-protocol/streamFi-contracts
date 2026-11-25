use soroban_sdk::{contracttype, panic_with_error, Env};

use crate::storage::{DataKey, MutableStreamState, StreamInfo, StreamMetadata};
#[allow(unused_imports)]
pub use crate::storage::StreamState;
use crate::Error;

/// Calculate stream age in seconds from start_time up to current_time (Issue #808).
#[allow(dead_code)]
pub fn elapsed_seconds(info: &StreamInfo, current_time: u64) -> u64 {
    info.elapsed_seconds(current_time)
}

/// Helper method to check if a stream's remaining balance is depleted (Issue #753).
#[allow(dead_code)]
pub fn is_depleted(state: &StreamState) -> bool {
    state.is_depleted()
}

/// True when the stream's schedule has completed by `current_time` (Issue #927).
#[allow(dead_code)]
pub fn is_fully_vested(info: &StreamInfo, current_time: u64) -> bool {
    info.is_fully_vested(current_time)
}

/// Load the full stream state in a single storage read.
///
/// Reads the consolidated `Config` key. All streams are now written with
/// the consolidated layout; pre-consolidation streams were migrated on their
/// first `save()` call (see `save()` for the one-time migration logic).
pub fn try_load(env: &Env) -> Result<StreamInfo, Error> {
    let s = env.storage().instance();
    if let (Some(metadata), Some(state)) = (
        s.get::<_, StreamMetadata>(&DataKey::Metadata),
        s.get::<_, MutableStreamState>(&DataKey::State),
    ) {
        return Ok(StreamInfo {
            sender: metadata.sender,
            recipient: state.recipient,
            token: metadata.token,
            rate_per_second: state.rate_per_second,
            start_time: metadata.start_time,
            end_time: state.end_time,
            withdrawn: state.withdrawn,
            paused_at: state.paused_at,
            flags: state.flags,
            event_sequence: state.event_sequence,
            cumulative_fees: state.cumulative_fees,
        });
    }
    s.get::<_, StreamInfo>(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

pub fn load(env: &Env) -> StreamInfo {
    match try_load(env) {
        Ok(info) => info,
        Err(err) => panic_with_error!(env, err),
    }
}

/// Persist the entire stream state in a single storage write.
///
/// All streams are now required to use the consolidated `Config` key.
/// Pre-consolidation streams were migrated on their first `save()` call
/// in a prior contract version (legacy keys were removed as part of the upgrade).
pub fn save(env: &Env, info: &StreamInfo) {
    let s = env.storage().instance();
    if !s.has(&DataKey::Metadata) {
        s.set(
            &DataKey::Metadata,
            &StreamMetadata {
                sender: info.sender.clone(),
                token: info.token.clone(),
                start_time: info.start_time,
            },
        );
    }
    s.set(
        &DataKey::State,
        &MutableStreamState {
            recipient: info.recipient.clone(),
            rate_per_second: info.rate_per_second,
            end_time: info.end_time,
            withdrawn: info.withdrawn,
            paused_at: info.paused_at,
            flags: info.flags,
            event_sequence: info.event_sequence,
            cumulative_fees: info.cumulative_fees,
        },
    );
    if s.has(&DataKey::Config) {
        s.remove(&DataKey::Config);
    }
}

pub fn assert_not_cancelled(info: &StreamInfo) -> Result<(), Error> {
    if info.is_cancelled() {
        Err(Error::StreamCancelled)
    } else {
        Ok(())
    }
}

/// Check if a stream is in a cancellable state (Issue #937).
///
/// Returns `true` if the stream can be cancelled:
/// - Stream is not already cancelled
/// - Stream has not been fully completed/withdrawn
///
/// The caller authorization (sender permissions, recipient restrictions) should be
/// validated separately at the contract method level using stream config and address.
pub fn is_cancellable(info: &StreamInfo) -> Result<bool, Error> {
    // Stream cannot be cancelled if it's already in a terminal cancelled state
    if info.is_cancelled() {
        return Ok(false);
    }

    // Stream can be cancelled as long as it's not fully cancelled
    // Other authorization checks (sender, recipient, etc.) are done at the method level
    Ok(info.is_in_cancellable_state())
}

/// Validate stream state transition from Paused to Cancelled.
///
/// Ensures that when a paused stream transitions to cancelled, the state remains consistent
/// and unvested remainder refund calculations are correct. This validates:
/// - Stream is currently paused
/// - Stream is not already cancelled
/// - Refund calculations will not overflow
/// - Cancelled state can be safely persisted
///
/// # Arguments
/// - `info` — The current stream state
/// - `current_time` — Current timestamp for calculating streamed amount
/// - `balance` — Current contract balance for the stream token
///
/// # Returns
/// - `Ok(())` if the transition is valid
/// - `Err(Error::NotPaused)` if the stream is not paused
/// - `Err(Error::StreamCancelled)` if the stream is already cancelled
/// - `Err(Error::ArithmeticOverflow)` if refund calculation overflows
pub fn validate_paused_to_cancelled_transition(
    info: &StreamInfo,
    current_time: u64,
    balance: i128,
) -> Result<(), Error> {
    // Stream must be in paused state to transition to cancelled
    if !info.is_paused() {
        return Err(Error::NotPaused);
    }

    // Stream must not already be cancelled
    if info.is_cancelled() {
        return Err(Error::StreamCancelled);
    }

    // Verify refund calculation will not overflow.
    // During cancellation:
    // 1. Calculate streamed amount (what has been earned)
    // 2. Calculate owed_to_recipient = max(0, streamed - withdrawn)
    // 3. Calculate refund_to_sender = max(0, balance - owed_to_recipient)
    // 4. Calculate total_withdrawn = withdrawn + owed_to_recipient (this must not overflow)

    let streamed = if info.rate_per_second > 0 && info.end_time > current_time {
        let elapsed = current_time.saturating_sub(info.start_time);
        (info.rate_per_second as u128)
            .saturating_mul(elapsed as u128)
            .min(i128::MAX as u128) as i128
    } else if info.end_time > 0 && current_time >= info.end_time {
        // Stream has ended or is fully vested
        let total_duration = info.end_time.saturating_sub(info.start_time);
        (info.rate_per_second as u128)
            .saturating_mul(total_duration as u128)
            .min(i128::MAX as u128) as i128
    } else {
        0
    };

    let owed_to_recipient = (streamed - info.withdrawn).max(0).min(balance);

    // Verify total_withdrawn calculation won't overflow
    info.withdrawn
        .checked_add(owed_to_recipient)
        .ok_or(Error::ArithmeticOverflow)?;

    Ok(())
}

/// Maximum allowed re-entrancy depth. Must never be > 1 in production —
/// Soroban's execution model makes re-entrancy structurally impossible,
/// so any depth > 0 signals a bug or an emerging cross-contract callback
/// capability. Kept as a named constant so a future protocol upgrade that
/// introduces hooks can be gated here rather than in every call site.
pub const MAX_REENTRANCY_DEPTH: u32 = 1;

/// Acquire the re-entrancy guard.
///
/// Uses a depth counter stored at [`DataKey::Guard`] instead of a boolean
/// flag. The counter approach:
///
/// - Detects unbalanced acquire / release (depth would underflow).
/// - Allows controlled re-entrancy up to [`MAX_REENTRANCY_DEPTH`] if the
///   protocol ever supports it.
/// - Prevents the "two simultaneous acquires both see 0" TOCTOU that a
///   plain boolean check-then-set is theoretically vulnerable to.
///
/// In Soroban's synchronous, single-shot execution model the TOCTOU is
/// not exploitable today; the counter is defense-in-depth for future
/// protocol capabilities (hooks, cross-contract callbacks, etc.).
pub fn lock(env: &Env) -> Result<(), Error> {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth >= MAX_REENTRANCY_DEPTH {
        return Err(Error::ReentrancyForbidden);
    }
    s.set(&DataKey::Guard, &(depth + 1));
    Ok(())
}

/// Release the re-entrancy guard.
///
/// Decrements the depth counter. If the counter is already zero (i.e.
/// `unlock` was called without a matching `lock`), the call is silently
/// swallowed — the state is already unlocked and panicking here would
/// unnecessarily block legitimate code paths.
pub fn unlock(env: &Env) {
    let s = env.storage().instance();
    let depth: u32 = s.get(&DataKey::Guard).unwrap_or(0);
    if depth > 0 {
        s.set(&DataKey::Guard, &(depth - 1));
    }
}

/// Execute `f` under the re-entrancy guard, releasing the lock afterwards.
///
/// This is the preferred entry point for all state-mutating contract
/// methods. It guarantees that the guard is released on every exit path —
/// early `return`, `?` propagation, or normal completion — without
/// requiring the caller to remember a manual `unlock()` call.
///
/// # Examples
///
/// ```ignore
/// pub fn withdraw(env: Env, amount: i128) -> Result<i128, Error> {
///     state::with_guard(&env, |env| Self::_withdraw(env, amount))
/// }
/// ```
pub fn with_guard<R>(env: &Env, f: impl FnOnce(&Env) -> Result<R, Error>) -> Result<R, Error> {
    lock(env)?;
    let result = f(env);
    unlock(env);
    result
}

/// Vesting trajectory topology for a stream (#883).
#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum VestingCurve {
    /// Uniform linear vesting over the stream duration.
    #[default]
    Linear,
    /// Front-loaded concave decay curve parameterized by basis-points decay coefficient (`decay_bps`).
    /// `0` reduces to linear vesting; larger values accelerate early vesting.
    ExponentialDecay(u32),
    /// Discrete step vesting unlocking in equal steps every `step_duration` seconds.
    Stepped(u64),
}

pub fn set_vesting_curve(env: &Env, curve: &VestingCurve) {
    env.storage().instance().set(&DataKey::VestingCurve, curve);
}

pub fn get_vesting_curve(env: &Env) -> VestingCurve {
    env.storage()
        .instance()
        .get(&DataKey::VestingCurve)
        .unwrap_or_default()
}

/// Overflow-safe `(a * b) / denom` for `u128` without intermediate overflow.
fn mul_div_u128(a: u128, b: u128, denom: u128) -> u128 {
    if denom == 0 || a == 0 || b == 0 {
        return 0;
    }
    if let Some(prod) = a.checked_mul(b) {
        return prod / denom;
    }
    let q = a / denom;
    let r = a % denom;
    let part1 = q.saturating_mul(b);
    let part2 = r.checked_mul(b).map(|p| p / denom).unwrap_or_else(|| {
        let q2 = b / denom;
        let r2 = b % denom;
        r.saturating_mul(q2)
            .saturating_add(r.saturating_mul(r2) / denom)
    });
    part1.saturating_add(part2)
}

/// Computes vested amount for a stream under the given [`VestingCurve`] (#883).
///
/// Guarantees `0 <= vested <= total_amount`, monotonicity in `elapsed`, and
/// overflow-free execution across extreme durations (`1` second up to `u64::MAX`).
pub fn vested_amount_for_curve(
    curve: &VestingCurve,
    total_amount: i128,
    elapsed: u64,
    duration: u64,
) -> Result<i128, Error> {
    if total_amount < 0 {
        return Err(Error::InvalidAmount);
    }
    if total_amount == 0 || elapsed == 0 {
        return Ok(0);
    }
    if duration == 0 || elapsed >= duration {
        return Ok(total_amount);
    }

    let total = total_amount as u128;
    let el = elapsed as u128;
    let dur = duration as u128;

    let vested = match *curve {
        VestingCurve::Linear => mul_div_u128(total, el, dur),
        VestingCurve::Stepped(step_duration) => {
            if step_duration == 0 {
                mul_div_u128(total, el, dur)
            } else {
                let total_steps = duration / step_duration;
                if total_steps == 0 {
                    0
                } else {
                    let completed_steps = (elapsed / step_duration).min(total_steps);
                    mul_div_u128(total, completed_steps as u128, total_steps as u128)
                }
            }
        }
        VestingCurve::ExponentialDecay(decay_bps) => {
            if decay_bps == 0 {
                mul_div_u128(total, el, dur)
            } else {
                const SCALE: u128 = 1_000_000;
                let x_ppm = mul_div_u128(el, SCALE, dur);
                let k = (decay_bps as u128).min(100_000) * 100;
                let num = (SCALE + k).saturating_mul(x_ppm);
                let den = SCALE
                    .saturating_mul(SCALE)
                    .saturating_add(k.saturating_mul(x_ppm));
                let ratio_ppm = mul_div_u128(num, SCALE, den).min(SCALE);
                mul_div_u128(total, ratio_ppm, SCALE)
            }
        }
    };

    Ok(vested.min(total) as i128)
}
