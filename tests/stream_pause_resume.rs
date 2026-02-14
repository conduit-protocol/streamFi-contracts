//! Tests for pause / resume behaviour.

#[cfg(test)]
mod stream_pause_resume {
    #[test]
    fn test_pause_freezes_withdrawable() {
        // TODO: advance ledger to T, pause, advance to T+1000,
        // verify withdrawable at T+1000 == withdrawable at T
    }

    #[test]
    fn test_resume_continues_from_pause_point() {
        // TODO: pause at T, resume at T+500, verify rate continues from T
        // not from T+500 (paused duration should not count)
    }

    #[test]
    fn test_double_pause_fails() {
        // TODO: pause, pause again → AlreadyPaused
    }

    #[test]
    fn test_resume_without_pause_fails() {
        // TODO: resume on running stream → NotPaused
    }

    #[test]
    fn test_only_sender_can_pause() {
        // TODO: recipient calls pause → NotAuthorized
    }

    #[test]
    fn test_recipient_can_still_withdraw_while_paused() {
        // TODO: stream some time, pause, recipient withdraws accumulated amount
    }
}
