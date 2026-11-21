use soroban_sdk::{Env, U256};

use crate::errors::Error;
use crate::storage::{CliffConfig, DataKey, RateCheckpoint, StreamInfo};

/// Calculates `value * multiplier / divisor` without overflowing an i128
/// intermediate. Inputs and the result are non-negative.
pub fn mul_div(env: &Env, value: i128, multiplier: u128, divisor: u128) -> Result<i128, Error> {
    if value < 0 || divisor == 0 {
        return Err(Error::ArithmeticOverflow);
    }

    let result = U256::from_u128(env, value as u128)
        .mul(&U256::from_u128(env, multiplier))
        .div(&U256::from_u128(env, divisor))
        .to_u128()
        .and_then(|value| i128::try_from(value).ok())
        .ok_or(Error::ArithmeticOverflow)?;
    Ok(result)
}

/// Returns the total tokens that have streamed up to `now`,
/// excluding any paused time. Does not account for withdrawals.
///
/// The amount is `rate_per_second * elapsed`, where `elapsed` is the time
/// between `start_time` and an "effective now" that is clamped so pausing and
/// stream completion never inflate the result:
/// - If `now` is before `start_time`, nothing has streamed yet (`0`).
/// - If the stream is currently paused, `effective now` is the later of
///   `paused_at` (the timestamp the pause began — time spent paused never
///   counts as elapsed) and, for a bounded stream, `end_time`. Pausing is
///   checked **before** the `end_time` clamp so a stream the sender left
///   paused cannot silently accrue the full contracted amount once ledger
///   time passes the original `end_time`.
/// - If `end_time` is set and has passed, `effective now` is `end_time` —
///   streaming stops accruing once the stream is over.
/// - Otherwise `effective now` is the current ledger time.
pub fn streamed_amount(env: &Env, info: &StreamInfo) -> Result<i128, Error> {
    let now = env.ledger().timestamp();

    // Stream has not started yet
    if now < info.start_time {
        return Ok(0);
    }

    // A paused stream freezes accrual at the pause, no matter how much
    // wall-clock time elapses afterwards — even past end_time. This is
    // guarded before the end_time clamp so a never-resumed stream cannot
    // inflate to the full contracted amount once its end_time passes.
    let effective_now = if info.is_paused() {
        if info.end_time > 0 && info.paused_at > info.end_time {
            info.end_time
        } else {
            info.paused_at
        }
    } else if info.end_time > 0 && now > info.end_time {
        info.end_time
    } else {
        now
    };

    // A rate change checkpoints all value accrued under the previous rate.
    // This makes the new rate prospective: previously earned value never gets
    // retroactively repriced. The checkpoint is optional so pre-upgrade streams
    // continue to use the original start/cliff calculation until their first
    // rate change.
    let checkpoint: Option<RateCheckpoint> = env.storage().instance().get(&DataKey::RateCheckpoint);
    if let Some(checkpoint) = checkpoint {
        if effective_now < checkpoint.at {
            return Ok(0);
        }
        let elapsed = effective_now
            .checked_sub(checkpoint.at)
            .ok_or(Error::ArithmeticOverflow)?;
        let linear = mul_div(env, info.rate_per_second, elapsed as u128, 1)?;
        return checkpoint
            .accrued
            .checked_add(linear)
            .ok_or(Error::ArithmeticOverflow);
    }

    // Check for optional cliff config (Issue #719)
    let cliff_cfg: Option<CliffConfig> = env.storage().instance().get(&DataKey::CliffConfig);
    if let Some(cfg) = cliff_cfg {
        if effective_now < cfg.cliff_time {
            return Ok(0);
        }
        let elapsed = effective_now
            .checked_sub(cfg.cliff_time)
            .ok_or(Error::ArithmeticOverflow)?;
        let linear = mul_div(env, info.rate_per_second, elapsed as u128, 1)?;
        return cfg
            .cliff_unlock_amount
            .checked_add(linear)
            .ok_or(Error::ArithmeticOverflow);
    }

    let elapsed = effective_now
        .checked_sub(info.start_time)
        .ok_or(Error::ArithmeticOverflow)?;

    mul_div(env, info.rate_per_second, elapsed as u128, 1)
}

