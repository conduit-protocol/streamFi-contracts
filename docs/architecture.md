# Architecture

A technical walkthrough of how the three Conduit contracts fit together.

---

## Overview

```
  Caller (wallet)
       │
       │  create_stream(sender, recipient, token, deposit, rate, start, end, clawback)
       ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │                         DripFactory                                  │
  │                                                                      │
  │  1. require_auth(sender)                                             │
  │  2. validate params                                                  │
  │  3. token.transfer(sender → factory, deposit)                       │
  │  4. deploy DripStream WASM with deterministic salt                  │
  │  5. invoke DripStream::initialize(sender, recipient, token, ...)    │
  │  6. token.transfer(factory → stream, deposit)                       │
  │  7. index: BySender[sender] += stream_id                            │
  │           ByRecipient[recipient] += stream_id                       │
  │           StreamAddr[stream_id] = stream_address                    │
  │                                                                      │
  │  Returns: stream_id (u64)                                            │
  └─────────────────────────────────────────────────────────────────────┘
                                │
                       deploys and funds
                                │
                                ▼
  ┌─────────────────────────────────────────────────────────────────────┐
  │                        DripStream                                    │
  │  (one contract instance per stream)                                  │
  │                                                                      │
  │  State machine:                                                      │
  │                                                                      │
  │    ACTIVE ──pause()──► PAUSED                                        │
  │      │  ◄──resume()──    │                                           │
  │      │                   │                                           │
  │    cancel()           cancel()                                       │
  │      │                   │                                           │
  │      ▼                   ▼                                           │
  │   CANCELLED           CANCELLED                                      │
  │                                                                      │
  │  Recipient calls withdraw() any time stream is ACTIVE or PAUSED.    │
  │  cancel() atomically settles both parties.                           │
  └─────────────────────────────────────────────────────────────────────┘
```

---

## Contract Responsibilities

### DripFactory

The factory is a singleton deployed once per network. It owns no token balance for longer than one transaction — funds enter from the sender, then immediately forward to the new stream contract.

**Key invariants:**
- `StreamCount` is monotonically increasing; IDs are never reused.
- `StreamAddr(id)` is set exactly once at creation and never mutated.
- `BySender` and `ByRecipient` indices append-only.
- Factory uses `persistent()` storage for all growing collections to avoid instance storage limits.

### DripStream

Each stream is a fully independent contract. Isolation is intentional — a bug in one stream cannot affect another, and the factory becomes non-critical after deployment (streams are self-contained).

**Withdrawable calculation:**

```
                       ┌──────────── paused_at ── ─ ─ ─
  start_time           │                                  end_time
  │                    │ paused                            │
  ├────────────────────┤▓▓▓▓▓▓▓▓▓▓▓▓▓│─────────────────────┤
  │                    │              │                     │
  │◄── elapsed ───────►│              │◄── elapsed cont. ──►│
  │                    resume_at      now
  │
  withdrawable = rate × (total_elapsed excluding paused time) − withdrawn
```

The pause/resume implementation shifts `start_time` forward by the paused duration on resume. This means `streamed_amount` always uses the simple formula `rate × (now − start_time)` — the paused time is absorbed into the shifted origin rather than tracked separately.

**Cancel settlement:**

When `cancel()` is called, both parties are settled in the same transaction:
- `owed_to_recipient = min(streamed − withdrawn, balance)` → transferred to recipient
- `refund_to_sender = balance − owed_to_recipient` → transferred to sender

After `Cancelled = true`, `withdraw()` is blocked. The atomic settlement in `cancel()` is therefore mandatory — the recipient cannot retrieve tokens any other way.

### DripGovernor

The governor holds mutable protocol parameters. In the current version it is controlled by a single `authority` address (intended to be a multisig). In a future release, governance will transition to on-chain token voting.

The factory reads governor parameters at stream creation time. The governor does not hold any token balance.

---

## Storage Tiers

Soroban has three storage tiers. Each has different persistence and TTL semantics.

| Tier | Used for | TTL policy |
|------|----------|------------|
| `instance()` | Contract config (wasm hash, governor address, stream count) | Tied to contract instance TTL |
| `persistent()` | Growing indices (`StreamAddr`, `BySender`, `ByRecipient`), all stream state | Per-entry TTL; must be extended by callers |
| `temporary()` | Not used | Auto-expires |

**TTL management (production note):** Every read of a `persistent()` entry should be accompanied by a TTL extension call:
```rust
env.storage().persistent().extend_ttl(&key, ledgers_to_live, ledgers_to_live);
```
The scaffold omits TTL extension for readability. Production deployments must add TTL bumps on every state access, or stream contracts risk expiration.

---

## Deployment Flow

```
deploy.sh local
    │
    ├─ cargo build --target wasm32-unknown-unknown --release
    │
    ├─ stellar contract upload drip_stream.wasm   → STREAM_WASM_HASH
    ├─ stellar contract upload drip_factory.wasm  → FACTORY_WASM_HASH
    ├─ stellar contract upload drip_governor.wasm → GOVERNOR_WASM_HASH
    │
    ├─ stellar contract deploy drip_governor.wasm → GOVERNOR_ID
    ├─ stellar contract deploy drip_factory.wasm  → FACTORY_ID
    │
    ├─ DripGovernor::initialize(authority, fee_recipient, FACTORY_ID)
    └─ DripFactory::initialize(STREAM_WASM_HASH, GOVERNOR_ID)
```

After deployment, users interact only with `DripFactory` (to create streams) and directly with their `DripStream` instances (to withdraw, pause, cancel).

---

## Per-Stream Contract Rationale

See [ADR-001](./adr/001-per-contract-per-stream.md).

## Cancel Settlement Design

See [ADR-002](./adr/002-cancel-settles-atomically.md).
