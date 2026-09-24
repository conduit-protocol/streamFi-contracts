//! Shared role-based access control (RBAC) for the Drip protocol.
//!
//! # Overview
//!
//! Both `DripGovernor` and `DripOracle` implement identical RBAC mechanics:
//!
//! - A set of roles, each of which may be held by any number of accounts.
//! - An `Admin` super-user that can grant/revoke any role (including `Admin`).
//! - A `LastAdmin` guard: the final `Admin` can never be revoked, so governance
//!   can never be permanently frozen.
//! - A `RoleMembers` index maintained on every `grant`/`revoke` so membership
//!   can be enumerated on-chain without replaying events.
//! - A `require_role_or_admin` gate: the caller must hold the specific role *or*
//!   be an `Admin`.
//!
//! # Generic design
//!
//! All functions are generic over the storage key types (`RK`, `AK`, `MK`) so
//! each contract can supply its own `DataKey` variants without pulling in this
//! crate's key enum.  The only constraint is that the keys implement
//! `soroban_sdk`'s `IntoVal<Env, Val>` + `TryFromVal<Env, Val>` (which every
//! `#[contracttype]`-derived type satisfies automatically).
//!
//! # TTL bumping
//!
//! `require_role_or_admin` accepts an optional `on_success: Option<fn(&Env)>`
//! callback. Pass `Some(ttl::bump)` in `DripGovernor` (which bumps instance TTL
//! on every successful role-gated write) or `None` in `DripOracle` (which bumps
//! TTL at the call-site entry point instead, before delegation).
//!
//! # Oracle-specific side-effects
//!
//! When `DripOracle` revokes a `PriceFeeder`, it must also purge the feeder's
//! `Submitters` entry and `Submission` record. That hook is deliberately kept
//! in `oracle/src/lib.rs` — the shared `revoke` function returns `true` when a
//! role was actually removed, and the oracle wrapper calls `remove_submitter`
//! only in that case.
//!
//! # Bug fix: RoleMembers storage tier
//!
//! Issue #345 noted that `RoleMembers` was documented as "persistent" but
//! stored in `instance()`. Both copies of the bug are fixed here: the index
//! is written to `instance()` storage, which is the correct tier for a
//! bounded, contract-lifetime membership list that must survive any ledger
//! within the contract's active TTL (matching `Role(RoleKey)` and
//! `AdminCount`). If future growth makes the list unbounded, migrate to
//! `persistent()` with explicit TTL management.

use soroban_sdk::{Address, Env, IntoVal, TryFromVal, Val, Vec as SorobanVec};

// ── Trait alias helpers ────────────────────────────────────────────────────

/// Convenience bound for any type usable as an instance-storage key.
///
/// Every `#[contracttype]` enum/struct satisfies this automatically.
pub trait StorageKey: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone {}

impl<T: IntoVal<Env, Val> + TryFromVal<Env, Val> + Clone> StorageKey for T {}

// ── Error type ────────────────────────────────────────────────────────────

/// Errors that the shared RBAC helpers can return.
///
/// Each contract maps these to its own `Error` enum via `From` or a match
/// arm, so no `contracterror` attribute is needed here.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RbacError {
    /// The caller does not hold the required role or `Admin`.
    NotAuthorized,
    /// Refused to revoke the last `Admin`, which would freeze governance.
    LastAdmin,
}

// ── Core helpers ──────────────────────────────────────────────────────────

/// Whether `account` currently holds the role identified by `role_key`.
///
/// `role_key` is the composite `(role, account)` key used to test membership
/// — typically `DataKey::Role(RoleKey { role, account })`.
pub fn has_role<RK: StorageKey>(env: &Env, role_key: &RK) -> bool {
    env.storage().instance().has(role_key)
}

