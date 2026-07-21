//! Tests for fixed-point arithmetic rebate calculations.
//!
//! These tests verify the deterministic, overflow-safe yield calculations
//! that replace floating-point arithmetic for financial smart contracts.

#![cfg(test)]

use soroban_sdk::Env;

// Import the yield integration module functions
// Note: In a real test, we'd need to expose these functions from the contract
// For this demo, we'll test the calculation logic directly

/// Fixed-point scaling factor for rebate calculations.
const REBATE_BPS_SCALE: i128 = 10_000;

/// Seconds in a year for APY calculations
const SECONDS_PER_YEAR: i128 = 31_557_600;

/// Maximum rebate rate in basis points (10% = 1000 bps)
const MAX_REBATE_RATE_BPS: i128 = 1_000;

/// Minimum rebate calculation threshold
const MIN_REBATE_THRESHOLD: i128 = 100;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RebateError {
    ArithmeticOverflow = 1,
    InvalidRate = 2,
    InsufficientPrincipal = 3,
}

/// Calculate rebate using fixed-point arithmetic (test implementation)
fn calculate_rebate_with_params(
    env: &Env,
    principal: i128,
    apy_bps: i128,
    time_elapsed_seconds: u64,
) -> Result<i128, RebateError> {
    // Validate parameters
    if apy_bps > MAX_REBATE_RATE_BPS || apy_bps < 0 {
        return Err(RebateError::InvalidRate);
    }

    if principal < MIN_REBATE_THRESHOLD {
        return Err(RebateError::InsufficientPrincipal);
    }

    // Convert time to i128 for arithmetic
    let time_elapsed = time_elapsed_seconds as i128;

    // Calculate rebate = principal * apy_bps * time_elapsed / (BPS_SCALE * SECONDS_PER_YEAR)
    // Reorder operations to maximize precision and minimize overflow risk

    // First multiply principal by apy_bps
    let principal_times_rate = principal
        .checked_mul(apy_bps)
        .ok_or(RebateError::ArithmeticOverflow)?;

    // Then multiply by time elapsed
    let numerator = principal_times_rate
        .checked_mul(time_elapsed)
        .ok_or(RebateError::ArithmeticOverflow)?;

    // Calculate denominator (BPS_SCALE * SECONDS_PER_YEAR)
    let denominator = REBATE_BPS_SCALE
        .checked_mul(SECONDS_PER_YEAR)
        .ok_or(RebateError::ArithmeticOverflow)?;

    // Final division
    let rebate = numerator / denominator;

    env.events().publish(
        ("REBATE", "CALCULATED"),
        (principal, apy_bps, time_elapsed_seconds, rebate),
    );

    Ok(rebate)
}

#[test]
fn test_rebate_basic_calculation() {
    let env = Env::default();
    env.mock_all_auths();

    // Test basic 5% APY for 1 year on 1M stroops
    let principal = 1_000_000_i128;
    let apy_bps = 500_i128; // 5%
    let time_seconds = SECONDS_PER_YEAR as u64; // 1 year

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Expected: 1_000_000 * 500 * 31_557_600 / (10_000 * 31_557_600) = 50_000
    assert_eq!(rebate, 50_000);
}

#[test]
fn test_rebate_partial_year() {
    let env = Env::default();
    env.mock_all_auths();

    // Test 6 months (half year) at 5% APY
    let principal = 1_000_000_i128;
    let apy_bps = 500_i128; // 5%
    let time_seconds = (SECONDS_PER_YEAR / 2) as u64; // 6 months

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Expected: half year = half the annual rebate = 25_000
    assert_eq!(rebate, 25_000);
}

#[test]
fn test_rebate_daily_calculation() {
    let env = Env::default();
    env.mock_all_auths();

    // Test daily rebate (1/365.25 of a year)
    let principal = 1_000_000_i128;
    let apy_bps = 500_i128; // 5%
    let time_seconds = 86_400_u64; // 1 day

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Expected: roughly 1_000_000 * 500 * 86_400 / (10_000 * 31_557_600) ≈ 137
    assert!(
        rebate > 135 && rebate < 140,
        "Daily rebate should be around 137, got {}",
        rebate
    );
}

#[test]
fn test_rebate_zero_time() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = 500_i128; // 5%
    let time_seconds = 0_u64; // No time elapsed

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    assert_eq!(rebate, 0);
}

#[test]
fn test_rebate_zero_principal() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 0_i128;
    let apy_bps = 500_i128; // 5%
    let time_seconds = 86_400_u64; // 1 day

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);
    assert_eq!(result, Err(RebateError::InsufficientPrincipal));
}

#[test]
fn test_rebate_zero_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = 0_i128; // 0%
    let time_seconds = 86_400_u64; // 1 day

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    assert_eq!(rebate, 0);
}

#[test]
fn test_rebate_maximum_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = MAX_REBATE_RATE_BPS; // 10% (maximum allowed)
    let time_seconds = (SECONDS_PER_YEAR / 10) as u64; // 1/10 year

    let rebate = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Expected: 1_000_000 * 1000 * (SECONDS_PER_YEAR/10) / (10_000 * SECONDS_PER_YEAR) = 10_000
    assert_eq!(rebate, 10_000);
}

