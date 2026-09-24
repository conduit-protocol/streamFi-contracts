# ADR-007: Batch transfer processor scope

**Status:** Accepted  
**Date:** 2026-09

---

## Context

The protocol needs a reusable execution boundary for processing bounded groups
of transfers. The batch operation has different concerns from stream creation:
it must enforce batch-size and arithmetic limits, protect its critical section,
and invalidate stale callback state. Putting that logic directly on
`DripFactory` would make the factory's deployment and registry responsibilities
depend on a separate batch-processing state machine.

We considered adding batch processing as another `DripFactory` method or
deploying it as a separate `BatchTransferProcessor` contract.

## Decision

Keep batch processing in a separate `BatchTransferProcessor` contract. The
processor owns only the bounded batch execution guard, state-version counter,
callback sequence, and transfer-total calculation. `DripFactory` remains the
protocol entry point for stream deployment and the stream registry.

## Rationale

**Separation of concerns.** Factory state tracks stream addresses and protocol
configuration; processor state tracks an individual batch's execution safety.
Keeping those state machines separate makes both contracts easier to audit and
upgrade independently.

**Independent limits.** Batch size, checked total calculations, and processor
locking can evolve without changing the factory's stream-creation interface or
its storage footprint.

**Failure isolation.** A rejected or stale batch cannot corrupt stream
registration state. The processor can clean up stale callbacks and return its
own errors without coupling those paths to stream deployment.

**Reusable boundary.** Other protocol components can call the processor for
bounded batch work without gaining access to the factory's deployment or
registry methods.

## Consequences

- Deployments and integrations must know the processor contract address when
  batch processing is enabled.
- Batch work adds a cross-contract boundary and therefore an additional call
  cost compared with an inline factory method.
- The processor is intentionally scoped to execution safety and aggregation;
  stream ownership, authorization, and registry updates remain with the
  contracts that own those concerns.
- Changes to batch limits or callback semantics can be made without changing
  `DripFactory`'s public stream API.