/// Calculates linear streamed amount: `rate_per_second * elapsed`, returning `Error::ArithmeticOverflow` on overflow (Issue #681).
/// Optimized to prevent precision loss by checking for zero cases first (Issue #784).
#[allow(dead_code)]
pub fn calculate_streamed_amount(rate_per_second: i128, elapsed: u64) -> Result<i128, Error> {
    if rate_per_second == 0 || elapsed == 0 {
        return Ok(0);
    }
    rate_per_second
        .checked_mul(elapsed as i128)
        .ok_or(Error::ArithmeticOverflow)
}

/// Safely calculates rate × duration with saturating arithmetic to prevent overflow (Issue #749).
/// Saturates at i128::MAX if the result exceeds the maximum value.
#[allow(dead_code)]
pub fn rate_duration_saturating(rate_per_second: i128, duration: u64) -> i128 {
    if rate_per_second == 0 || duration == 0 {
        return 0;
    }
    rate_per_second.saturating_mul(duration as i128)
}

/// Calculates exponential decay streamed amount based on half-life (Issue #804).
///
/// Formula: tokens unlock following an exponential decay schedule where half the remaining
/// unstreamed amount unlocks every `half_life` seconds.
/// - At `elapsed == 0`: returns 0
/// - At `elapsed == half_life`: returns `initial_amount / 2`
/// - At `elapsed == 2 * half_life`: returns `initial_amount * 3 / 4`
/// - As `elapsed -> inf`: approaches `initial_amount`
#[allow(dead_code)]
pub fn calculate_decay_stream(
    initial_amount: i128,
    elapsed: u64,
    half_life: u64,
) -> Result<i128, Error> {
    if half_life == 0 {
        return Err(Error::InvalidTimeRange);
    }
    if initial_amount < 0 {
        return Err(Error::InvalidAmount);
    }
    if initial_amount == 0 || elapsed == 0 {
        return Ok(0);
    }

    let halvings = elapsed / half_life;
    if halvings >= 127 {
        return Ok(initial_amount);
    }

    let base_remaining = initial_amount >> halvings;
    let rem = elapsed % half_life;

    let decay_in_interval = if rem == 0 {
        0
    } else {
        let divisor = 2 * (half_life as i128);
        match base_remaining.checked_mul(rem as i128) {
            Some(prod) => prod / divisor,
            None => {
                let q = base_remaining / divisor;
                let r = base_remaining % divisor;
                q.saturating_mul(rem as i128)
                    .saturating_add((r.saturating_mul(rem as i128)) / divisor)
            }
        }
    };

    let remaining = base_remaining.saturating_sub(decay_in_interval);
    let streamed = initial_amount.saturating_sub(remaining);
    Ok(streamed.min(initial_amount))
}

