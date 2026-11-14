//! Instance and persistent TTL management for `TokenVault`.
//!
//! The threshold/extend-to values are protocol-wide policy and live in
//! [`drip_common::ttl`], so every contract renews storage on the same
//! schedule.

pub use drip_common::ttl::bump_instance;

/// Renew all still-live persistent token-vault keys recorded in the index.
pub fn bump_all_vault_keys(env: &soroban_sdk::Env) -> u32 {
    let keys = crate::storage::tracked_vault_keys(env);
    let mut refreshed = 0u32;
    for key in keys.iter() {
        // An expired persistent entry cannot be restored by extend_ttl.
        if env.storage().persistent().has(&key) {
            drip_common::ttl::bump_persistent(env, &key);
            refreshed += 1;
        }
    }
    refreshed
}
