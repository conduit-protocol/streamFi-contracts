use soroban_sdk::{symbol_short, Address, Env};

use crate::Role;

pub fn initialized(env: &Env, authority: &Address, fee_recipient: &Address, factory_address: &Address) {
    env.events().publish(
        (symbol_short!("init"), authority.clone()),
        (fee_recipient.clone(), factory_address.clone()),
    );
}

pub fn grant_role(env: &Env, caller: &Address, role: Role, account: &Address) {
    env.events().publish(
        (symbol_short!("grant"), caller.clone()),
        (role, account.clone()),
    );
}

pub fn revoke_role(env: &Env, caller: &Address, role: Role, account: &Address) {
    env.events().publish(
        (symbol_short!("revoke"), caller.clone()),
        (role, account.clone()),
    );
}

pub fn transfer_authority(env: &Env, caller: &Address, new_authority: &Address) {
    env.events().publish(
        (symbol_short!("transfer"), caller.clone()),
        new_authority.clone(),
    );
}

pub fn set_fee_bps(env: &Env, caller: &Address, fee_bps: u32) {
    env.events().publish(
        (symbol_short!("fee_bps"), caller.clone()),
        fee_bps,
    );
}

pub fn set_fee_recipient(env: &Env, caller: &Address, recipient: &Address) {
    env.events().publish(
        (symbol_short!("fee_rec"), caller.clone()),
        recipient.clone(),
    );
}

pub fn set_min_duration(env: &Env, caller: &Address, seconds: u64) {
    env.events().publish(
        (symbol_short!("min_dur"), caller.clone()),
        seconds,
    );
}

pub fn set_max_rate(env: &Env, caller: &Address, max_rate: i128) {
    env.events().publish(
        (symbol_short!("max_rate"), caller.clone()),
        max_rate,
    );
}

pub fn set_max_duration(env: &Env, caller: &Address, seconds: u64) {
    env.events().publish(
        (symbol_short!("max_dur"), caller.clone()),
        seconds,
    );
}
