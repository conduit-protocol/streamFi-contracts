# Changelog

All notable changes are documented here. Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- `force_cancel()` on `DripStream` — recipient can settle atomically after sender leaves stream paused for more than 30 days (`PauseThresholdNotMet` error returned if threshold not met)
- `PauseThresholdNotMet` error code (13) added to `Error` enum

### Changed
- `get_escrow_for_user` visibility scoped; bare `get_escrow` now `pub(crate)` only

### Security
- Documented pause-state griefing attack vector in `docs/security.md` (ADR-003 records the fix design)

---

## [0.2.0] - 2026-04-05

### Added
- `upgrade_stream_wasm()` on `DripFactory` — governor-gated WASM hash update for deployed stream contracts
- `streamed_total()` view on `DripStream` — returns gross streamed amount (before withdrawal subtraction) for UI display
- `transfer_recipient()` on `DripStream` — recipient can reassign stream rights to a new address; emits `xfer_rec` event
- ADR-003: Recipient transfer without sender veto

### Fixed
- `DripFactory`: extend persistent storage TTL on `StreamAddr` entries to prevent ledger pruning on long-lived streams
- `math.rs`: use `saturating_sub` in `withdrawable()` to guard against `withdrawn > streamed` edge case on rapid successive withdrawals

### Tests
- 9 additional unit tests in `stream/src/tests.rs`: cancelled state, pause-then-cancel, `info()` struct fields, sequential withdrawals, edge cases
- Full integration test suite: `stream_lifecycle`, `stream_pause_resume`, `stream_clawback`, `factory_deploy`, `governor_config`

### CI
- Split unit and integration test steps in `ci.yml`
- Added WASM `clippy` pass to catch `no_std` incompatibilities
- Report WASM artifact sizes on each build

---

## [0.1.0] - 2026-02-14

### Added
- `DripStream` contract: `initialize`, `withdraw`, `cancel`, `pause`, `resume`, `top_up`, `clawback`, `withdrawable`, `info`
- `DripFactory` contract: `initialize`, `create_stream`, `stream_address`, `streams_by_sender`, `streams_by_recipient`, `stream_count`, `protocol_fee_bps`
- `DripGovernor` contract: `initialize`, `config`, `set_fee_bps`, `set_fee_recipient`, `set_min_duration`, `set_max_rate`, `transfer_authority`
- Deterministic stream deployment via SHA-256 salted WASM hash
- Per-sender and per-recipient stream index in persistent storage
- Event emission for all state changes: `withdrawn`, `cancelled`, `paused`, `resumed`, `topped_up`, `clawback`
- Architecture docs, security threat model, ADR-001 (per-contract-per-stream), ADR-002 (atomic cancel settlement)
- Deploy, upgrade, and query scripts for local / testnet / mainnet