/// Returns tokens available for the recipient to withdraw right now.
///
/// Computed as `streamed_amount(env, info) - info.withdrawn`, so it inherits
/// the same pausing/completion clamping as [`streamed_amount`]: while a
/// stream is paused, `streamed_amount` stops growing and `withdrawable`
/// stays flat until the stream is resumed.
///
/// Guards against the case where `info.withdrawn` could theoretically
/// exceed `streamed` (e.g. if ledger time skews or a top-up changes
/// effective balance) by clamping to zero rather than returning an error.
pub fn withdrawable(env: &Env, info: &StreamInfo) -> Result<i128, Error> {
    let streamed = streamed_amount(env, info)?;
    let available = streamed.saturating_sub(info.withdrawn);
    Ok(available.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_streamed_amount_zero_elapsed_or_rate() {
        assert_eq!(calculate_streamed_amount(0, 100).unwrap(), 0);
        assert_eq!(calculate_streamed_amount(500, 0).unwrap(), 0);
        assert_eq!(calculate_streamed_amount(0, 0).unwrap(), 0);
    }

    #[test]
    fn test_calculate_streamed_amount_normal_ranges() {
        assert_eq!(calculate_streamed_amount(10, 5).unwrap(), 50);
        assert_eq!(
            calculate_streamed_amount(1_000_000, 3600).unwrap(),
            3_600_000_000
        );
    }

    #[test]
    fn test_calculate_streamed_amount_maximum_boundary_conditions() {
        // i128::MAX with elapsed 1 is exact i128::MAX
        assert_eq!(calculate_streamed_amount(i128::MAX, 1).unwrap(), i128::MAX);

        // i128::MAX with elapsed > 1 overflows safely
        assert_eq!(
            calculate_streamed_amount(i128::MAX, 2),
            Err(Error::ArithmeticOverflow)
        );

        // Boundary division: half of i128::MAX
        let half_max = i128::MAX / 2;
        assert_eq!(
            calculate_streamed_amount(half_max, 2).unwrap(),
            half_max * 2
        );
        assert_eq!(
            calculate_streamed_amount(half_max, 3),
            Err(Error::ArithmeticOverflow)
        );

        // Max elapsed u64::MAX with rate > 1 overflows
        assert_eq!(
            calculate_streamed_amount(i128::MAX, u64::MAX),
            Err(Error::ArithmeticOverflow)
        );
    }

    #[test]
    fn test_calculate_streamed_amount_fuzz_parameterized_ranges() {
        // Pseudo-random / deterministic sweep across powers of 2 and extreme scales
        let rates: [i128; 8] = [
            1,
            1_000,
            1_000_000_000,
            1_000_000_000_000_000_000,
            1 << 60,
            1 << 90,
            1 << 120,
            i128::MAX,
        ];

        let elapsed_samples: [u64; 8] = [
            1,
            10,
            60,
            3600,
            86400,
            31_536_000, // 1 year
            u32::MAX as u64,
            u64::MAX,
        ];

        for &rate in &rates {
            for &elapsed in &elapsed_samples {
                let res = calculate_streamed_amount(rate, elapsed);
                // Verify that if checked_mul would overflow, calculate_streamed_amount returns ArithmeticOverflow
                match rate.checked_mul(elapsed as i128) {
                    Some(expected) => assert_eq!(res.unwrap(), expected),
                    None => assert_eq!(res, Err(Error::ArithmeticOverflow)),
                }
            }
        }
    }

    #[test]
    fn test_rate_duration_saturating_zero_cases() {
        // Zero rate or duration returns 0
        assert_eq!(rate_duration_saturating(0, 100), 0);
        assert_eq!(rate_duration_saturating(500, 0), 0);
        assert_eq!(rate_duration_saturating(0, 0), 0);
    }

    #[test]
    fn test_rate_duration_saturating_normal_ranges() {
        // Normal ranges compute correctly
        assert_eq!(rate_duration_saturating(10, 5), 50);
        assert_eq!(rate_duration_saturating(1_000_000, 3600), 3_600_000_000);
    }

    #[test]
    fn test_rate_duration_saturating_overflow_behavior() {
        // Overflow saturates to i128::MAX instead of panicking
        assert_eq!(rate_duration_saturating(i128::MAX, 2), i128::MAX);
        assert_eq!(rate_duration_saturating(i128::MAX, u64::MAX), i128::MAX);
        assert_eq!(rate_duration_saturating(1i128 << 70, u64::MAX), i128::MAX);
    }

    #[test]
    fn test_rate_duration_saturating_boundary() {
        // Test exact boundary at i128::MAX
        assert_eq!(rate_duration_saturating(i128::MAX, 1), i128::MAX);

        // Test half of max
        let half_max = i128::MAX / 2;
        assert_eq!(
            rate_duration_saturating(half_max, 2),
            half_max.saturating_mul(2)
        );
        // With 3, it would overflow to max
        assert_eq!(rate_duration_saturating(half_max, 3), i128::MAX);
    }

    #[test]
    fn test_calculate_decay_stream_validations() {
        // Zero half_life returns InvalidTimeRange
        assert_eq!(
            calculate_decay_stream(1000, 10, 0),
            Err(Error::InvalidTimeRange)
        );

        // Negative initial amount returns InvalidAmount
        assert_eq!(
            calculate_decay_stream(-100, 10, 60),
            Err(Error::InvalidAmount)
        );

        // Zero initial amount or zero elapsed returns 0
        assert_eq!(calculate_decay_stream(0, 100, 60).unwrap(), 0);
        assert_eq!(calculate_decay_stream(1000, 0, 60).unwrap(), 0);
    }

    #[test]
    fn test_calculate_decay_stream_half_lives() {
        let initial = 1_000_000;
        let half_life = 3600;

        // At 0 seconds: 0 streamed
        assert_eq!(calculate_decay_stream(initial, 0, half_life).unwrap(), 0);

        // At 1 half-life (3600s): exactly 50% streamed
        assert_eq!(
            calculate_decay_stream(initial, half_life, half_life).unwrap(),
            500_000
        );

        // At 2 half-lives (7200s): 75% streamed
        assert_eq!(
            calculate_decay_stream(initial, half_life * 2, half_life).unwrap(),
            750_000
        );

        // At 3 half-lives: 87.5% streamed
        assert_eq!(
            calculate_decay_stream(initial, half_life * 3, half_life).unwrap(),
            875_000
        );
    }

    #[test]
    fn test_calculate_decay_stream_monotonic_interpolation() {
        let initial = 10_000;
        let half_life = 100;

        let mut prev = 0;
        for t in 0..=300 {
            let streamed = calculate_decay_stream(initial, t, half_life).unwrap();
            assert!(streamed >= prev, "streamed amount must be monotonic");
            assert!(streamed <= initial, "streamed cannot exceed initial amount");
            prev = streamed;
        }
    }

    #[test]
    fn fixed_point_withdrawal_without_cumulative_precision_loss() {
        let env = Env::default();
        // 100,000 U256 operations exceed the default per-invocation budget.
        env.budget().reset_unlimited();
        // 2^53 + 1 is the first integer a f64 cannot represent exactly.
        let rate: i128 = 9_007_199_254_740_993;
        let steps: u64 = 100_000;

        let mut withdrawn_fixed: i128 = 0;
        let mut previous_fixed: i128 = 0;
        let mut previous_float: f64 = 0.0;

        for step in 1..=steps {
            let vested_fixed = mul_div(&env, rate, step as u128, 1).unwrap();
            let vested_float = rate as f64 * step as f64;

            withdrawn_fixed += vested_fixed - previous_fixed;

            previous_fixed = vested_fixed;
            previous_float = vested_float;
        }

        // The u128 fixed-point path is exact: every withdrawal is the exact
        // delta, so the cumulative total equals `rate * steps` with no drift.
        assert_eq!(withdrawn_fixed, rate * steps as i128);
        // The floating-point approximation cannot represent the final vested
        // amount exactly, demonstrating why the contract must not use it.
        assert_ne!(previous_fixed as f64, previous_float);
    }

    #[test]
    fn test_calculate_decay_stream_extreme_elapsed_and_amounts() {
        let initial = 1_000_000_000_000_000_000_i128;
        let half_life = 100;

        // Long elapsed time saturates to initial amount
        let streamed = calculate_decay_stream(initial, 100 * 200, half_life).unwrap();
        assert_eq!(streamed, initial);

        // u64::MAX elapsed time saturates without overflow or panic
        let max_streamed = calculate_decay_stream(initial, u64::MAX, half_life).unwrap();
        assert_eq!(max_streamed, initial);
    }

    #[test]
    fn rounding_extremes_never_exceed_deposit() {
        let rates: [i128; 8] = [
            1,
            2,
            7,
            1_000_000,
            1_i128 << 32,
            1_i128 << 64,
            i128::MAX / 10_000,
            i128::MAX / 2,
        ];
        let durations: [u64; 8] = [1, 2, 3, 59, 60, 3_600, 86_400, 1_000_000];

        for rate in rates {
            for duration in durations {
                match rate.checked_mul(duration as i128) {
                    Some(deposit) => {
                        for elapsed in [0, 1, duration / 2, duration] {
                            let accrued = calculate_streamed_amount(rate, elapsed).unwrap();
                            assert!(accrued <= deposit);
                            assert!(accrued >= 0);
                        }
                    }
                    None => {
                        assert_eq!(
                            calculate_streamed_amount(rate, duration),
                            Err(Error::ArithmeticOverflow)
                        );
                    }
                }
            }
        }
    }
}

/// Rounding direction for [`interpolate_vested`] (#879).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingMode {
    RoundUp,
    RoundDown,
}

