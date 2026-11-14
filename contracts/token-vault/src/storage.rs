use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Owner,
    Token,
    MaxLimit,
    Balance,
    /// Optional operator address delegated by the owner.
    ///
    /// When set, the operator can perform owner-level actions (`withdraw`,
    /// `set_limit`) on behalf of the owner — a hot-wallet / ops-key
    /// pattern matching `DripStream`'s `set_operator` design.
    /// Absent key means no operator has been delegated.
    Operator,
    /// Maximum amount a delegated operator may withdraw in a single call.
    ///
    /// When set, `withdraw` enforces this cap for operator-authenticated
    /// withdrawals while the owner remains unbounded. If this key is absent,
    /// the operator is effectively unable to withdraw until the owner sets a
    /// positive cap.
    OperatorWithdrawLimit,
    /// Emergency-pause flag. When `true`, all state-mutating entry points
    /// (deposit, withdraw, set_limit) revert before touching state.
    Paused,
    /// Pending owner address for the 2-step owner transfer.
    /// Set by `propose_owner`, consumed by `accept_owner`.
    PendingOwner,
    /// The current owner who proposed the transfer.
    /// Used to hand ownership over when the pending owner accepts.
    PendingOwnerProposer,
    /// Ledger timestamp when the pending owner was proposed.
    PendingOwnerProposedAt,
    /// Validity period for pending owner proposals, in seconds.
    OwnerProposalTtl,
    /// Persistent storage: balance of a specific token held for a specific account (Issue #724).
    TokenAccountBalance(Address, Address),
    /// Persistent storage: optional maximum deposit limit per token (Issue #724).
    TokenMaxLimit(Address),
    /// Pending emergency migration to cold storage with a timelock (Issue #679).
    EmergencyMigration,
    /// Transient reentrancy guard lock flag for external token transfers.
    ReentrancyLock,
    /// Owner-approved SEP-0041 token contract address.
    ApprovedToken(Address),
    /// First observed / last verified vault balance in a ledger.
    BalanceSnapshot(Address),
    /// Index of persistent token-account and token-limit keys requiring renewal.
    TrackedVaultKeys,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultBalanceSnapshot {
    pub ledger: u32,
    pub expected_balance: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyMigration {
    pub destination: Address,
    pub proposed_at: u64,
    pub executable_at: u64,
}

pub fn set_owner(env: &Env, a: &Address) {
    env.storage().instance().set(&DataKey::Owner, a);
}

pub fn get_owner(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Owner)
}

pub fn set_token(env: &Env, t: &Address) {
    env.storage().instance().set(&DataKey::Token, t);
}

pub fn get_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Token)
}

pub fn set_max_limit(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::MaxLimit, v);
}

pub fn get_max_limit(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::MaxLimit)
}

#[allow(dead_code)]
pub fn set_balance(env: &Env, v: &i128) {
    env.storage().instance().set(&DataKey::Balance, v);
}

#[allow(dead_code)]
pub fn get_balance(env: &Env) -> Option<i128> {
    env.storage().instance().get(&DataKey::Balance)
}

#[allow(dead_code)]
pub fn remove_operator_withdraw_limit(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::OperatorWithdrawLimit);
}

pub fn set_operator(env: &Env, op: &Address) {
    env.storage().instance().set(&DataKey::Operator, op);
}

pub fn get_operator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::Operator)
}

pub fn remove_operator(env: &Env) {
    env.storage().instance().remove(&DataKey::Operator);
}

pub fn set_operator_withdraw_limit(env: &Env, v: &i128) {
    env.storage()
        .instance()
        .set(&DataKey::OperatorWithdrawLimit, v);
}

pub fn get_operator_withdraw_limit(env: &Env) -> Option<i128> {
    env.storage()
        .instance()
        .get(&DataKey::OperatorWithdrawLimit)
}

pub fn set_paused(env: &Env, paused: bool) {
    let _ = drip_common::pause::set_paused(env, &DataKey::Paused, paused);
}

pub fn is_paused(env: &Env) -> bool {
    drip_common::pause::is_paused(env, &DataKey::Paused)
}

pub fn set_pending_owner(env: &Env, owner: &Address) {
    env.storage().instance().set(&DataKey::PendingOwner, owner);
}

pub fn get_pending_owner(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingOwner)
}

pub fn remove_pending_owner(env: &Env) {
    env.storage().instance().remove(&DataKey::PendingOwner);
}

