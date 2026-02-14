#![no_std]

mod storage;

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

use storage::DataKey;

#[contracttype]
#[derive(Clone)]
pub struct GovernorConfig {
    pub fee_bps:             u32,
    pub fee_recipient:       Address,
    pub min_duration_seconds: u64,
    pub max_rate_per_second:  i128,
    pub factory_address:     Address,
}

#[derive(soroban_sdk::contracterror, Copy, Clone, Debug)]
#[repr(u32)]
pub enum Error {
    NotAuthorized = 1,
    InvalidParam  = 2,
}

#[contract]
pub struct DripGovernor;

#[contractimpl]
impl DripGovernor {
    /// Deploy-time initialisation.
    pub fn initialize(
        env:                  Env,
        authority:            Address,
        fee_recipient:        Address,
        factory_address:      Address,
    ) {
        let s = env.storage().instance();
        s.set(&DataKey::Authority,           &authority);
        s.set(&DataKey::FeeBps,              &30_u32);
        s.set(&DataKey::FeeRecipient,        &fee_recipient);
        s.set(&DataKey::MinDurationSeconds,  &3600_u64);
        s.set(&DataKey::MaxRatePerSecond,    &1_000_000_000_000_000_i128);
        s.set(&DataKey::FactoryAddress,      &factory_address);
    }

    // ── Reads ────────────────────────────────────────────────────────────

    pub fn config(env: Env) -> GovernorConfig {
        let s = env.storage().instance();
        GovernorConfig {
            fee_bps:              s.get(&DataKey::FeeBps).unwrap_or(30),
            fee_recipient:        s.get(&DataKey::FeeRecipient).unwrap(),
            min_duration_seconds: s.get(&DataKey::MinDurationSeconds).unwrap_or(3600),
            max_rate_per_second:  s.get(&DataKey::MaxRatePerSecond).unwrap_or(1_000_000_000_000_000),
            factory_address:      s.get(&DataKey::FactoryAddress).unwrap(),
        }
    }

    // ── Writes (authority-gated) ─────────────────────────────────────────

    pub fn set_fee_bps(env: Env, fee_bps: u32) -> Result<(), Error> {
        Self::require_authority(&env)?;
        if fee_bps > 10_000 { return Err(Error::InvalidParam); }
        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);
        Ok(())
    }

    pub fn set_fee_recipient(env: Env, recipient: Address) -> Result<(), Error> {
        Self::require_authority(&env)?;
        env.storage().instance().set(&DataKey::FeeRecipient, &recipient);
        Ok(())
    }

    pub fn set_min_duration(env: Env, seconds: u64) -> Result<(), Error> {
        Self::require_authority(&env)?;
        if seconds == 0 { return Err(Error::InvalidParam); }
        env.storage().instance().set(&DataKey::MinDurationSeconds, &seconds);
        Ok(())
    }

    pub fn set_max_rate(env: Env, max_rate: i128) -> Result<(), Error> {
        Self::require_authority(&env)?;
        if max_rate <= 0 { return Err(Error::InvalidParam); }
        env.storage().instance().set(&DataKey::MaxRatePerSecond, &max_rate);
        Ok(())
    }

    pub fn transfer_authority(env: Env, new_authority: Address) -> Result<(), Error> {
        Self::require_authority(&env)?;
        env.storage().instance().set(&DataKey::Authority, &new_authority);
        Ok(())
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn require_authority(env: &Env) -> Result<(), Error> {
        let authority: Address = env.storage().instance()
            .get(&DataKey::Authority)
            .ok_or(Error::NotAuthorized)?;
        authority.require_auth();
        Ok(())
    }
}
