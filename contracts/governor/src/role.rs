use soroban_sdk::{contracttype, Address, Env, Vec};

use drip_common::rbac;

use crate::storage::{DataKey, RoleKey};
use crate::ttl;
use crate::Error;

/// Re-export so callers using `role::Role` continue to work unchanged.
pub use crate::storage::Role;

/// A page of role members, with the total count for pagination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleMembersPage {
    pub members: Vec<Address>,
    pub total: u32,
}

const MAX_ROLE_MEMBERS_PAGE_SIZE: u32 = 100;

// ── Key helpers ────────────────────────────────────────────────────────────

fn role_key(role: Role, account: &Address) -> DataKey {
    DataKey::Role(RoleKey {
        role,
        account: account.clone(),
    })
}

fn members_key(role: Role) -> DataKey {
    DataKey::RoleMembers(role)
}

// ── Public API (delegates to drip_common::rbac) ────────────────────────────

/// Whether `account` currently holds `role`.
///
/// **Storage note:** Role membership is stored per (role, account) key in instance storage
/// (see `role_key` and `DataKey::Role`). Lookup is O(1) — a direct storage read, not a
/// traversal of role members. This remains efficient regardless of the number of accounts
/// holding the role, so `require_role` (called on every role-gated write) has constant-time
/// cost independent of membership size.
pub fn has_role(env: &Env, role: Role, account: &Address) -> bool {
    rbac::has_role(env, &role_key(role, account))
}

/// Number of accounts currently holding `Role::SuperAdmin` (zero pre-initialization).
///
/// Part of the governor RBAC surface; tracks the highest tier of administration.
#[allow(dead_code)]
pub fn super_admin_count(env: &Env) -> u32 {
    rbac::admin_count(env, &DataKey::SuperAdminCount)
}

/// Number of accounts currently holding `Role::Admin` (zero pre-initialization).
///
/// Part of the governor RBAC surface; no contract entry point calls it yet.
#[allow(dead_code)]
pub fn admin_count(env: &Env) -> u32 {
    rbac::admin_count(env, &DataKey::AdminCount)
}

/// Grants `role` to `account` with an optional expiration timestamp (Issue #692).
pub fn grant_with_expiration(
    env: &Env,
    role: Role,
    account: &Address,
    expires_at: Option<u64>,
) -> bool {
    let count_key = match role {
        Role::SuperAdmin => &DataKey::SuperAdminCount,
        Role::Admin => &DataKey::AdminCount,
        _ => &DataKey::AdminCount,
    };

    rbac::grant_with_expiration(
        env,
        &role_key(role, account),
        count_key,
        &members_key(role),
        role == Role::SuperAdmin || role == Role::Admin,
        account,
        expires_at,
    )
}

/// Grants `role` to `account`.
///
/// Idempotent: re-granting a role the account already holds is a no-op, so the
/// admin count can never be inflated by repeated grants. Returns `true` if the
/// role was newly granted, or `false` if the account already held it.
pub fn grant(env: &Env, role: Role, account: &Address) -> bool {
    let count_key = match role {
        Role::SuperAdmin => &DataKey::SuperAdminCount,
        Role::Admin => &DataKey::AdminCount,
        _ => &DataKey::AdminCount,
    };

    rbac::grant(
        env,
        &role_key(role, account),
        count_key,
        &members_key(role),
        role == Role::SuperAdmin || role == Role::Admin,
        account,
    )
}

/// Requires that the caller has sufficient privilege to revoke a target role (Issue #907).
///
/// Enforces role hierarchy to prevent governance capture:
/// - Only `SuperAdmin` can revoke `SuperAdmin` or `Admin` roles
/// - `Admin` can revoke lower roles (`FeeManager`, `RateManager`, `Pauser`, `Executor`) only
/// - Lower roles cannot revoke any role
pub fn require_revoke_privilege(
    env: &Env,
    caller: &Address,
    target_role: Role,
) -> Result<(), Error> {
    let caller_is_super_admin = has_role(env, Role::SuperAdmin, caller);
    let caller_is_admin = has_role(env, Role::Admin, caller);

    match target_role {
        Role::SuperAdmin | Role::Admin => {
            if caller_is_super_admin {
                Ok(())
            } else {
                Err(Error::NotAuthorized)
            }
        }
        _ => {
            if caller_is_super_admin || caller_is_admin {
                Ok(())
            } else {
                Err(Error::NotAuthorized)
            }
        }
    }
}

/// Revokes `role` from `account`.
///
/// Idempotent when the account doesn't hold the role. Enforces role hierarchy
/// to prevent governance capture via compromised sub-admin keys:
/// - Only `SuperAdmin` can revoke `SuperAdmin` or `Admin` roles
/// - `Admin` can revoke lower roles only
/// - Refuses to remove the final `SuperAdmin` or `Admin` (`LastAdmin`):
///   a governor with zero admins could never grant a new one, permanently
///   freezing every protocol parameter. Returns `true` if the role was newly
///   revoked, or `false` if the account did not hold it.
pub fn revoke(env: &Env, role: Role, account: &Address) -> Result<bool, Error> {
    let count_key = match role {
        Role::SuperAdmin => &DataKey::SuperAdminCount,
        Role::Admin => &DataKey::AdminCount,
        _ => &DataKey::AdminCount,
    };

    rbac::revoke(
        env,
        &role_key(role, account),
        count_key,
        &members_key(role),
        role == Role::SuperAdmin || role == Role::Admin,
        account,
    )
    .map_err(|e| match e {
        rbac::RbacError::LastAdmin => Error::LastAdmin,
        rbac::RbacError::NotAuthorized => Error::NotAuthorized,
    })
}