/// Number of accounts currently holding `Admin` (zero pre-initialization).
///
/// `admin_count_key` is the key under which the count is stored — typically
/// `DataKey::AdminCount`.
pub fn admin_count<AK: StorageKey>(env: &Env, admin_count_key: &AK) -> u32 {
    env.storage()
        .instance()
        .get(admin_count_key)
        .unwrap_or(0u32)
}

/// Grants the role identified by `role_key` to an account.
///
/// - `role_key`:       composite `(role, account)` key
/// - `admin_count_key`: key for the admin counter (incremented when `is_admin == true`)
/// - `members_key`:    key for the `Vec<Address>` membership index
/// - `is_admin`:       `true` when the role being granted is `Admin`
/// - `account`:        the address being granted the role
///
/// Idempotent: re-granting a held role is a no-op.
/// Returns `true` if the role was newly granted, `false` if already held.
pub fn grant<RK, AK, MK>(
    env: &Env,
    role_key: &RK,
    admin_count_key: &AK,
    members_key: &MK,
    is_admin: bool,
    account: &Address,
) -> bool
where
    RK: StorageKey,
    AK: StorageKey,
    MK: StorageKey,
{
    if has_role(env, role_key) {
        return false;
    }
    env.storage().instance().set(role_key, &true);
    if is_admin {
        let next = admin_count(env, admin_count_key) + 1;
        env.storage().instance().set(admin_count_key, &next);
    }
    // Maintain the role-members index.
    let mut members: SorobanVec<Address> = env
        .storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env));
    members.push_back(account.clone());
    env.storage().instance().set(members_key, &members);
    true
}

/// Revokes the role identified by `role_key` from an account.
///
/// - `role_key`:        composite `(role, account)` key
/// - `admin_count_key`: key for the admin counter (decremented when `is_admin == true`)
/// - `members_key`:     key for the `Vec<Address>` membership index
/// - `is_admin`:        `true` when the role being revoked is `Admin`
/// - `account`:         the address whose role is being revoked
///
/// Idempotent: revoking a role not held returns `Ok(false)`.
/// Refuses to revoke the last Admin (`Err(RbacError::LastAdmin)`).
/// Returns `Ok(true)` when the role was actually removed.
///
/// **Note:** Oracle-specific side-effects (e.g. removing a `PriceFeeder`
/// from the submitters set) belong at the call site, gated on the
/// `Ok(true)` return value.
pub fn revoke<RK, AK, MK>(
    env: &Env,
    role_key: &RK,
    admin_count_key: &AK,
    members_key: &MK,
    is_admin: bool,
    account: &Address,
) -> Result<bool, RbacError>
where
    RK: StorageKey,
    AK: StorageKey,
    MK: StorageKey,
{
    if !has_role(env, role_key) {
        return Ok(false);
    }
    if is_admin {
        let count = admin_count(env, admin_count_key);
        if count <= 1 {
            return Err(RbacError::LastAdmin);
        }
        env.storage().instance().set(admin_count_key, &(count - 1));
    }
    env.storage().instance().remove(role_key);
    // Rebuild the members index without this account.
    let members: SorobanVec<Address> = env
        .storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env));
    let mut updated = SorobanVec::new(env);
    for i in 0..members.len() {
        let m = members.get(i).unwrap();
        if m != *account {
            updated.push_back(m);
        }
    }
    env.storage().instance().set(members_key, &updated);
    Ok(true)
}

/// Returns every account currently holding a role.
///
/// `members_key` is the `DataKey::RoleMembers(role)` variant maintained by
/// `grant` and `revoke`. Returns an empty vector when no accounts hold the role.
pub fn role_members<MK: StorageKey>(env: &Env, members_key: &MK) -> SorobanVec<Address> {
    env.storage()
        .instance()
        .get(members_key)
        .unwrap_or(SorobanVec::new(env))
}