pub fn set_pending_owner_proposer(env: &Env, proposer: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::PendingOwnerProposer, proposer);
}

pub fn get_pending_owner_proposer(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingOwnerProposer)
}

pub fn remove_pending_owner_proposer(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::PendingOwnerProposer);
}

pub fn set_pending_owner_proposed_at(env: &Env, timestamp: u64) {
    env.storage()
        .instance()
        .set(&DataKey::PendingOwnerProposedAt, &timestamp);
}

pub fn get_pending_owner_proposed_at(env: &Env) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::PendingOwnerProposedAt)
}

pub fn remove_pending_owner_proposed_at(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::PendingOwnerProposedAt);
}

pub fn set_owner_proposal_ttl(env: &Env, ttl_seconds: u64) {
    env.storage()
        .instance()
        .set(&DataKey::OwnerProposalTtl, &ttl_seconds);
}

pub fn get_owner_proposal_ttl(env: &Env) -> Option<u64> {
    env.storage().instance().get(&DataKey::OwnerProposalTtl)
}

pub fn get_token_account_balance(env: &Env, token: &Address, account: &Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::TokenAccountBalance(
            token.clone(),
            account.clone(),
        ))
        .unwrap_or(0)
}

pub fn set_token_account_balance(env: &Env, token: &Address, account: &Address, balance: i128) {
    let key = DataKey::TokenAccountBalance(token.clone(), account.clone());
    env.storage().persistent().set(&key, &balance);
    drip_common::ttl::bump_persistent(env, &key);
    track_vault_key(env, &key);
}

pub fn get_token_max_limit(env: &Env, token: &Address) -> Option<i128> {
    env.storage()
        .persistent()
        .get(&DataKey::TokenMaxLimit(token.clone()))
}

pub fn set_token_max_limit(env: &Env, token: &Address, limit: &i128) {
    let key = DataKey::TokenMaxLimit(token.clone());
    env.storage().persistent().set(&key, limit);
    drip_common::ttl::bump_persistent(env, &key);
    track_vault_key(env, &key);
}

pub fn set_emergency_migration(env: &Env, m: &EmergencyMigration) {
    env.storage()
        .instance()
        .set(&DataKey::EmergencyMigration, m);
}

pub fn get_emergency_migration(env: &Env) -> Option<EmergencyMigration> {
    env.storage().instance().get(&DataKey::EmergencyMigration)
}

pub fn remove_emergency_migration(env: &Env) {
    env.storage()
        .instance()
        .remove(&DataKey::EmergencyMigration);
}

/// Explicit owner approval is required before accepting a multi-token deposit.
pub fn is_token_approved(env: &Env, token: &Address) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::ApprovedToken(token.clone()))
        .unwrap_or(false)
}

pub fn set_token_approved(env: &Env, token: &Address, allowed: bool) {
    env.storage()
        .instance()
        .set(&DataKey::ApprovedToken(token.clone()), &allowed);
}

/// Store the last verified balance during this ledger. A subsequent call
/// observing a different balance cannot use a potentially manipulated value.
pub fn check_vault_balance(env: &Env, token: &Address, balance: i128) -> bool {
    match env
        .storage()
        .instance()
        .get::<_, VaultBalanceSnapshot>(&DataKey::BalanceSnapshot(token.clone()))
    {
        Some(snapshot) if snapshot.ledger == env.ledger().sequence() => {
            snapshot.expected_balance == balance
        }
        _ => true,
    }
}

pub fn record_vault_balance(env: &Env, token: &Address, balance: i128) {
    let snapshot = VaultBalanceSnapshot {
        ledger: env.ledger().sequence(),
        expected_balance: balance,
    };
    env.storage()
        .instance()
        .set(&DataKey::BalanceSnapshot(token.clone()), &snapshot);
}

fn track_vault_key(env: &Env, key: &DataKey) {
    let mut keys: Vec<DataKey> = env
        .storage()
        .instance()
        .get(&DataKey::TrackedVaultKeys)
        .unwrap_or(Vec::new(env));
    if !keys.contains(key) {
        keys.push_back(key.clone());
        env.storage()
            .instance()
            .set(&DataKey::TrackedVaultKeys, &keys);
    }
}

pub fn tracked_vault_keys(env: &Env) -> Vec<DataKey> {
    env.storage()
        .instance()
        .get(&DataKey::TrackedVaultKeys)
        .unwrap_or(Vec::new(env))
}
