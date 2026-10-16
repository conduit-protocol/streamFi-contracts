use soroban_sdk::Env;

use crate::errors::Error;
use crate::storage::{CliffConfig, DataKey, RateCheckpoint, StreamInfo};

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
        let linear = info
            .rate_per_second
            .checked_mul(elapsed as i128)
            .ok_or(Error::ArithmeticOverflow)?;
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
        let linear = (info.rate_per_second)
            .checked_mul(elapsed as i128)
            .ok_or(Error::ArithmeticOverflow)?;
        return cfg
            .cliff_unlock_amount
            .checked_add(linear)
            .ok_or(Error::ArithmeticOverflow);
    }

    let elapsed = effective_now
        .checked_sub(info.start_time)
        .ok_or(Error::ArithmeticOverflow)?;

    calculate_streamed_amount(info.rate_per_second, elapsed)
}

/// Calculates linear streamed amount: `rate_per_second * elapsed`, returning `Error::ArithmeticOverflow` on overflow (Issue #681).
pub fn calculate_streamed_amount(rate_per_second: i128, elapsed: u64) -> Result<i128, Error> {
    rate_per_second
        .checked_mul(elapsed as i128)
        .ok_or(Error::ArithmeticOverflow)
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
        assert_eq!(calculate_streamed_amount(1_000_000, 3600).unwrap(), 3_600_000_000);
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
        assert_eq!(calculate_streamed_amount(half_max, 2).unwrap(), half_max * 2);
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
}
