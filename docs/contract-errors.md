# Contract Error Reference

Every Conduit contract defines its **own** `#[contracterror]` enum — the same
numeric code means something different in each contract (e.g. code `1` is
`NotAuthorized` in `DripStream`, `DripGovernor`, and `BatchTransferProcessor`,
but `NotInitialized` in `DripFactory`). Always match errors against the enum
for the specific contract you called, never by number alone.

This document is the single, authoritative list of every error variant across
all contracts, with a short description of when each fires. Numeric codes are
the raw `#[repr(u32)]` values. Variants marked _reserved_ are still part of the
live enum (so the numeric space is stable) but no current code path throws
them.

Contracts covered:

- [BatchTransferProcessor](#batchtransferprocessor)
- [DripFactory](#dripfactory)
- [DripGovernor](#dripgovernor)
- [TwapOracle](#twaporacle)
- [DripStream](#dripstream)
- [TokenVault](#tokenvault)

---

## BatchTransferProcessor

Source: [`contracts/batch-processor/src/lib.rs`](../contracts/batch-processor/src/lib.rs)

| Code | Name                 | Fires when                                                                        |
| ---- | -------------------- | --------------------------------------------------------------------------------- |
| `1`  | `LengthMismatch`     | `process_batch` receives `recipients` and `amounts` vectors of different lengths. |
| `2`  | `BatchTooLarge`      | The batch exceeds `MAX_BATCH_SIZE` (100 entries).                                 |
| `3`  | `InvalidAmount`      | An individual `amount` is zero or negative.                                       |
| `4`  | `ArithmeticOverflow` | Integer overflow while summing the total to pull from the funder.                 |

---

## DripFactory

Source: [`contracts/factory/src/errors.rs`](../contracts/factory/src/errors.rs)

| Code      | Name                      | Fires when                                                                                                                                           |
| --------- | ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `1`       | `NotInitialized`          | A state-mutating call (`create_stream`, batch creation, `pause`, `unpause`, `upgrade`, …) runs before `initialize` has been called.                  |
| `2`       | `InvalidDeposit`          | `deposit <= 0` in `create_stream`.                                                                                                                   |
| `3`       | `InvalidRate`             | `rate_per_sec <= 0` in `create_stream`.                                                                                                              |
| `4`       | `InvalidTimeRange`        | _Reserved_ — an earlier name for the `end_time <= start_time` check; the current `create_stream` path reports `InvalidDuration` (code `27`) instead. |
| `5`       | `InsufficientDeposit`     | `deposit < rate_per_sec` (cannot fund even one second), or the deposit does not cover `rate_per_sec × duration` for a fixed-duration stream.         |
| `6`       | `BackdatedStream`         | `start_time` is before the current ledger timestamp.                                                                                                 |
| `7`       | `AlreadyInitialized`      | `initialize` is called on a factory that is already initialized.                                                                                     |
| `8`       | `RateExceedsMax`          | `rate_per_sec` exceeds the governor's `max_rate_per_second`.                                                                                         |
| `9`       | `DurationTooShort`        | `end_time - start_time` is below the governor's `min_duration_seconds`.                                                                              |
| `10`      | `ArithmeticOverflow`      | Integer overflow validating `rate_per_sec × duration` / the protocol fee surcharge.                                                                  |
| `11`      | `ContractPaused`          | Stream creation is attempted while the factory is under an emergency pause.                                                                          |
| `12`      | `AlreadyPaused`           | `pause` is called while the factory is already paused.                                                                                               |
| `13`      | `NotPaused`               | `unpause` is called while the factory is not paused.                                                                                                 |
| `14`      | `DurationExceedsMax`      | `end_time - start_time` exceeds the governor's `max_duration_seconds`.                                                                               |
| `15`      | `GovernorNotResponding`   | A cross-contract call into the governor fails or is rejected (archived, not initialised, or host-level error).                                       |
| `16`      | `EmptyBatch`              | `create_batch_streams` is called with an empty `requests` vector.                                                                                    |
| `17`      | `BatchTooLarge`           | `create_batch_streams` requests exceed `MAX_BATCH_SIZE`.                                                                                             |
| `18`–`21` | _(unassigned)_            | Reserved numeric space between the original and later error additions.                                                                               |
| `22`      | `InvalidRecipient`        | The recipient is the all-zero Stellar account address, or identical to `sender`.                                                                     |
| `23`      | `CreateLocked`            | Another `create_stream` call is already in progress (re-entrancy guard held).                                                                        |
| `24`      | `DepositTransferFailed`   | The deposit transfer from `sender` to the factory did not arrive.                                                                                    |
| `25`      | `StreamFundingFailed`     | The deposit forward from the factory to the deployed stream did not arrive.                                                                          |
| `26`      | `InvalidWasmHash`         | `upgrade_stream_wasm`/`upgrade` is given an all-zero WASM hash.                                                                                      |
| `27`      | `InvalidDuration`         | `end_time > 0` and `end_time <= start_time` — stream duration is zero or negative.                                                                   |
| `28`      | `InvalidToken`            | The token address is the all-zero Stellar account address.                                                                                           |
| `29`      | `StartTimeTooFarInFuture` | `start_time` sits further ahead than the protocol's `max_duration_seconds` scheduling window (would lock the deposit beyond any legitimate stream).  |
| `30`      | `StorageVersionMismatch`  | `upgrade` is given a WASM whose storage layout version does not match `DataKey::FactoryStorageVersion`.                                              |
| `31`      | `InvalidGovernor`         | `initialize` is given the all-zero governor address, which would point every `create_stream` at a non-existent contract.                             |

---

## DripGovernor

Source: [`contracts/governor/src/errors.rs`](../contracts/governor/src/errors.rs)

| Code | Name                  | Fires when                                                                                                                                                        |
| ---- | --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `1`  | `NotAuthorized`       | The caller does not hold the role (or ownership) gating the operation.                                                                                            |
| `2`  | `InvalidParam`        | A setter argument fails validation (e.g. `fee_bps > 10_000`, a zero `min`/`max` duration or rate, or an arithmetic overflow while computing a rate × time bound). |
| `3`  | `AlreadyInitialized`  | `initialize` is called on a governor that is already initialized.                                                                                                 |
| `4`  | `LastAdmin`           | `revoke_role` would remove the final `Admin`, freezing governance.                                                                                                |
| `5`  | `NotInitialized`      | A call requires governor configuration before `initialize` has run (required storage keys missing).                                                               |
| `6`  | `ContractPaused`      | A parameter-changing call is attempted while the governor is under an emergency pause.                                                                            |
| `7`  | `AlreadyPaused`       | `pause` is called while the governor is already paused.                                                                                                           |
| `8`  | `NotPaused`           | `unpause` is called while the governor is not paused.                                                                                                             |
| `9`  | `FactoryCallFailed`   | A cross-contract `pause_factory`/`unpause_factory` call into `DripFactory` fails or is rejected by the factory.                                                   |
| `10` | `NoPendingAuthority`  | `accept_authority` (or a related transfer op) is called when there is no pending authority transfer to accept.                                                    |
| `11` | `NotPendingAuthority` | `accept_authority` is called by an address that is not the pending authority.                                                                                     |
| `12` | `InvalidWasmHash`     | `upgrade` is given an all-zero WASM hash.                                                                                                                         |

---

## TwapOracle

Source: [`contracts/oracle/src/lib.rs`](../contracts/oracle/src/lib.rs)

| Code   | Name                   | Fires when                                                                                                                                                         |
| ------ | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `1001` | `OracleStalePrice`     | `get_twap_price` finds zero fresh submissions — every recorded observation is older than `max_staleness`.                                                          |
| `1002` | `OracleNotConfigured`  | A read (`get_twap_price`, `price_status`, `price_age`, `min_submitters`, …) runs before `configure_oracle`, or `configure_oracle` itself runs before `initialize`. |
| `1003` | `InvalidPrice`         | `submit_price` is given a zero price.                                                                                                                              |
| `1004` | `OracleLocked`         | A guarded entry point is invoked while the re-entrancy guard is already held.                                                                                      |
| `1005` | `CalculationOverflow`  | _Reserved_ — no current code path throws it (arithmetic errors surface as `ArithmeticOverflow`).                                                                   |
| `1006` | `NotAuthorized`        | The caller lacks the required role (e.g. `Admin` for `configure_oracle`/`grant_role`, `PriceFeeder` for `submit_price`) or its auth verification fails.            |
| `1007` | `AlreadyInitialized`   | `initialize` is called on an oracle that is already initialized.                                                                                                   |
| `1008` | `NoPriceAvailable`     | `get_twap_price`/`price_status`/`price_age` is called before any feeder has ever submitted a price.                                                                |
| `1009` | `ArithmeticOverflow`   | Integer overflow while computing the fiat payout from a price.                                                                                                     |
| `1010` | `InvalidDecimals`      | `configure_oracle` is given `config.decimals > 19` (u64 price limit).                                                                                              |
| `1011` | `ContractPaused`       | `submit_price` (or another guarded op) is attempted while the oracle is under an emergency pause.                                                                  |
| `1012` | `AlreadyPaused`        | `pause` is called while the oracle is already paused.                                                                                                              |
| `1013` | `NotPaused`            | `unpause` is called while the oracle is not paused.                                                                                                                |
| `1014` | `LastAdmin`            | A role-revocation would remove the last `Admin`, freezing oracle governance.                                                                                       |
| `1015` | `InvalidMaxStaleness`  | `configure_oracle` is given `max_staleness == 0` (degenerate — every price would be immediately stale).                                                            |
| `1016` | `TooManySubmitters`    | Adding a feeder would exceed the `MAX_SUBMITTERS` cap (32).                                                                                                        |
| `1017` | `PriceExceedsMaxPrice` | A submitted price exceeds the configured `max_price` ceiling.                                                                                                      |
| `1018` | `SubmitTooSoon`        | A feeder re-submits before `min_submit_interval` seconds have elapsed since its last accepted submission.                                                          |
| `1019` | `InvalidMinSubmitters` | `configure_oracle` is given `min_submitters == 0` or `> MAX_SUBMITTERS` — the quorum must be at least one feeder and can never exceed the capped submitter set.    |
| `1020` | `InsufficientQuorum`   | `get_twap_price` finds fewer fresh, non-zero feeder submissions than the configured `min_submitters` quorum, so the TWAP is not reliable enough to report.         |

> The oracle crate also contains a legacy, uncompiled
> [`contracts/oracle/src/errors.rs`](../contracts/oracle/src/errors.rs)
> (`NotInitialized` … `NotAuthorized`, codes `1`–`11`). It is not declared as a
> module and is **not** the enum the deployed `TwapOracle` uses — the table
> above is authoritative for on-chain calls.

---

## DripStream

Source: [`contracts/stream/src/errors.rs`](../contracts/stream/src/errors.rs)

| Code | Name                   | Fires when                                                                                                                                                                                          |
| ---- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `1`  | `NotAuthorized`        | A state-mutating operation is attempted by an address that is neither the sender (nor their operator) nor the recipient, as required by that operation.                                             |
| `2`  | `StreamNotFound`       | _Reserved_ — formerly "invalid stream ID"; the current `state::load` reports `NotInitialized` for an inaccessible stream instance instead.                                                          |
| `3`  | `StreamCancelled`      | An operation (`withdraw`, `pause`, `resume`, `cancel`, `clawback`, `force_cancel`, `top_up`, `extend_duration`, …) targets a stream that has been cancelled.                                        |
| `4`  | `StreamNotStarted`     | `withdraw`/`pause` is called before the stream's `start_time`.                                                                                                                                      |
| `5`  | `StreamEnded`          | `withdraw`/`resume`/`top_up` is called after the stream's `end_time` (stream is over).                                                                                                              |
| `6`  | `NothingToWithdraw`    | `withdraw` is called with zero accrued (nothing withdrawable yet).                                                                                                                                  |
| `7`  | `InsufficientDeposit`  | _Reserved_ — deposit sufficiency is enforced by `DripFactory` at creation; no current `DripStream` code path throws it.                                                                             |
| `8`  | `InvalidTimeRange`     | `initialize` is given `end_time > 0` with `end_time <= start_time`; or `extend_duration`/`top_up_and_extend` is called with `extra_time_seconds == 0` or on an open-ended (`end_time == 0`) stream. |
| `9`  | `AlreadyPaused`        | `pause` is called on a stream that is already paused.                                                                                                                                               |
| `10` | `NotPaused`            | `resume` is called on a non-paused stream, `clawback` is called while the stream **is** paused, or `force_cancel` is called on a non-paused stream.                                                 |
| `11` | `ClawbackDisabled`     | `clawback` is called on a stream initialized with clawback disabled.                                                                                                                                |
| `12` | `ArithmeticOverflow`   | Integer overflow in release/settlement math.                                                                                                                                                        |
| `13` | `PauseThresholdNotMet` | `force_cancel` is called before the pause-to-cancel threshold (default 30 days) has elapsed since the pause began.                                                                                  |
| `14` | `AlreadyInitialized`   | `initialize` is called on a stream that is already initialized (re-initialization guard against draining an escrowed balance).                                                                      |
| `15` | `InvalidAmount`        | `withdraw`/`top_up` is called with `amount <= 0`, or `initialize` is given `rate_per_second <= 0` or `force_cancel_pause_secs == 0`.                                                                |
| `16` | `ReentrancyForbidden`  | The re-entrancy guard's depth counter exceeds `MAX_REENTRANCY_DEPTH` (a bug or malicious callback).                                                                                                 |
| `17` | `OperatorAlreadySet`   | _Reserved_ — `set_operator` now atomically replaces an existing operator rather than rejecting; no current code path throws it.                                                                     |
| `18` | `NotInitialized`       | Any operation on a stream instance whose state has not been initialized (missing storage keys).                                                                                                     |
| `19` | `InvalidRecipient`     | `initialize` is given the all-zero Stellar account address as recipient, or `recipient == sender`.                                                                                                  |
| `20` | `BackdatedStream`      | `initialize` is given a `start_time` in the past.                                                                                                                                                   |
| `21` | `StreamUnderfunded`    | A withdrawal is requested while the stream has accrued tokens but is not funded enough to cover them.                                                                                               |

---

## TokenVault

Source: [`contracts/token-vault/src/errors.rs`](../contracts/token-vault/src/errors.rs)

| Code | Name                     | Fires when                                                                                                                                                                                                                                             |
| ---- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `1`  | `InvalidAmount`          | `deposit`/`withdraw` is given `amount <= 0`, `withdraw_batch` contains a nonpositive amount or no payouts, `initialize` is given `max_limit <= 0`, `set_operator_withdraw_limit` is given `new_limit <= 0`, or `set_owner_proposal_ttl` is given zero. |
| `2`  | `ArithmeticOverflow`     | Integer overflow computing the expected vault balance or aggregate batch amount, or the owner proposal expiry timestamp.                                                                                                                               |
| `3`  | `LimitExceeded`          | A deposit would push the vault balance past `max_limit`; a withdrawal or aggregate batch withdrawal exceeds the caller's cap or available balance; or `set_limit` tries to set a final balance below the current balance.                              |
| `4`  | `NotAuthorized`          | The caller is neither the vault owner nor the delegated operator for an operation that allows both (or lacks the owner role for an owner-only operation).                                                                                              |
| `5`  | `ContractPaused`         | `deposit`, `withdraw`, `withdraw_batch`, or `set_limit` is called while the vault is under an emergency pause.                                                                                                                                         |
| `6`  | `AlreadyPaused`          | `pause` is called while the vault is already paused.                                                                                                                                                                                                   |
| `7`  | `NotPaused`              | `unpause` is called while the vault is not paused.                                                                                                                                                                                                     |
| `8`  | `NotInitialized`         | A call runs before `initialize` has been called (no owner/token stored).                                                                                                                                                                               |
| `9`  | `AlreadyInitialized`     | `initialize` is called on an already-initialized vault (an owner exists).                                                                                                                                                                              |
| `10` | `InvalidParam`           | `propose_owner` is given the all-zero Stellar account address as the pending owner.                                                                                                                                                                    |
| `11` | `NoPendingOwner`         | `accept_owner` (or `pending_owner` bookkeeping) is called when there is no pending owner transfer to accept.                                                                                                                                           |
| `12` | `NotPendingOwner`        | `accept_owner` is called by an address that is not the proposed pending owner.                                                                                                                                                                         |
| `13` | `DepositTransferFailed`  | The token transfer into the vault did not move exactly the expected amount for `deposit`.                                                                                                                                                              |
| `14` | `WithdrawTransferFailed` | A token transfer out of the vault did not move exactly the expected amount for `withdraw` or `withdraw_batch`.                                                                                                                                         |
| `15` | `InvalidWasmHash`        | `upgrade` is given an all-zero WASM hash.                                                                                                                                                                                                              |
| `16` | `PendingOwnerExpired`    | `accept_owner` is called after the pending owner's configured validity window, or the pending proposal has no timestamp.                                                                                                                               |