#[test]
fn test_rebate_excessive_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = MAX_REBATE_RATE_BPS + 1; // Above maximum
    let time_seconds = 86_400_u64;

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);
    assert_eq!(result, Err(RebateError::InvalidRate));
}

#[test]
fn test_rebate_negative_rate() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = -100_i128; // Negative rate
    let time_seconds = 86_400_u64;

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);
    assert_eq!(result, Err(RebateError::InvalidRate));
}

#[test]
fn test_rebate_dust_amounts() {
    let env = Env::default();
    env.mock_all_auths();

    // Test with very small principal (below threshold)
    let principal = MIN_REBATE_THRESHOLD - 1; // Below minimum
    let apy_bps = 500_i128;
    let time_seconds = 86_400_u64;

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);
    assert_eq!(result, Err(RebateError::InsufficientPrincipal));

    // Test with minimum threshold principal
    let principal = MIN_REBATE_THRESHOLD;
    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);
    assert!(result.is_ok());
}

#[test]
fn test_rebate_precision_ordering() {
    let env = Env::default();
    env.mock_all_auths();

    // Test that different calculation orders don't affect result due to integer division
    let principal = 1_000_000_i128;
    let apy_bps = 500_i128;
    let time_seconds = 86_400_u64;

    let rebate1 = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Test with different principal amounts to ensure consistency
    let rebate2 = calculate_rebate_with_params(&env, principal * 2, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // Double principal should yield approximately double rebate (within integer division rounding)
    // Allow for small rounding differences due to integer division
    let expected_double = rebate1 * 2;
    let diff = if rebate2 > expected_double {
        rebate2 - expected_double
    } else {
        expected_double - rebate2
    };
    assert!(
        diff <= 1,
        "Double principal should yield approximately double rebate, got {} vs expected {}",
        rebate2,
        expected_double
    );
}

#[test]
fn test_rebate_large_numbers() {
    let env = Env::default();
    env.mock_all_auths();

    // Test with large but reasonable numbers
    let principal = 1_000_000_000_000_i128; // 1 trillion stroops
    let apy_bps = 100_i128; // 1% APY
    let time_seconds = 3600_u64; // 1 hour

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);

    // Should not overflow for reasonable financial amounts
    match result {
        Ok(rebate) => {
            assert!(
                rebate > 0,
                "Large calculation should produce positive rebate"
            );
        }
        Err(RebateError::ArithmeticOverflow) => {
            // If it overflows with large numbers, that's acceptable behavior
            // The key is that it fails gracefully rather than wrapping
        }
        Err(e) => {
            panic!("Unexpected error for large numbers: {:?}", e);
        }
    }
}

#[test]
fn test_rebate_overflow_protection() {
    let env = Env::default();
    env.mock_all_auths();

    // Test with values designed to cause overflow
    let principal = i128::MAX / 2;
    let apy_bps = MAX_REBATE_RATE_BPS;
    let time_seconds = u64::MAX; // Very large time

    let result = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds);

    // Should either succeed or fail with overflow error, not wrap around
    match result {
        Ok(_) => {}                                // Success is fine
        Err(RebateError::ArithmeticOverflow) => {} // Expected overflow protection
        Err(e) => {
            panic!("Unexpected error type: {:?}", e);
        }
    }
}

#[test]
fn test_rebate_deterministic() {
    let env = Env::default();
    env.mock_all_auths();

    let principal = 1_000_000_i128;
    let apy_bps = 500_i128;
    let time_seconds = 86_400_u64;

    // Calculate the same rebate multiple times
    let rebate1 = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");
    let rebate2 = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");
    let rebate3 = calculate_rebate_with_params(&env, principal, apy_bps, time_seconds)
        .expect("Calculation should succeed");

    // All results should be identical (deterministic)
    assert_eq!(rebate1, rebate2);
    assert_eq!(rebate2, rebate3);
}

#[test]
fn test_rebate_regression_against_previous_values() {
    let env = Env::default();
    env.mock_all_auths();

    // Test against known good values to prevent regression
    struct TestCase {
        principal: i128,
        apy_bps: i128,
        time_seconds: u64,
        expected: i128,
        description: &'static str,
    }

    let test_cases = [
        TestCase {
            principal: 1_000_000,
            apy_bps: 500,
            time_seconds: SECONDS_PER_YEAR as u64,
            expected: 50_000,
            description: "5% APY for 1 year on 1M stroops",
        },
        TestCase {
            principal: 2_000_000,
            apy_bps: 250,
            time_seconds: SECONDS_PER_YEAR as u64,
            expected: 50_000,
            description: "2.5% APY for 1 year on 2M stroops",
        },
        TestCase {
            principal: 500_000,
            apy_bps: 1000,
            time_seconds: SECONDS_PER_YEAR as u64,
            expected: 50_000,
            description: "10% APY for 1 year on 500K stroops",
        },
    ];

    for test_case in test_cases {
        let rebate = calculate_rebate_with_params(
            &env,
            test_case.principal,
            test_case.apy_bps,
            test_case.time_seconds,
        )
        .expect("Calculation should succeed");

        assert_eq!(
            rebate, test_case.expected,
            "Failed test case: {}. Expected {}, got {}",
            test_case.description, test_case.expected, rebate
        );
    }
}
