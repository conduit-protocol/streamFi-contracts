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

| Contract       | Step 1                                     | Step 2                     | Storage keys                                   |
| -------------- | ------------------------------------------ | -------------------------- | ---------------------------------------------- |
| `DripGovernor` | `propose_authority(caller, new_authority)` | `accept_authority(caller)` | `PendingAuthority`, `PendingAuthorityProposer` |
| `TokenVault`   | `propose_owner(caller, new_owner)`         | `accept_owner(caller)`     | `PendingOwner`, `PendingOwnerProposer`         |

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

TokenVault also stores the proposal timestamp and rejects acceptance after the
configured validity period, which defaults to seven days. The owner can change
that period; an expired transfer can be restarted by submitting a fresh
`propose_owner` call. DripGovernor proposals do not currently expire.

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

### Governor proposal expiry

DripGovernor pending authority proposals have no expiry. A proposal can be
accepted months later, after the security posture of the proposed address may
have changed.

**Follow-up:** Consider adding an expiry timestamp to the governor's pending
authority record, after which `accept_authority` reverts.

---

## Consequences

- **New contracts adopting this pattern** should follow the same two-step
  structure and address the revoke gap and any domain-specific expiry needs.
- **Auditors** should check both contracts for the same class of issues, since
  the pattern is duplicated rather than shared.
- **The implicit-revoke-via-reproposal behaviour** is safe but surprising.
  It should be documented in user-facing API references so callers don't assume
  proposals are append-only.
