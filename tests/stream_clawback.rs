//! Tests for clawback behaviour.

#[cfg(test)]
mod stream_clawback {
    #[test]
    fn test_clawback_reclaims_unstreamed() {
        // TODO: create stream with clawback=true, advance ledger partially, clawback
        // verify sender receives deposit - streamed amount
    }

    #[test]
    fn test_clawback_disabled_returns_error() {
        // TODO: create stream with clawback=false, call clawback → ClawbackDisabled
    }

    #[test]
    fn test_only_sender_can_clawback() {
        // TODO: recipient calls clawback → NotAuthorized
    }

    #[test]
    fn test_clawback_on_cancelled_stream_fails() {
        // TODO: cancel stream, then clawback → StreamCancelled
    }
}