/// Requires that `caller` both authorized the transaction and holds `role`,
/// then bumps instance TTL. Every role-gated write funnels through here.
pub fn require_role(env: &Env, caller: &Address, role: Role) -> Result<(), Error> {
    rbac::require_role(env, caller, &role_key(role, caller), Some(ttl::bump))
        .map_err(|_| Error::NotAuthorized)
}

/// Requires that `caller` is authorized and holds the `Executor` role.
pub fn require_executor(env: &Env, caller: &Address) -> Result<(), Error> {
    require_role(env, caller, Role::Executor)
}

/// Returns a page of accounts currently holding `role`.
///
/// Reads from the `RoleMembers` index maintained by `grant`/`revoke`.
/// The result is capped at 100 members and includes the total member count.
pub fn role_members(env: &Env, role: Role, offset: u32, limit: u32) -> RoleMembersPage {
    let all_members = rbac::role_members(env, &members_key(role));
    let mut active_members = Vec::new(env);
    for account in all_members.iter() {
        if has_role(env, role, &account) {
            active_members.push_back(account);
        }
    }
    let total = active_members.len();
    let start = offset.min(total);
    let end = start
        .saturating_add(limit.min(MAX_ROLE_MEMBERS_PAGE_SIZE))
        .min(total);
    let mut members = Vec::new(env);
    for index in start..end {
        members.push_back(active_members.get(index).unwrap());
    }
    RoleMembersPage { members, total }
}

/// Requires that `caller` authorized the transaction and holds `role` **or**
/// is an `Admin`.  Admin acts as a super-user: even after delegating
/// `FeeManager` / `RateManager` to dedicated wallets the deployer can still
/// adjust every parameter directly.
///
/// Bumps instance TTL on success.
pub fn require_role_or_admin(env: &Env, caller: &Address, role: Role) -> Result<(), Error> {
    rbac::require_role_or_admin(
        env,
        caller,
        &role_key(role, caller),
        &role_key(Role::Admin, caller),
        Some(ttl::bump),
    )
    .map_err(|_| Error::NotAuthorized)
}

/// Require that `caller` authorized the transaction and holds `Pauser` **or** `Admin` role.
/// Emergency responders (pausers) can execute pause/unpause operations (Issue #771).
/// This separates regular parameter tuning from emergency pause authority.
///
/// Bumps instance TTL on success.
pub fn require_emergency_responder(env: &Env, caller: &Address) -> Result<(), Error> {
    require_role_or_admin(env, caller, Role::Pauser)
}

/// Propose transferring the admin role to a new address (step 1 of 2) (Issue #682).
pub fn propose_admin(env: &Env, proposer: &Address, new_admin: &Address) -> Result<(), Error> {
    require_role(env, proposer, Role::Admin)?;
    if env.storage().instance().has(&DataKey::PendingAuthority) {
        return Err(Error::AdminTransferAlreadyInProgress);
    }
    if has_role(env, Role::Admin, new_admin) {
        return Err(Error::InvalidParam);
    }
    env.storage()
        .instance()
        .set(&DataKey::PendingAuthority, new_admin);
    env.storage()
        .instance()
        .set(&DataKey::PendingAuthorityProposer, proposer);
    crate::events::propose_authority(env, proposer, new_admin);
    Ok(())
}

/// Claim proposed admin role (step 2 of 2) (Issue #682).
pub fn claim_admin(env: &Env, claimer: &Address) -> Result<(), Error> {
    let pending: Address = env
        .storage()
        .instance()
        .get(&DataKey::PendingAuthority)
        .ok_or(Error::NoPendingAuthority)?;
    if *claimer != pending {
        return Err(Error::NotPendingAuthority);
    }
    claimer.require_auth();
    ttl::bump(env);

    let proposer: Address = env
        .storage()
        .instance()
        .get(&DataKey::PendingAuthorityProposer)
        .ok_or(Error::NoPendingAuthority)?;

    grant(env, Role::Admin, claimer);
    let _ = revoke(env, Role::Admin, &proposer)?;

    env.storage().instance().remove(&DataKey::PendingAuthority);
    env.storage()
        .instance()
        .remove(&DataKey::PendingAuthorityProposer);

    crate::events::accept_authority(env, claimer, &proposer);
    Ok(())
}

/// Returns the pending proposed admin address, if any (Issue #682).
pub fn pending_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::PendingAuthority)
}

/// Revokes the currently pending admin proposal.
pub fn revoke_propose_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    require_role(env, caller, Role::Admin)?;
    let storage = env.storage().instance();
    let pending: Address = storage
        .get(&DataKey::PendingAuthority)
        .ok_or(Error::NoPendingAuthority)?;
    storage.remove(&DataKey::PendingAuthority);
    storage.remove(&DataKey::PendingAuthorityProposer);
    crate::events::revoke_propose_authority(env, caller, &pending);
    Ok(())
}
