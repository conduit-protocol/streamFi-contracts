//! Tests for DripFactory stream creation and registry.

#[cfg(test)]
mod factory_deploy {
    #[test]
    fn test_create_stream_returns_incrementing_id() {
        // TODO: create 3 streams, verify IDs are 0, 1, 2
    }

    #[test]
    fn test_stream_address_registered() {
        // TODO: create stream, verify factory.stream_address(id) returns valid address
    }

    #[test]
    fn test_streams_by_sender_indexed() {
        // TODO: create 3 streams from same sender, verify streams_by_sender returns all 3
    }

    #[test]
    fn test_streams_by_recipient_indexed() {
        // TODO: create 2 streams to same recipient, verify streams_by_recipient returns both
    }

    #[test]
    fn test_invalid_deposit_rejected() {
        // TODO: deposit=0 → InvalidDeposit
    }

    #[test]
    fn test_backdated_stream_rejected() {
        // TODO: start_time < now → BackdatedStream
    }

    #[test]
    fn test_invalid_time_range_rejected() {
        // TODO: end_time <= start_time → InvalidTimeRange
    }

    #[test]
    fn test_insufficient_deposit_rejected() {
        // TODO: deposit < rate_per_sec → InsufficientDeposit
    }
}