/// Linear vesting `total * elapsed / duration` with an explicit rounding
/// mode, overflow free for any `i128` total and `u64` times (#879).
///
/// `elapsed` is clamped to `duration`, so at or after completion the result
/// is exactly `total`, never more, and no rounding dust is left behind. A
/// zero `duration` counts as fully vested; a non-positive `total` vests
/// nothing.
#[allow(dead_code)]
pub fn interpolate_vested(
    elapsed: u64,
    duration: u64,
    total: i128,
    rounding: RoundingMode,
) -> i128 {
    if total <= 0 {
        return 0;
    }
    if duration == 0 || elapsed >= duration {
        return total;
    }

    let (total, elapsed, duration) = (total as u128, elapsed as u128, duration as u128);
    // Split total = q * duration + r so neither product can overflow:
    // q * elapsed <= total, and r * elapsed < duration^2 < 2^128.
    let (q, r) = (total / duration, total % duration);
    let partial = r * elapsed;
    let mut vested = q * elapsed + partial / duration;
    if rounding == RoundingMode::RoundUp && partial % duration != 0 {
        vested += 1;
    }
    vested as i128
}

#[cfg(test)]
mod interpolate_tests {
    use super::*;

    #[test]
    fn exact_division_is_the_same_in_both_modes() {
        for mode in [RoundingMode::RoundDown, RoundingMode::RoundUp] {
            assert_eq!(interpolate_vested(25, 100, 1_000, mode), 250);
        }
    }

