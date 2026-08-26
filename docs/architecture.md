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
  │      │  ◄──resume()──    │  ──force_cancel()──►  CANCELLED           │
  │      │                   │   (recipient-only, 30d after pause)       │
  │    cancel()           cancel()                                       │
  │      │                   │                                           │
  │      ▼                   ▼                                           │
  │   CANCELLED           CANCELLED                                      │
  │                                                                      │
  │  pause/resume/cancel/top_up/clawback/extend_duration may be called   │
  │  by the sender OR their delegated operator (see "Operator            │
  │  Delegation" below) — every transition arrow above implicitly        │
  │  includes that alternate caller.                                     │
  │                                                                      │
  │  extend_duration()/top_up_and_extend() don't change ACTIVE/PAUSED/   │
  │  CANCELLED status — they mutate end_time (and, for the latter,       │
  │  deposit balance) in place while the stream stays ACTIVE.            │
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

**Recipient-only escape hatches:**

- `force_cancel()` lets the recipient settle unilaterally if the sender pauses the stream and never resumes it — without this, a malicious or abandoned sender could freeze a paused stream indefinitely and hold the recipient's earned-but-unwithdrawn balance hostage. Guarded by a hardcoded 30-day threshold (`PAUSE_THRESHOLD_SECS`) measured from `paused_at`; settles identically to `cancel()`.
- `transfer_recipient(new_recipient)` reassigns the recipient address. Any balance already earned stays claimable by whoever holds the role — it isn't tied to the original recipient's identity. The sender is not notified on-chain (an indexer watching the `xfer_rec` event is the intended integration point).

**Operator delegation:**

The sender can delegate a fixed set of sender-level actions to another address via `set_operator(caller, operator)`, without handing over `sender` itself (which stays fixed for the life of the stream). The delegated actions are exactly: `pause`, `resume`, `cancel`, `top_up`, `clawback`, and `extend_duration` (and by extension `top_up_and_extend`, which combines the last two). Every one of these entry points authorizes via a shared `require_sender_or_operator` check — the sender or the current operator, not both required.

The operator has no power over `withdraw` (recipient-only, unaffected by this mechanism) or `transfer_recipient` (sender-only, not delegatable). Only the original `sender` may call `set_operator`/`revoke_operator` — an operator cannot re-delegate itself, and delegation doesn't survive a `revoke_operator` call or a `cancel`/`force_cancel` settlement. `operator()` is a read-only accessor for the currently-delegated address, if any.

**Duration extension:**

`extend_duration(caller, extra_time_seconds)` pushes `end_time` forward by `extra_time_seconds`, pulling the exact required deposit (`rate_per_second × extra_time_seconds`) from the sender in the same call. `top_up_and_extend(caller, amount, extra_time_seconds)` does the same end_time push alongside an independently-sized `amount` deposit, so a sender isn't forced to deposit exactly the rate-implied amount when extending. Neither works on an open-ended stream (`end_time == 0`) — use `top_up` alone in that case.

### DripGovernor

The governor holds mutable protocol parameters. In the current version it is controlled by a single `authority` address (intended to be a multisig). In a future release, governance will transition to on-chain token voting.

The governor does not hold any token balance.

`DripFactory::create_stream` cross-contract-calls `DripGovernor::config()` to enforce `max_rate_per_second`, `min_duration_seconds`, and `max_duration_seconds` (for fixed-duration streams), and `DripFactory::protocol_fee_bps()` reads `fee_bps` live from the governor — falling back to the 30bps default only if the factory itself hasn't been initialized yet.

---

## Storage Tiers

Soroban has three storage tiers. Each has different persistence and TTL semantics.

| Tier | Used for | TTL policy |
|------|----------|------------|
| `instance()` | Contract config (wasm hash, governor address, stream count) | Tied to contract instance TTL |
| `persistent()` | Growing indices (`StreamAddr`, `BySender`, `ByRecipient`), all stream state | Per-entry TTL; must be extended by callers |
| `temporary()` | Not used | Auto-expires |

**TTL management:** every state-mutating call on all three contracts extends instance TTL, and `DripFactory` additionally extends the `StreamAddr`/`BySender`/`ByRecipient` persistent entries on `create_stream`:
```rust
env.storage().instance().extend_ttl(threshold, extend_to);
env.storage().persistent().extend_ttl(&key, threshold, extend_to);
```
Pure read-only functions (`withdrawable`, `streamed_total`, `info`, `config`, `stream_address`, etc.) do not bump TTL themselves — an entry only stays alive if something actually mutates it. A long-idle stream that nobody touches can still expire; a `keep_alive`-style function anyone could call without mutating state remains a possible future addition.

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
