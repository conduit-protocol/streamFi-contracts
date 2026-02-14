//! Tests for DripGovernor parameter management.

#[cfg(test)]
mod governor_config {
    #[test]
    fn test_defaults_set_on_initialize() {
        // TODO: deploy governor, verify fee_bps=30, min_duration=3600
    }

    #[test]
    fn test_authority_can_set_fee_bps() {
        // TODO: set_fee_bps(50), verify config().fee_bps == 50
    }

    #[test]
    fn test_non_authority_cannot_set_fee_bps() {
        // TODO: call set_fee_bps from non-authority → NotAuthorized
    }

    #[test]
    fn test_fee_bps_cannot_exceed_10000() {
        // TODO: set_fee_bps(10001) → InvalidParam
    }

    #[test]
    fn test_transfer_authority() {
        // TODO: transfer_authority(new_addr), verify old authority can no longer write
    }

    #[test]
    fn test_set_zero_min_duration_rejected() {
        // TODO: set_min_duration(0) → InvalidParam
    }
}
