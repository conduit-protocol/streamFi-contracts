# ADR-008: Oracle defines its own Role enum

**Status:** Accepted
**Date:** 2026-09

---

## Context

`drip_common::rbac` (in `contracts/common/src/rbac.rs`) provides generic,
storage-tier-agnostic RBAC helpers (`grant`, `revoke`, `has_role`,
`require_role_or_admin`, etc.) that are parameterised over arbitrary storage-key
types. Both `DripGovernor` and `DripOracle` use these helpers.

However, each contract defines its **own** `Role` enum rather than importing a
shared one:

| Contract | Roles |
|----------|-------|
| `DripGovernor` | `Admin`, `FeeManager`, `RateManager`, `Pauser` |
| `DripOracle` | `Admin`, `PriceFeeder`, `Pauser` |

This ADR records why the divergence is intentional.

---

## Decision

Each contract defines a local `Role` enum whose variants match its own
domain-specific responsibilities. `drip_common::rbac` remains role-agnostic: it
stores and queries role grants but never names or interprets the roles
themselves.

---

## Rationale

**Domain-specific roles.** `PriceFeeder` is meaningless to the governor;
`FeeManager` and `RateManager` are meaningless to the oracle. A shared enum
would force every contract to carry variants it never uses, and `#[contracttype]`
enums become part of the on-chain ABI -- unused variants waste discriminant
space and confuse integrators reading the published schema.

**Non-superuser semantics differ.** The oracle intentionally gates
`submit_price` with `require_role` (exact match) rather than
`require_role_or_admin`. An `Admin` alone cannot inject price observations;
it must explicitly grant itself `PriceFeeder` first -- an auditable on-chain
action. This semantic constraint is expressed naturally when the contract owns
its own `Role` type and chooses which rbac helper to call per-endpoint. A shared
enum would obscure this per-contract policy.

**TTL callback strategy differs.** `DripGovernor` passes
`Some(ttl::bump)` as the `on_success` callback to `require_role_or_admin`,
bumping TTL inside the rbac check. `DripOracle` passes `None` and bumps TTL at
the entry-point level before delegating. These per-contract choices are
orthogonal to role identity and would not benefit from a shared type.

**Compile-time isolation.** Each contract compiles independently with only a
`drip_common` dependency. A shared `Role` enum living in `drip_common` would
couple the release cadence of all role-bearing contracts and require a common
release whenever any single contract adds a role variant.

---

## Consequences

- **Pattern, not type, is shared.** New contracts adopting RBAC should define
  their own `Role` enum and wrap `drip_common::rbac` the same way `DripOracle`
  and `DripGovernor` do. The wrappers (`role_key`, `has_role`,
  `grant_role_inner`, `revoke_role_inner`, `require_role_or_admin`) are thin
  and follow a consistent template across both contracts.
- **No single source of truth for "all roles in the protocol."** An auditor
  must inspect each contract's `Role` enum individually. This is an accepted
  cost of isolation.
- **Role-specific side-effects stay local.** For example, the oracle's
  `revoke_role_inner` calls `remove_submitter` when a `PriceFeeder` is revoked
  to purge stale submission data. This domain logic belongs in the oracle, not
  in a shared module.
