#![no_std]

//! Shared constants and utilities for the Drip protocol contracts.

/// TTL threshold for instance storage extension.
/// When the remaining TTL falls below this value, extend to `TTL_EXTEND_TO`.
pub const TTL_THRESHOLD: u32 = 100_000;

/// Target TTL for instance storage extension.
/// Extended to this value when `TTL_THRESHOLD` is reached.
pub const TTL_EXTEND_TO: u32 = 200_000;