/// Requires that `caller` authorized the transaction and holds the role
/// identified by `role_key` **or** is an `Admin` (identified by `admin_key`).
///
/// - `caller`:    the signer being checked
/// - `role_key`:  composite `(role, caller)` key for the specific role
/// - `admin_key`: composite `(Admin, caller)` key for the Admin super-user check
/// - `on_success`: optional callback invoked after a successful auth check,
///   used by `DripGovernor` to bump instance TTL. Pass `None` from contexts
///   where TTL is managed at the entry-point level (e.g. `DripOracle`).
///
/// Returns `Ok(())` on success, `Err(RbacError::NotAuthorized)` otherwise.
pub fn require_role_or_admin<RK: StorageKey>(
    env: &Env,
    caller: &Address,
    role_key: &RK,
    admin_key: &RK,
    on_success: Option<fn(&Env)>,
) -> Result<(), RbacError> {
    caller.require_auth();
    if has_role(env, admin_key) || has_role(env, role_key) {
        if let Some(bump) = on_success {
            bump(env);
        }
        Ok(())
    } else {
        Err(RbacError::NotAuthorized)
    }
}

/// Requires that `caller` authorized the transaction and holds the role
/// identified by `role_key` exactly (no Admin fallback).
///
/// Used by `DripGovernor::require_role` for operations that must be
/// performed by the exact role holder — e.g. Admin-only `grant_role`.
/// Pass `on_success` to bump TTL on success.
///
/// Returns `Ok(())` on success, `Err(RbacError::NotAuthorized)` otherwise.
pub fn require_role<RK: StorageKey>(
    env: &Env,
    caller: &Address,
    role_key: &RK,
    on_success: Option<fn(&Env)>,
) -> Result<(), RbacError> {
    caller.require_auth();
    if has_role(env, role_key) {
        if let Some(bump) = on_success {
            bump(env);
        }
        Ok(())
    } else {
        Err(RbacError::NotAuthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, contracttype, symbol_short};

    // ── Contract frame ────────────────────────────────────────────────────────
    //
    // `rbac` is storage-only shared code: it owns no `#[contract]` of its own,
    // so the suite has to supply the contract frame that `soroban_sdk` requires
    // before any instance-storage access. The host below is registered per test
    // purely so the frame has instance storage to open — `Env::as_contract`
    // fails with a `MissingValue` host error against an address that was never
    // registered.

    #[contract]
    struct RbacTestHost;

    #[contractimpl]
    impl RbacTestHost {
        /// Never invoked. Present only because registering a contract requires
        /// at least one callable entry point.
        pub fn noop(_env: Env) {}
    }

    fn in_contract<T>(env: &Env, f: impl FnOnce() -> T) -> T {
        env.as_contract(&env.register_contract(None, RbacTestHost), f)
    }

    // ── Minimal key space ─────────────────────────────────────────────────────
    //
    // `rbac` is generic over its storage keys, so the tests supply their own
    // `#[contracttype]` key enum rather than reaching for a consumer contract's
    // `DataKey`. That is the whole point of the issue: this suite must be
    // independent of `DripGovernor` and `DripOracle` while still exercising the
    // exact key shapes they pass in.

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestRole {
        Admin,
        Operator,
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestRoleKey {
        role: TestRole,
        account: Address,
    }

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    enum TestKey {
        Role(TestRoleKey),
        AdminCount,
        RoleMembers(TestRole),
    }

    const ADMIN_COUNT: TestKey = TestKey::AdminCount;

    fn role_key(role: TestRole, account: &Address) -> TestKey {
        TestKey::Role(TestRoleKey {
            role,
            account: account.clone(),
        })
    }

    fn members_key(role: &TestRole) -> TestKey {
        TestKey::RoleMembers(role.clone())
    }

    fn grant_role(env: &Env, role: TestRole, account: &Address, is_admin: bool) -> bool {
        grant(
            env,
            &role_key(role.clone(), account),
            &ADMIN_COUNT,
            &members_key(&role),
            is_admin,
            account,
        )
    }

    fn revoke_role(
        env: &Env,
        role: TestRole,
        account: &Address,
        is_admin: bool,
    ) -> Result<bool, RbacError> {
        revoke(
            env,
            &role_key(role.clone(), account),
            &ADMIN_COUNT,
            &members_key(&role),
            is_admin,
            account,
        )
    }

    /// Stand-in for `DripGovernor`'s `ttl::bump`: records that the
    /// `on_success` hook actually ran. Observing the real TTL would couple
    /// these unit tests to ledger mechanics that `governor` already covers.
    fn mark_success(env: &Env) {
        env.storage()
            .instance()
            .set(&symbol_short!("bumped"), &true);
    }

    fn success_recorded(env: &Env) -> bool {
        env.storage().instance().has(&symbol_short!("bumped"))
    }

    // ── has_role ──────────────────────────────────────────────────────────────

    #[test]
    fn has_role_is_false_before_any_grant() {
        let env = Env::default();
        let alice = Address::generate(&env);

        in_contract(&env, || {
            assert!(!has_role(&env, &role_key(TestRole::Operator, &alice)));
        });
    }

    // ── admin_count ───────────────────────────────────────────────────────────

    #[test]
    fn admin_count_is_zero_before_initialization() {
        let env = Env::default();

        in_contract(&env, || {
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 0);
        });
    }

    #[test]
    fn only_admin_grants_increment_the_admin_count() {
        let env = Env::default();
        let operator = Address::generate(&env);
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Operator, &operator, false);
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 0);

            grant_role(&env, TestRole::Admin, &admin, true);
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 1);

            // Re-granting a role that is already held must not double count.
            grant_role(&env, TestRole::Admin, &admin, true);
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 1);
        });
    }

    // ── grant ─────────────────────────────────────────────────────────────────

    #[test]
    fn grant_marks_membership_and_reports_the_transition() {
        let env = Env::default();
        let alice = Address::generate(&env);

        in_contract(&env, || {
            assert!(grant_role(&env, TestRole::Operator, &alice, false));
            assert!(has_role(&env, &role_key(TestRole::Operator, &alice)));
        });
    }

    #[test]
    fn grant_is_idempotent_and_does_not_duplicate_members() {
        let env = Env::default();
        let alice = Address::generate(&env);

        in_contract(&env, || {
            assert!(grant_role(&env, TestRole::Operator, &alice, false));
            assert!(!grant_role(&env, TestRole::Operator, &alice, false));

            let members = role_members(&env, &members_key(&TestRole::Operator));
            assert_eq!(members.len(), 1);
            assert_eq!(members.get(0).unwrap(), alice);
        });
    }

    #[test]
    fn grant_appends_members_in_grant_order() {
        let env = Env::default();
        let first = Address::generate(&env);
        let second = Address::generate(&env);
        let third = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Operator, &first, false);
            grant_role(&env, TestRole::Operator, &second, false);
            grant_role(&env, TestRole::Operator, &third, false);

            let members = role_members(&env, &members_key(&TestRole::Operator));
            assert_eq!(members.len(), 3);
            assert_eq!(members.get(0).unwrap(), first);
            assert_eq!(members.get(1).unwrap(), second);
            assert_eq!(members.get(2).unwrap(), third);
        });
    }

    // ── revoke ────────────────────────────────────────────────────────────────

    #[test]
    fn revoke_of_a_role_that_is_not_held_reports_no_change() {
        let env = Env::default();
        let alice = Address::generate(&env);

        in_contract(&env, || {
            assert_eq!(
                revoke_role(&env, TestRole::Operator, &alice, false),
                Ok(false)
            );
            assert!(!has_role(&env, &role_key(TestRole::Operator, &alice)));
            assert_eq!(
                role_members(&env, &members_key(&TestRole::Operator)).len(),
                0
            );
        });
    }

    #[test]
    fn revoke_removes_the_role_and_only_that_member() {
        let env = Env::default();
        let first = Address::generate(&env);
        let middle = Address::generate(&env);
        let last = Address::generate(&env);

        in_contract(&env, || {
            for account in [&first, &middle, &last] {
                grant_role(&env, TestRole::Operator, account, false);
            }

            assert_eq!(
                revoke_role(&env, TestRole::Operator, &middle, false),
                Ok(true)
            );

            assert!(has_role(&env, &role_key(TestRole::Operator, &first)));
            assert!(!has_role(&env, &role_key(TestRole::Operator, &middle)));
            assert!(has_role(&env, &role_key(TestRole::Operator, &last)));

            // The index is rebuilt rather than truncated, so order survives.
            let members = role_members(&env, &members_key(&TestRole::Operator));
            assert_eq!(members.len(), 2);
            assert_eq!(members.get(0).unwrap(), first);
            assert_eq!(members.get(1).unwrap(), last);
        });
    }

    #[test]
    fn revoke_decrements_the_admin_count_while_another_admin_remains() {
        let env = Env::default();
        let first = Address::generate(&env);
        let second = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &first, true);
            grant_role(&env, TestRole::Admin, &second, true);
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 2);

            assert_eq!(revoke_role(&env, TestRole::Admin, &second, true), Ok(true));
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 1);
            assert!(has_role(&env, &role_key(TestRole::Admin, &first)));
        });
    }

    #[test]
    fn revoke_refuses_to_remove_the_last_admin() {
        let env = Env::default();
        let only_admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &only_admin, true);

            assert_eq!(
                revoke_role(&env, TestRole::Admin, &only_admin, true),
                Err(RbacError::LastAdmin)
            );

            // This is the guarantee that stops the protocol from freezing, so
            // assert every piece of state is left untouched.
            assert!(has_role(&env, &role_key(TestRole::Admin, &only_admin)));
            assert_eq!(admin_count(&env, &ADMIN_COUNT), 1);
            assert_eq!(role_members(&env, &members_key(&TestRole::Admin)).len(), 1);
        });
    }

    // ── role_members ──────────────────────────────────────────────────────────

    #[test]
    fn role_members_is_empty_for_an_unset_role() {
        let env = Env::default();

        in_contract(&env, || {
            assert_eq!(
                role_members(&env, &members_key(&TestRole::Operator)).len(),
                0
            );
        });
    }

    /// The module docs claim the membership index lives in `instance()` storage
    /// (the issue #345 fix). Pin the tier, so a regression to a different tier
    /// fails here instead of only surfacing as a surprise TTL expiry on-chain.
    #[test]
    fn role_state_is_written_to_instance_storage() {
        let env = Env::default();
        let alice = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Operator, &alice, false);

            let role = role_key(TestRole::Operator, &alice);
            let members = members_key(&TestRole::Operator);
            assert!(env.storage().instance().has(&role));
            assert!(env.storage().instance().has(&members));
            assert!(!env.storage().persistent().has(&role));
            assert!(!env.storage().persistent().has(&members));

            // The admin counter is written lazily, so a non-admin grant must
            // not create it; an Admin grant must, in the same tier.
            assert!(!env.storage().instance().has(&ADMIN_COUNT));
            grant_role(&env, TestRole::Admin, &alice, true);
            assert!(env.storage().instance().has(&ADMIN_COUNT));
            assert!(!env.storage().persistent().has(&ADMIN_COUNT));
        });
    }

    // ── require_role_or_admin ─────────────────────────────────────────────────

    #[test]
    fn require_role_or_admin_accepts_a_role_holder() {
        let env = Env::default();
        env.mock_all_auths();
        let operator = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Operator, &operator, false);

            assert_eq!(
                require_role_or_admin(
                    &env,
                    &operator,
                    &role_key(TestRole::Operator, &operator),
                    &role_key(TestRole::Admin, &operator),
                    None,
                ),
                Ok(())
            );
        });
    }

    #[test]
    fn require_role_or_admin_accepts_an_admin_without_the_specific_role() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &admin, true);

            // The super-user holds no Operator role but must still pass.
            assert!(!has_role(&env, &role_key(TestRole::Operator, &admin)));
            assert_eq!(
                require_role_or_admin(
                    &env,
                    &admin,
                    &role_key(TestRole::Operator, &admin),
                    &role_key(TestRole::Admin, &admin),
                    None,
                ),
                Ok(())
            );
        });
    }

    #[test]
    fn require_role_or_admin_rejects_an_unrelated_caller() {
        let env = Env::default();
        env.mock_all_auths();
        let stranger = Address::generate(&env);

        in_contract(&env, || {
            assert_eq!(
                require_role_or_admin(
                    &env,
                    &stranger,
                    &role_key(TestRole::Operator, &stranger),
                    &role_key(TestRole::Admin, &stranger),
                    None,
                ),
                Err(RbacError::NotAuthorized)
            );
        });
    }

    #[test]
    fn require_role_or_admin_invokes_on_success_only_when_authorized() {
        let env = Env::default();
        env.mock_all_auths();
        let operator = Address::generate(&env);
        let stranger = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Operator, &operator, false);

            assert_eq!(
                require_role_or_admin(
                    &env,
                    &operator,
                    &role_key(TestRole::Operator, &operator),
                    &role_key(TestRole::Admin, &operator),
                    Some(mark_success),
                ),
                Ok(())
            );
            assert!(success_recorded(&env));

            // A rejected caller must not run the hook, or the TTL would be
            // bumped on a failed authorization.
            env.storage().instance().remove(&symbol_short!("bumped"));
            assert_eq!(
                require_role_or_admin(
                    &env,
                    &stranger,
                    &role_key(TestRole::Operator, &stranger),
                    &role_key(TestRole::Admin, &stranger),
                    Some(mark_success),
                ),
                Err(RbacError::NotAuthorized)
            );
            assert!(!success_recorded(&env));
        });
    }

    // ── require_role ──────────────────────────────────────────────────────────

    /// The whole difference between the two gates: `require_role` must **not**
    /// fall back to the Admin super-user. That is what keeps `grant_role`
    /// itself Admin-only instead of reachable by any role holder.
    ///
    /// Split from the positive case below rather than asserting a rejection and
    /// an acceptance in one test: both would call `admin.require_auth()`, and a
    /// second `require_auth` for the same address inside one invocation fails
    /// with `Auth, ExistingValue` before the gate is even reached.
    #[test]
    fn require_role_rejects_an_admin_that_lacks_the_exact_role() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &admin, true);

            assert_eq!(
                require_role(&env, &admin, &role_key(TestRole::Operator, &admin), None),
                Err(RbacError::NotAuthorized)
            );
        });
    }

    #[test]
    fn require_role_accepts_the_exact_role_holder() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &admin, true);

            assert_eq!(
                require_role(&env, &admin, &role_key(TestRole::Admin, &admin), None),
                Ok(())
            );
        });
    }

    #[test]
    fn require_role_skips_on_success_when_the_exact_role_is_missing() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &admin, true);

            assert_eq!(
                require_role(
                    &env,
                    &admin,
                    &role_key(TestRole::Operator, &admin),
                    Some(mark_success),
                ),
                Err(RbacError::NotAuthorized)
            );
            assert!(!success_recorded(&env));
        });
    }

    #[test]
    fn require_role_invokes_on_success_for_the_exact_role() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        in_contract(&env, || {
            grant_role(&env, TestRole::Admin, &admin, true);

            assert_eq!(
                require_role(
                    &env,
                    &admin,
                    &role_key(TestRole::Admin, &admin),
                    Some(mark_success),
                ),
                Ok(())
            );
            assert!(success_recorded(&env));
        });
    }
}
