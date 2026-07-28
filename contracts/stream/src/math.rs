use soroban_sdk::Env;

use crate::errors::Error;
use crate::storage::StreamInfo;

/// Returns the total tokens that have streamed up to `now`,
/// excluding any paused time. Does not account for withdrawals.
///
/// The amount is `rate_per_second * elapsed`, where `elapsed` is the time
/// between `start_time` and an "effective now" that is clamped so pausing and
/// stream completion never inflate the result:
/// - If `now` is before `start_time`, nothing has streamed yet (`0`).
/// - If `end_time` is set and has passed, `effective now` is `end_time` —
///   streaming stops accruing once the stream is over.
/// - If the stream is currently paused, `effective now` is `paused_at`, the
///   timestamp the pause began — time spent paused never counts as elapsed,
///   so accrual freezes until `resume()` shifts `start_time` forward.
/// - Otherwise `effective now` is the current ledger time.
pub fn streamed_amount(env: &Env, info: &StreamInfo) -> Result<i128, Error> {
    let now = env.ledger().timestamp();

    // Stream has not started yet
    if now < info.start_time {
        return Ok(0);
    }

    // Clamp to end_time if set
    let effective_now = if info.end_time > 0 && now > info.end_time {
        info.end_time
    } else if info.is_paused() {
        info.paused_at
    } else {
        now
    };

    let elapsed = effective_now
        .checked_sub(info.start_time)
        .ok_or(Error::ArithmeticOverflow)?;

    (info.rate_per_second)
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
