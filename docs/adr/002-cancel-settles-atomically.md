# ADR-002: cancel() settles both parties atomically

**Status:** Accepted  
**Date:** 2026-01

---

## Context

When a stream is cancelled, two balances need to be distributed:

1. Tokens earned but not yet withdrawn by the recipient.
2. Unstreamed tokens refunded to the sender.

We considered two approaches:

**Option A — Cancel leaves earned tokens claimable.** Set `Cancelled = true`, refund the sender's portion, but leave the recipient's earned portion in the contract for a subsequent `withdraw()` call.

**Option B — Cancel settles both parties in one transaction.** Transfer both the recipient's owed portion and the sender's refund atomically. After `cancel()`, the contract balance is zero.

---

## Decision

Option B — atomic settlement.

---

## Rationale

**Option A has a critical flaw:** If `withdraw()` is blocked after cancellation (which it must be to prevent double-counting), the recipient permanently loses their earned tokens if `cancel()` doesn't pay them out. If `withdraw()` is *not* blocked, an attacker (the sender) could cancel and immediately withdraw as recipient (if they control both sides), creating a race condition.

The only clean design is for `cancel()` to be the terminal, total-settlement operation:

```
cancel()
  → mark Cancelled
  → transfer owed_to_recipient to recipient
  → transfer refund_to_sender to sender
  → contract balance = 0
```

After this, no further token operations are possible on the stream. The `Cancelled` flag ensures `withdraw()`, `pause()`, `resume()`, `top_up()`, and `clawback()` all fail fast with `StreamCancelled`.

**Atomicity prevents race conditions.** Because Soroban transactions are atomic, the recipient cannot be in a state where the stream is being cancelled but their tokens are not yet transferred. Either the whole settlement happens or nothing does.

---

## Consequences

- `cancel()` may initiate two token transfers (to recipient and to sender). This is slightly more expensive in ledger fees than Option A's single transfer.
- The `withdraw()` function's `assert_not_cancelled` check is now a complete block — there is no valid post-cancellation withdrawal path. This is simpler to audit.
- Recipients do not need to monitor for cancellation events and rush to withdraw. The cancellation itself guarantees their payment.
