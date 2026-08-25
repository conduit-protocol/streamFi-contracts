#![no_std]

//! Shared constants and utilities for the Drip protocol contracts.

use soroban_sdk::{Address, Env};

/// TTL threshold for instance storage extension.
/// When the remaining TTL falls below this value, extend to `TTL_EXTEND_TO`.
pub const TTL_THRESHOLD: u32 = 100_000;

/// Target TTL for instance storage extension.
/// Extended to this value when `TTL_THRESHOLD` is reached.
pub const TTL_EXTEND_TO: u32 = 200_000;

/// Returns true when `address` is the all-zero Stellar account address.
///
/// The zero Stellar account is represented by an Ed25519 public key
/// consisting entirely of zero bytes. Its G... string form is hardcoded
/// here once so every contract sharing this helper uses the exact same
/// literal — a duplicated copy would be easy to typo differently without
/// anyone noticing.
pub fn is_zero_stellar_account(env: &Env, address: &Address) -> bool {
    let zero_account = Address::from_string(&soroban_sdk::String::from_str(
        env,
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
    ));

    address == &zero_account
}
