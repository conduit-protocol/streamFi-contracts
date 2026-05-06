## What does this PR do?

<!-- One or two sentences. What problem does it solve or feature does it add? -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor (no behaviour change)
- [ ] Test coverage
- [ ] Documentation
- [ ] Security fix

## Related issue

Closes #

## Changes

<!-- List the key files changed and what was done in each. -->

| File | Change |
|------|--------|
| | |

## Checklist

- [ ] `cargo fmt --all` — no diff
- [ ] `cargo clippy --all-targets -- -D warnings` — zero warnings
- [ ] `cargo test --all` — all tests pass
- [ ] New public functions have tests covering the happy path and each error variant
- [ ] All arithmetic uses `checked_*` methods
- [ ] `require_auth()` called before any state mutation in modified functions
- [ ] State mutations happen before cross-contract calls (token transfers)
- [ ] Events emitted for all external state changes
- [ ] `CHANGELOG.md` updated under `[Unreleased]`
- [ ] `docs/architecture.md` updated if design changed
- [ ] ADR written if a significant design decision was made

## Security notes

<!-- Any auth, arithmetic, storage, or event changes worth calling out for reviewers? -->

## Testing notes

<!-- How to verify this manually if applicable (e.g. ledger sequence, test commands). -->
