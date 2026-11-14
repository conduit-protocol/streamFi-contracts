# ADR-004: TokenVault remains independent; stream withdrawals can be batched

**Status:** Accepted, amended by issue #841
**Date:** 2026-08

---

## Context

`contracts/token-vault/` is a full workspace member with its own error type,
storage module, events and 35 tests. It is built by `cargo build --all`, tested
by `cargo test`, and deployed by `scripts/deploy.sh`.

It remains a separate custody contract and is not used to hold stream funds.
However, a token-vault entry point can act as a transaction-level coordinator
for recipient-authorized withdrawals from multiple independent stream
contracts. That integration does not change where stream funds are held.

- `DripStream`, `DripFactory` and `DripGovernor` do not store its address or
  route deposits and settlements through it.
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

TokenVault stays in the workspace as an independent custody contract. It may
provide an explicit convenience call that invokes recipient-authorized
withdrawals across a bounded list of stream contracts; stream escrows remain
isolated and are never transferred through the vault.

It is not removed.

---

## Rationale

**Why document rather than delete.** Removal is irreversible in review terms:
if it is a planned escrow backend, deleting it discards working, tested code
and the design intent behind it. Documenting costs nothing and resolves the
actual harm, which is ambiguity rather than the code's existence. The original
issue offered either option; this is the one that cannot destroy information.

**Why the batch call does not centralize custody.** Stream deposits still flow
sender → factory → stream contract, and each stream holds its own balance.
The batch call only coordinates authorized `withdraw` invocations; it does not
move stream deposits into the vault or allow the vault to bypass stream-level
recipient authorization. A failure in any invocation reverts the transaction.

**Why it still matters for audit scope.** `deploy.sh` deploys it. A deployed
instance can therefore exist on a network with an owner, a balance and a pause
switch, even though no protocol contract will ever call it. An audit scoped to
"contracts deployed from this repository" must include it; one scoped to "the
streaming protocol" may exclude it. That distinction now has a written answer.

---

## Consequences

- `docs/architecture.md` describes the vault as an optional withdrawal
  coordinator as well as a standalone custody contract.
- Security reviews can scope explicitly, citing this ADR.
- If TokenVault is later adopted as an escrow backend, a new ADR must address
  the ADR-001 isolation tradeoff directly.
- If it is instead confirmed to be an experiment, removing it from the
  workspace becomes a follow-up with a recorded rationale rather than a
  judgement call made in a pull request.
