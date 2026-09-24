# ADR-004: TokenVault is out of scope for the streaming protocol

**Status:** Accepted
**Date:** 2026-08

---

## Context

`contracts/token-vault/` is a full workspace member with its own error type,
storage module, events and 35 tests. It is built by `cargo build --all`, tested
by `cargo test`, and deployed by `scripts/deploy.sh`.

It is also completely disconnected from the protocol:

- `DripStream`, `DripFactory` and `DripGovernor` contain no reference to it —
  no import, no cross-contract call, no stored address.
- It appeared in neither `docs/architecture.md` nor any ADR.
- Its interface has no notion of streams, rates or schedules. It is an
  owner-controlled vault: `deposit`, `withdraw`, `max_limit`, an optional
  operator, and a pause switch.

That combination is the problem. A contract that builds, tests and deploys
alongside the protocol reads as part of it. Someone auditing "the streaming
protocol" has no way to tell from the repository whether TokenVault is a
component they must review, dead code they can skip, or a planned escrow
backend whose absence from the call graph is a bug.

---

## Decision

TokenVault stays in the workspace and is documented as **an independent
contract that lives in this repository but is not part of the streaming
protocol**.

It is not removed.

---

## Rationale

**Why document rather than delete.** Removal is irreversible in review terms:
if it is a planned escrow backend, deleting it discards working, tested code
and the design intent behind it. Documenting costs nothing and resolves the
actual harm, which is ambiguity rather than the code's existence. The original
issue offered either option; this is the one that cannot destroy information.

**Why not wire it into the protocol.** Stream deposits currently flow
sender → factory → stream contract, and each stream holds its own balance.
That is the whole point of ADR-001: per-stream isolation, so a bug in one
stream cannot reach another. Routing deposits through a shared vault would
reintroduce exactly the shared-state failure mode ADR-001 exists to avoid, and
is not a change to make incidentally.

**Why it still matters for audit scope.** `deploy.sh` deploys it. A deployed
instance can therefore exist on a network with an owner, a balance and a pause
switch, even though no protocol contract will ever call it. An audit scoped to
"contracts deployed from this repository" must include it; one scoped to "the
streaming protocol" may exclude it. That distinction now has a written answer.

---

## Consequences

- `docs/architecture.md` carries a TokenVault section stating it is not part of
  the protocol.
- Security reviews can scope explicitly, citing this ADR.
- If TokenVault is later adopted as an escrow backend, that is a new ADR
  superseding this one — and it will need to address the ADR-001 isolation
  tradeoff directly.
- If it is instead confirmed to be an experiment, removing it from the
  workspace becomes a follow-up with a recorded rationale rather than a
  judgement call made in a pull request.
