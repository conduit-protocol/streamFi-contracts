# Security

Threat model, known limitations, and safe usage guidance for Conduit contracts.

> **Audit status:** Not yet audited. Do not deploy on Stellar Mainnet with real funds.

---

## Threat Model

### Assets at risk

- Token balances held inside `DripStream` contracts.
- In-flight deposits held by `DripFactory` for the duration of `create_stream` (one transaction, sub-second).

### Trusted parties

| Party | Trust level | What they can do |
|-------|-------------|-----------------|
| Stream sender | High | Created and funded the stream. Can pause, resume, cancel, top-up. Can clawback if enabled. |
| Stream recipient | Partial | Can only `withdraw()` — cannot pause, cancel, or access more than their earned balance. |
| DripGovernor authority | High | Can change fee rates and protocol parameters. Cannot touch stream balances. |
| DripFactory | Internal | Deploys streams and forwards deposits. Holds funds for ≤ 1 transaction. |

---

## Authentication

All sensitive operations are guarded by `address.require_auth()`. This delegates to Soroban's built-in auth framework, which verifies that the signed payload matches the contract invocation being authorised. There is no manual signature verification in the contracts.

Specifically:
- `cancel`, `pause`, `resume`, `top_up`, `clawback` → `sender.require_auth()`
- `withdraw` → `recipient.require_auth()`
- `create_stream` → `sender.require_auth()`
- All governor write functions → `authority.require_auth()`

---

## Reentrancy

Soroban's execution model is synchronous and single-threaded within a transaction. Cross-contract calls (e.g., `token.transfer`) cannot call back into the calling contract mid-execution. Reentrancy is structurally impossible.

For defence-in-depth, state mutations (setting `Cancelled = true`) are performed before token transfers in `cancel()`.

---

## Arithmetic

All arithmetic uses Rust's `checked_*` methods. Overflow returns `Error::ArithmeticOverflow` rather than wrapping silently. The `i128` type is used for all token amounts (matching Stellar's native representation).

---

## Known Limitations

### 1. No TTL management in scaffold

Soroban `persistent()` storage entries expire after a configurable number of ledgers if their TTL is not extended. The scaffold does not include TTL extension calls. In production:
- Every `persistent().get(...)` should be paired with `persistent().extend_ttl(...)`.
- Expired stream state would cause `unwrap()` panics; use `unwrap_or_default()` for reads in production.

**Mitigation:** Add TTL bump calls. Consider a `keep_alive(stream_id)` function on the factory that anyone can call to extend TTLs of active streams.

### 2. BySender / ByRecipient indices are unbounded vecs

Storing a `Vec<u64>` of stream IDs per address means a single address creating thousands of streams will hold a very large persistent storage entry. Large entries cost more in ledger fees.

**Mitigation (future):** Switch to a cursor-based linked list or off-chain indexing via Horizon events.

### 3. Governor authority is a single address

The current `DripGovernor` uses a single `authority` address for all configuration changes. If this key is compromised, an attacker could set `fee_bps = 10000` (100%) on future streams or drain the fee recipient.

**Mitigation:** Use a multisig or time-lock contract as the `authority`.

### 4. No minimum deposit validation against end_time

The factory validates `deposit >= rate_per_sec` (at least 1 second of streaming), but does not validate `deposit >= rate_per_sec × (end_time - start_time)`. A stream can be created where the deposit covers less time than the declared duration. The stream will simply drain early.

**Mitigation:** Add validation: if `end_time > 0`, require `deposit >= rate_per_sec × (end_time - start_time)`.

### 5. Clawback can be used adversarially

A sender with `clawback_enabled = true` can call `clawback()` to reclaim all unstreamed tokens at any time, effectively starving the recipient of future payments. Recipients should verify `clawback_enabled` before accepting a stream.

**Mitigation:** The app should display a prominent warning when `clawback_enabled = true`. Consider requiring recipient acknowledgement before stream activation.

---

## Disclosure

To report a security vulnerability privately:

- **Email:** security@conduit.sh
- **PGP:** key in `SECURITY.md` at the org root

Do not open a public GitHub issue for security vulnerabilities.

---

## Audit Checklist (pre-mainnet)

- [ ] All arithmetic overflow paths tested under adversarial inputs
- [ ] TTL management added to all persistent storage reads
- [ ] Governor authority switched to multisig
- [ ] Full integration test suite passing on testnet
- [ ] External audit completed by a Soroban-specialised firm
- [ ] Bug bounty program active before mainnet deployment
