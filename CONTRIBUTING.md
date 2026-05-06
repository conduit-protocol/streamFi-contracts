# Contributing to conduit-contracts

Thank you for your interest in contributing to Conduit's smart contract layer. This document covers everything you need to get a working environment, understand the codebase, write good tests, and get your PR merged.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Getting Started](#getting-started)
3. [Repository Layout](#repository-layout)
4. [Development Workflow](#development-workflow)
5. [Writing Contracts](#writing-contracts)
6. [Writing Tests](#writing-tests)
7. [Commit Convention](#commit-convention)
8. [Pull Request Process](#pull-request-process)
9. [Security Vulnerabilities](#security-vulnerabilities)
10. [Architecture Decision Records](#architecture-decision-records)

---

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](./CODE_OF_CONDUCT.md). By participating you agree to uphold it. Report unacceptable behaviour to **conduct@conduit.sh**.

---

## Getting Started

### Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust | ≥ 1.78 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `wasm32-unknown-unknown` target | — | `rustup target add wasm32-unknown-unknown` |
| Stellar CLI | ≥ 20.0 | `cargo install --locked stellar-cli` |
| Docker | ≥ 24 | Required for local network |

### Clone and build

```bash
git clone https://github.com/conduit-protocol/conduit-contracts
cd conduit-contracts

# Install the WASM target
rustup target add wasm32-unknown-unknown

# Compile all contracts
cargo build --target wasm32-unknown-unknown --release

# Run all tests (unit + integration)
cargo test --all

# Lint
cargo clippy --all-targets --all-features -- -D warnings

# Format check
cargo fmt --all -- --check
```

### Local network

```bash
# Start a local Stellar node (Docker required)
stellar network start local

# Fund a local key
stellar keys generate dev --network local --fund

# Deploy all contracts
./scripts/deploy.sh local

# Contract IDs are written to .contract-ids/local.json
cat .contract-ids/local.json
```

---

## Repository Layout

```
conduit-contracts/
├── Cargo.toml                  # workspace root
├── contracts/
│   ├── stream/                 # DripStream — per-stream payment contract
│   │   └── src/
│   │       ├── lib.rs          # public contract interface
│   │       ├── math.rs         # withdrawable / streamed_amount calculations
│   │       ├── storage.rs      # DataKey enum + StreamInfo struct
│   │       ├── errors.rs       # Error contracterror enum
│   │       ├── events.rs       # event emission helpers
│   │       └── tests.rs        # unit tests (run with cargo test -p drip-stream)
│   ├── factory/                # DripFactory — deployment + global registry
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── deploy.rs       # deterministic WASM deployment logic
│   │       └── storage.rs
│   └── governor/               # DripGovernor — protocol parameters
│       └── src/
│           ├── lib.rs
│           └── storage.rs
├── tests/                      # cross-contract integration tests
│   ├── stream_lifecycle.rs
│   ├── stream_pause_resume.rs
│   ├── stream_clawback.rs
│   ├── factory_deploy.rs
│   └── governor_config.rs
├── scripts/
│   ├── deploy.sh               # deploy to local / testnet / mainnet
│   ├── upgrade.sh              # upgrade WASM hash via governor
│   └── query.sh                # query stream state from CLI
└── docs/
    ├── architecture.md         # contract interaction diagram + design rationale
    ├── security.md             # threat model and known limitations
    └── adr/                    # Architecture Decision Records
```

---

## Development Workflow

We use a **feature branch → PR → squash merge** workflow.

```
main          ←── always deployable, protected
  └── feat/your-feature     ← your work lives here
```

1. **Fork** the repo and clone your fork.
2. **Create a branch** from `main`:
   ```bash
   git checkout -b feat/my-feature
   ```
3. Make your changes, keeping commits small and logical.
4. **Run the full check suite** before pushing:
   ```bash
   cargo fmt --all
   cargo clippy --all-targets -- -D warnings
   cargo test --all
   ```
5. **Push** and open a PR against `conduit-protocol/conduit-contracts:main`.

---

## Writing Contracts

### Style rules

- `#![no_std]` is required in every contract crate — Soroban runs in a constrained WASM environment.
- Use `soroban_sdk` types everywhere. Never use `std::collections`, `Box`, `String`, etc.
- Keep contract entry points (`#[contractimpl]` functions) thin. Move business logic into private helpers or sub-modules (`math.rs`, etc.).
- All arithmetic on token amounts must use `checked_*` methods and return `Error::ArithmeticOverflow` on failure. Never use `+`, `-`, `*` directly on `i128` values.
- Emit events for every state-changing operation. Use helpers in `events.rs`.
- State mutations must happen **before** cross-contract calls (token transfers) in every function. This preserves the correct state-machine ordering even though Soroban prevents reentrancy.

### Auth pattern

```rust
// Always use require_auth(), never manual signature verification
info.sender.require_auth();
```

### Error handling

All errors must be variants of the `Error` `#[contracterror]` enum in `errors.rs`. Never `panic!` in production paths. `unwrap()` is acceptable only for storage reads that are guaranteed to be initialised by `initialize()`.

### Storage tiers

| Use case | Tier |
|----------|------|
| Contract config (set once, read often) | `instance()` |
| Stream state (mutable per-stream data) | `instance()` (on DripStream) |
| Registry indices (BySender, ByRecipient, StreamAddr) | `persistent()` |
| Nothing | `temporary()` |

Remember: `persistent()` entries expire. Production code must call `extend_ttl()` on every read of a persistent entry.

---

## Writing Tests

### Unit tests (`contracts/stream/src/tests.rs`)

Use `soroban_sdk::testutils` to mock auth, ledger timestamps, and token contracts:

```rust
#[test]
fn my_test() {
    let env = Env::default();
    env.mock_all_auths();

    // Advance time by bumping ledger timestamp
    env.ledger().set(LedgerInfo {
        timestamp: 1_000_000 + 600,  // 600 seconds later
        ..env.ledger().get()
    });
}
```

- Every new public function needs at least one positive test and one negative test (testing each error variant it can return).
- Tests that check token balances must create a mock Stellar asset contract via `env.register_stellar_asset_contract()`.

### Integration tests (`tests/`)

Integration tests deploy all three contracts together and test cross-contract interactions. They follow the same `Env::default()` + `mock_all_auths()` pattern but register multiple contracts.

When adding a new integration test file, add it to `Cargo.toml`'s `[[test]]` section:

```toml
[[test]]
name = "my_integration_test"
path = "tests/my_integration_test.rs"
```

### Running specific tests

```bash
# Unit tests for stream contract only
cargo test -p drip-stream

# Single integration test file
cargo test --test stream_lifecycle

# A specific test function
cargo test --test stream_lifecycle -- cancel_halfway_splits_correctly
```

---

## Commit Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short description>

[optional body]

[optional footer]
```

**Types:**

| Type | When to use |
|------|-------------|
| `feat` | New contract function or feature |
| `fix` | Bug fix in existing logic |
| `test` | Adding or fixing tests |
| `refactor` | Code restructuring with no behaviour change |
| `docs` | Documentation only |
| `chore` | Tooling, CI, dependency updates |
| `security` | Security fix (coordinate with maintainers first) |

**Scopes:** `stream`, `factory`, `governor`, `math`, `deploy`, `tests`, `ci`, `docs`

**Examples:**

```
feat(stream): add transfer_recipient() — reassign stream rights

fix(math): use saturating_sub to guard against withdrawn > streamed edge case

test(factory): add integration tests for BySender index pagination

docs(adr): ADR 003 — recipient transfer without sender veto

security(governor): validate fee_bps does not exceed 10000
```

---

## Pull Request Process

### Before opening a PR

- [ ] `cargo fmt --all` — no diff
- [ ] `cargo clippy --all-targets -- -D warnings` — zero warnings
- [ ] `cargo test --all` — all tests pass
- [ ] If adding a new feature, update `docs/architecture.md` if the design changes
- [ ] If introducing a new security-relevant decision, write an ADR in `docs/adr/`
- [ ] PR description filled in using the template (`.github/PULL_REQUEST_TEMPLATE.md`)

### Review requirements

- At least **1 approval** from a maintainer before merging.
- Any PR touching `math.rs`, `errors.rs`, or authentication logic requires **2 approvals**.
- CI must be green (fmt, clippy, tests).

### What reviewers look for

- Checked arithmetic on all token amounts.
- `require_auth()` called before any state mutation.
- State mutated before cross-contract calls.
- Events emitted for every external state change.
- No `unwrap()` on data that could be absent in a reachable code path.
- Tests cover the new behaviour and each new error variant.

---

## Security Vulnerabilities

**Do not open a public GitHub issue for security vulnerabilities.**

Report privately via email to **security@conduit.sh** with:
- A description of the vulnerability
- Steps to reproduce or a proof-of-concept
- Potential impact
- Suggested fix (if you have one)

We aim to acknowledge within 48 hours and provide a status update within 1 week. See [`docs/security.md`](./docs/security.md) for the full threat model.

---

## Architecture Decision Records

When you make a significant design decision (choice of storage tier, new invariant, trade-off in the cancel/settle flow), document it as an ADR:

1. Copy `docs/adr/001-per-contract-per-stream.md` as a template.
2. Number sequentially (`004-your-decision.md`).
3. Fill in **Context**, **Decision**, **Rationale**, and **Consequences**.
4. Reference the ADR in your PR description.

ADRs are immutable once merged. If a decision is reversed, write a new ADR that supersedes the old one — do not edit the original.

---

## License

By contributing you agree that your contributions will be licensed under the [MIT License](./LICENSE).
