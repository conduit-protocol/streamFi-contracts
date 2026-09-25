# GitHub Repository Settings

This document outlines the required GitHub repository settings for security and CI integrity.

## Issue #434: Main Branch Protection

**Status**: Requires Repository Admin Access

The `main` branch should have the following protection rules configured:

1. **Require status checks to pass before merging**:
   - Require the following checks to pass:
     - `Build WASM` (contracts/*/Cargo.toml WASM compilation)
     - `Lint` (clippy + rustfmt checks)
     - `Test` (unit and integration tests)
   - Require branches to be up to date before merging

2. **Restrict who can push to matching branches**:
   - Dismiss stale pull request approvals when new commits are pushed
   - Require code review approvals before merging

3. **Enforce restrictions on force pushes**:
   - Disallow force pushes to `main`
   - Disallow deletions of `main` branch

4. **Additional protection**:
   - Require branches to be up to date with main before merge
   - Include administrators in these restrictions

**Configuration Steps**:
1. Go to Settings → Branches
2. Under "Branch protection rules", click "Add rule"
3. Apply "main" pattern
4. Configure checks as outlined above

---

## Issue #435: Fork PR CI Approval

**Status**: Requires Repository Admin Access + Workflow Configuration

Fork PRs from external contributors currently don't trigger CI runs without approval, potentially masking failures.

### Workflow Trigger Configuration:

The GitHub Actions CI workflow uses the `pull_request` trigger, which has limited permissions for security. For fork PRs to run CI without masking results:

1. **Enable "Require approval for all outside collaborators"**:
   - Go to Settings → Actions → General
   - Under "Fork pull request workflows from outside collaborators", select:
     - "Require approval for all outside collaborators"

2. **Workflow improvement (optional)**:
   - Consider adding a `workflow_run` trigger that runs on completion of fork approval
   - This provides better visibility into CI status for fork PRs

### Current Workflow Status:
- The `.github/workflows/ci.yml` workflow currently triggers on:
  - `push` to `main` (always runs)
  - `pull_request` to `main` (requires fork PR approval to run)

### What This Achieves:
- Fork PRs will be clearly marked as requiring approval
- Once approved, full CI runs (Build WASM, Lint, Test)
- Prevents the false "green" state where mergeable PRs have no checks reported
- CI results become visible to reviewers

**Configuration Steps**:
1. Go to Settings → Actions → General
2. Under "Fork pull request workflows from outside collaborators"
3. Select "Require approval for all outside collaborators"
4. Save

---

## References
- [About branch protection rules](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [GitHub Actions - Fork PRs](https://docs.github.com/en/actions/using-workflows/triggering-a-workflow#pull-request-events-for-forked-repositories)