    #[test]
    fn modes_differ_by_one_on_a_fraction() {
        // 10 * 1 / 3 = 3.33...
        assert_eq!(interpolate_vested(1, 3, 10, RoundingMode::RoundDown), 3);
        assert_eq!(interpolate_vested(1, 3, 10, RoundingMode::RoundUp), 4);
    }

    #[test]
    fn completion_returns_exactly_the_total() {
        for mode in [RoundingMode::RoundDown, RoundingMode::RoundUp] {
            assert_eq!(interpolate_vested(7, 7, 10, mode), 10);
            assert_eq!(interpolate_vested(u64::MAX, 7, 10, mode), 10);
        }
    }

    #[test]
    fn edge_inputs() {
        assert_eq!(interpolate_vested(5, 0, 10, RoundingMode::RoundDown), 10);
        assert_eq!(interpolate_vested(0, 100, 10, RoundingMode::RoundUp), 0);
        assert_eq!(interpolate_vested(50, 100, 0, RoundingMode::RoundUp), 0);
        assert_eq!(interpolate_vested(50, 100, -10, RoundingMode::RoundUp), 0);
    }

    #[test]
    fn does_not_overflow_at_the_extremes() {
        let half = u64::MAX / 2;
        let down = interpolate_vested(half, u64::MAX, i128::MAX, RoundingMode::RoundDown);
        let up = interpolate_vested(half, u64::MAX, i128::MAX, RoundingMode::RoundUp);
        assert!(down > 0 && down <= i128::MAX / 2);
        assert!(up == down || up == down + 1);
        let near_end = interpolate_vested(u64::MAX - 1, u64::MAX, i128::MAX, RoundingMode::RoundUp);
        assert!(near_end < i128::MAX);
        assert_eq!(
            interpolate_vested(u64::MAX, u64::MAX, i128::MAX, RoundingMode::RoundUp),
            i128::MAX
        );
    }

    #[test]
    fn micro_schedule_withdrawals_sum_to_the_principal() {
        // 7 units over 3 seconds, claimed every second: increments of the
        // vested curve add back to the full deposit with no dust.
        for mode in [RoundingMode::RoundDown, RoundingMode::RoundUp] {
            let mut claimed = 0;
            let mut previous = 0;
            for t in 1..=3 {
                let vested = interpolate_vested(t, 3, 7, mode);
                assert!(vested >= previous);
                claimed += vested - previous;
                previous = vested;
            }
            assert_eq!(claimed, 7);
        }
    }

    #[test]
    fn round_down_never_exceeds_round_up() {
        for elapsed in 0..=13 {
            let down = interpolate_vested(elapsed, 13, 1_000_003, RoundingMode::RoundDown);
            let up = interpolate_vested(elapsed, 13, 1_000_003, RoundingMode::RoundUp);
            assert!(down <= up && up - down <= 1);
        }
    }
}
