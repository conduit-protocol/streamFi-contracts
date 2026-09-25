# ADR-009: Two-step propose/accept ownership transfer

**Status:** Accepted
**Date:** 2026-09

---

## Context

Both `DripGovernor` and `TokenVault` need a mechanism for the current
owner/authority to hand control to a new address. A single-step
`set_owner(new)` is simple but dangerous: a typo or incorrect address
permanently locks the contract. This is a well-known footgun in on-chain
governance (OpenZeppelin's `Ownable2Step` exists for exactly this reason on
EVM).

Both contracts independently implement the same two-step pattern:

| Contract | Step 1 | Step 2 | Storage keys |
|----------|--------|--------|--------------|
| `DripGovernor` | `propose_authority(caller, new_authority)` | `accept_authority(caller)` | `PendingAuthority`, `PendingAuthorityProposer` |
| `TokenVault` | `propose_owner(caller, new_owner)` | `accept_owner(caller)` | `PendingOwner`, `PendingOwnerProposer` |

---

## Decision

Ownership transfer uses a two-step propose/accept pattern. The current
owner proposes a new address; the proposed address must explicitly accept.
Ownership only changes when both steps complete.

Each contract implements this independently rather than extracting a shared
primitive into `drip_common`.

---

## Rationale

**Safety against misaddressed transfers.** The accepting address must call
`accept_*` with `require_auth()`, proving it controls the private key. If the
proposer specifies an incorrect address, the accept call simply never arrives
and ownership stays with the original party.

**No shared primitive (intentional).** Governor transfers authority over
protocol parameters; TokenVault transfers ownership of a standalone custody
contract. The lifecycle implications differ (governor authority affects
downstream factory behaviour; vault ownership does not). Keeping the
implementations local avoids coupling their release cycles and lets each
contract add domain-specific guards (e.g., the governor validates the new
authority doesn't already hold Admin role).

**Implementation consistency.** Despite being independent, both implementations
follow the same structure:
1. Proposer calls `propose_*`, which stores the pending address and the
   proposer's address.
2. Acceptor calls `accept_*`, which validates `caller == pending`, completes
   the transfer, and cleans up pending state.
3. A new proposal overwrites any existing pending proposal (implicit revoke).

---

## Known Limitations

The following gaps are documented as explicit follow-up items:

### No explicit revoke

Neither contract exposes a `revoke_proposed_*` function. The only way to cancel
a pending proposal is to propose a different address (which overwrites the
pending state). This means:
- A stale proposal to an address the proposer no longer wants to transfer to
  remains active until overwritten.
- The proposed address can accept at any time, even if the proposer's intent
  has changed.

**Follow-up:** Consider adding `revoke_proposed_authority` /
`revoke_proposed_owner` that clears pending state and emits a cancellation
event.

### No expiry / timeout

Pending proposals have no TTL. A proposal made today can be accepted months
later. In the governor's case this is especially sensitive: the security posture
of the proposed address may have changed since the proposal was made.

**Follow-up:** Consider adding an `expires_at` ledger sequence or timestamp to
the pending record, after which `accept_*` reverts. This adds one storage field
and one comparison.

---

## Consequences

- **New contracts adopting this pattern** should follow the same two-step
  structure and should address the revoke and expiry gaps from the start rather
  than inheriting them.
- **Auditors** should check both contracts for the same class of issues, since
  the pattern is duplicated rather than shared.
- **The implicit-revoke-via-reproposal behaviour** is safe but surprising.
  It should be documented in user-facing API references so callers don't assume
  proposals are append-only.
