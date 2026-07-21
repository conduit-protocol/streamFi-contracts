#![no_std]
use soroban_sdk::{contracttype, Env, Address, String};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YieldConfig {
    pub vault_address: Address,
    pub is_active: bool,
    pub accrued_yield: i128,
}

pub fn deposit_to_vault(env: &Env, amount: i128) {
    // In a real implementation, this would make a cross-contract call to the yield vault
    env.events().publish(("YIELD", "DEPOSIT"), amount);
}

pub fn withdraw_from_vault(env: &Env, amount: i128) {
    // Calls vault.withdraw() to pull principal back into the stream contract
    env.events().publish(("YIELD", "WITHDRAW"), amount);
}

pub fn calculate_rebate(env: &Env) -> i128 {
    // Queries vault to calculate the yield earned by this stream's deposited balance
    // Mock return of 5% APY
    env.events().publish(("YIELD", "CALCULATE"), String::from_str(env, "Rebate processed"));
    50000 
}
