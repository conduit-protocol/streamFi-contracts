#!/usr/bin/env bash
# deploy.sh — Deploy all Conduit contracts to a Stellar network
#
# Usage:
#   ./scripts/deploy.sh local
#   ./scripts/deploy.sh testnet
#   ./scripts/deploy.sh mainnet
#
# Prerequisites:
#   - stellar CLI installed and on PATH
#   - jq installed and on PATH (also used by query.sh / upgrade.sh)
#   - Rust + wasm32-unknown-unknown target
#   - For testnet/mainnet: funded identity set up with `stellar keys generate`
#
# Env vars (optional):
#   TOKEN_ADDRESS      — SAC contract ID TokenVault should manage.
#                         Defaults to the native XLM SAC for $NETWORK.
#   TOKEN_VAULT_MAX_LIMIT — TokenVault's per-account deposit ceiling (stroops).
#                         Defaults to 1_000_000_000_000 (100k XLM).
#
# Deploys all 6 contracts that produce a WASM (drip-common is a shared rlib,
# not a deployable contract; DripStream is uploaded — not deployed as a
# standalone instance — because DripFactory deploys new stream instances
# per-user from its stored WASM hash) (#295).
#
# ── Failure semantics ─────────────────────────────────────────────────────────
# `set -euo pipefail` halts the script at the first failed step and the exit
# code propagates to the caller.
#
# ── Resume after a partial failure (#663) ────────────────────────────────────
# Progress is persisted incrementally to .contract-ids/$NETWORK.json after
# EVERY step (each WASM upload, each deploy, each initialize) instead of only
# at the very end, so an interrupted run leaves a usable on-disk record of what
# already committed on-chain. To resume, simply re-run the script with the same
# network argument: steps whose result is already recorded are skipped — WASM
# hashes and deployed contract IDs are reused, and contracts whose initialize
# step already completed are not re-initialized. If an initialize reports
# `AlreadyInitialized` during a resumed run, the script treats it as done and
# continues. To force a from-scratch redeploy, delete (or back up)
# .contract-ids/$NETWORK.json before re-running.
#
# ── Rollback of a partial deployment (#663) ──────────────────────────────────
# deploy.sh only ever CREATES new contract instances — Stellar has no
# "un-deploy", and nothing references the new instances until the factory and
# governor are explicitly upgraded or repointed by other processes. Rolling
# back a partial deploy therefore means:
#   1. Restore the previous known-good .contract-ids/$NETWORK.json (from git /
#      backup) so query.sh and upgrade.sh keep targeting the old instances.
#   2. Leave any half-deployed instances untouched on-chain — they are inert:
#      a freshly deployed governor holds no authority from any live factory,
#      a freshly deployed factory is not referenced by the old governor, and
#      the oracle / batch-processor / token-vault instances are only reachable
#      by addresses that are told about them.
#   3. Before restoring, note any partial IDs recorded in the interrupted run's
#      state file — it is the only on-book record of what that run deployed.
#   4. Verify no factory or governor was UPGRADED during the failed run, not
#      just deployed: upgrade.sh repoints an existing instance in place, so a
#      rolled-back file must not be mixed with a live upgrade of the same
#      contract. Deploy-vs-upgrade is the distinguishing check.

set -euo pipefail

NETWORK="${1:-testnet}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT_DIR="$ROOT_DIR/.contract-ids"
IDS_FILE="$OUT_DIR/$NETWORK.json"

mkdir -p "$OUT_DIR"

# ── Deploy state (persisted per network, #663) ──────────────────────────────
# Every successful step writes .contract-ids/$NETWORK.json, so a partial run
# can be resumed (steps already recorded are skipped) and audited (see the
# RESUME / ROLLBACK sections at the top of this file).
state_has() { # state_has <key>: true when key is present and non-null
  [[ -f "$IDS_FILE" ]] && jq -e --arg k "$1" 'has($k)' "$IDS_FILE" >/dev/null 2>&1
}

state_get() { # state_get <key>: prints the value (empty string when absent)
  if [[ -f "$IDS_FILE" ]]; then
    jq -r --arg k "$1" '.[$k] // empty' "$IDS_FILE"
  fi
}

state_set() { # state_set <key> <value>: set a key, writing atomically
  if [[ -f "$IDS_FILE" ]]; then
    jq --arg k "$1" --arg v "$2" '.[$k] = $v' "$IDS_FILE" > "$IDS_FILE.tmp"
    mv "$IDS_FILE.tmp" "$IDS_FILE"
  else
    jq -n --arg k "$1" --arg v "$2" '{ ($k): $v }' > "$IDS_FILE"
  fi
}

# Run a contract's `initialize` with resume semantics: record success in the
# state file so a later resume skips it; treat an `AlreadyInitialized` error as
# "already done on a previous run" (the on-chain init committed but the flag
# was never persisted); any other failure aborts so the operator can resume or
# roll back.
initialize_contract() {
  local label="$1" flag="$2"
  shift 2
  set +e
  local out rc
  out=$(stellar contract invoke --network "$NETWORK" $IDENTITY "$@" 2>&1)
  rc=$?
  set -e
  if (( rc == 0 )); then
    state_set "$flag" true
    echo "    $label initialized."
  elif grep -q "AlreadyInitialized" <<<"$out"; then
    state_set "$flag" true
    echo "    $label reports AlreadyInitialized — marking done and resuming (issue #663)."
  else
    echo "❌  $label initialize failed (exit $rc):" >&2
    echo "$out" >&2
    exit 1
  fi
}

# ── Identity ──────────────────────────────────────────────────────────────────
if [[ "$NETWORK" == "local" ]]; then
  IDENTITY="--source alice"
  # Ensure a local identity exists
  stellar keys generate alice --network local 2>/dev/null || true
else
  IDENTITY="--source dev"
fi

echo "🔨  Building contracts (release)…"
cd "$ROOT_DIR"
cargo build --target wasm32-unknown-unknown --release --quiet

WASM_DIR="$ROOT_DIR/target/wasm32-unknown-unknown/release"

# ── Resume detection (#663) ───────────────────────────────────────────────────
if [[ -f "$IDS_FILE" ]]; then
  EXISTING_NET=$(state_get network)
  if [[ -n "$EXISTING_NET" && "$EXISTING_NET" != "$NETWORK" ]]; then
    echo "❌  $IDS_FILE belongs to network '$EXISTING_NET' but '$NETWORK' was requested." >&2
    echo "    Refusing to reuse IDs across networks. Use a separate file per network." >&2
    exit 1
  fi
  # Old state files (pre-resume-support) have no `network` key; adopt the
  # requested network rather than refusing to resume them.
  if [[ -z "$EXISTING_NET" ]]; then
    state_set network "$NETWORK"
  fi
  echo "♻️   Existing deployment state found: $IDS_FILE"
  echo "    Resuming — recorded steps will be reused, not redone."
  echo "    Force a from-scratch redeploy by deleting this file first (see header)."
else
  echo "🆕  No deployment state for '$NETWORK' yet — starting fresh."
  state_set network "$NETWORK"
fi

# ── Upload WASMs ──────────────────────────────────────────────────────────────
echo "📤  Uploading DripStream WASM…"
STREAM_WASM_HASH=$(state_get streamWasmHash)
if [[ -z "$STREAM_WASM_HASH" ]]; then
  STREAM_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_stream.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set streamWasmHash "$STREAM_WASM_HASH"
fi
echo "    DripStream WASM hash: $STREAM_WASM_HASH"

echo "📤  Uploading DripFactory WASM…"
FACTORY_WASM_HASH=$(state_get factoryWasmHash)
if [[ -z "$FACTORY_WASM_HASH" ]]; then
  FACTORY_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_factory.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set factoryWasmHash "$FACTORY_WASM_HASH"
fi

echo "📤  Uploading DripGovernor WASM…"
GOVERNOR_WASM_HASH=$(state_get governorWasmHash)
if [[ -z "$GOVERNOR_WASM_HASH" ]]; then
  GOVERNOR_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_governor.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set governorWasmHash "$GOVERNOR_WASM_HASH"
fi

echo "📤  Uploading TwapOracle WASM…"
ORACLE_WASM_HASH=$(state_get oracleWasmHash)
if [[ -z "$ORACLE_WASM_HASH" ]]; then
  ORACLE_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_oracle.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set oracleWasmHash "$ORACLE_WASM_HASH"
fi

echo "📤  Uploading BatchTransferProcessor WASM…"
BATCH_PROCESSOR_WASM_HASH=$(state_get batchProcessorWasmHash)
if [[ -z "$BATCH_PROCESSOR_WASM_HASH" ]]; then
  BATCH_PROCESSOR_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_batch_processor.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set batchProcessorWasmHash "$BATCH_PROCESSOR_WASM_HASH"
fi

echo "📤  Uploading TokenVault WASM…"
TOKEN_VAULT_WASM_HASH=$(state_get tokenVaultWasmHash)
if [[ -z "$TOKEN_VAULT_WASM_HASH" ]]; then
  TOKEN_VAULT_WASM_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/token_vault.wasm" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set tokenVaultWasmHash "$TOKEN_VAULT_WASM_HASH"
fi

# ── Deploy contracts ──────────────────────────────────────────────────────────
AUTHORITY=$(stellar keys address dev 2>/dev/null || stellar keys address alice)

echo "🚀  Deploying DripGovernor…"
GOVERNOR_ID=$(state_get governor)
if [[ -z "$GOVERNOR_ID" ]]; then
  GOVERNOR_ID=$(stellar contract deploy \
    --wasm-hash "$GOVERNOR_WASM_HASH" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set governor "$GOVERNOR_ID"
fi
echo "    DripGovernor: $GOVERNOR_ID"

echo "🚀  Deploying DripFactory…"
FACTORY_ID=$(state_get factory)
if [[ -z "$FACTORY_ID" ]]; then
  FACTORY_ID=$(stellar contract deploy \
    --wasm-hash "$FACTORY_WASM_HASH" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set factory "$FACTORY_ID"
fi
echo "    DripFactory: $FACTORY_ID"

echo "🚀  Deploying TwapOracle…"
ORACLE_ID=$(state_get oracle)
if [[ -z "$ORACLE_ID" ]]; then
  ORACLE_ID=$(stellar contract deploy \
    --wasm-hash "$ORACLE_WASM_HASH" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set oracle "$ORACLE_ID"
fi
echo "    TwapOracle: $ORACLE_ID"

echo "🚀  Deploying BatchTransferProcessor…"
BATCH_PROCESSOR_ID=$(state_get batchProcessor)
if [[ -z "$BATCH_PROCESSOR_ID" ]]; then
  BATCH_PROCESSOR_ID=$(stellar contract deploy \
    --wasm-hash "$BATCH_PROCESSOR_WASM_HASH" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set batchProcessor "$BATCH_PROCESSOR_ID"
fi
echo "    BatchTransferProcessor: $BATCH_PROCESSOR_ID"

echo "🚀  Deploying TokenVault…"
TOKEN_VAULT_ID=$(state_get tokenVault)
if [[ -z "$TOKEN_VAULT_ID" ]]; then
  TOKEN_VAULT_ID=$(stellar contract deploy \
    --wasm-hash "$TOKEN_VAULT_WASM_HASH" \
    --network "$NETWORK" $IDENTITY \
    --quiet)
  state_set tokenVault "$TOKEN_VAULT_ID"
fi
echo "    TokenVault: $TOKEN_VAULT_ID"

# ── Initialise contracts ──────────────────────────────────────────────────────
echo "⚙️   Initialising DripGovernor…"
if state_has governorInitialized; then
  echo "    already done on a previous run — skipped."
else
  initialize_contract "DripGovernor" governorInitialized \
    --id "$GOVERNOR_ID" \
    -- initialize \
    --authority "$AUTHORITY" \
    --fee_recipient "$AUTHORITY" \
    --factory_address "$FACTORY_ID"
fi

echo "⚙️   Initialising DripFactory…"
if state_has factoryInitialized; then
  echo "    already done on a previous run — skipped."
else
  initialize_contract "DripFactory" factoryInitialized \
    --id "$FACTORY_ID" \
    -- initialize \
    --stream_wasm_hash "$STREAM_WASM_HASH" \
    --governor "$GOVERNOR_ID"
fi

echo "⚙️   Initialising TwapOracle…"
if state_has oracleInitialized; then
  echo "    already done on a previous run — skipped."
else
  initialize_contract "TwapOracle" oracleInitialized \
    --id "$ORACLE_ID" \
    -- initialize \
    --admin "$AUTHORITY"
fi

echo "⚙️   Initialising BatchTransferProcessor…"
# `process_batch` is permissionless and works uninitialised; `initialize` only
# records the admin allowed to call `upgrade` on the processor (issue #651).
if state_has batchProcessorInitialized; then
  echo "    already done on a previous run — skipped."
else
  initialize_contract "BatchTransferProcessor" batchProcessorInitialized \
    --id "$BATCH_PROCESSOR_ID" \
    -- initialize \
    --admin "$AUTHORITY"
fi

TOKEN_ADDRESS="${TOKEN_ADDRESS:-}"
if [[ -z "$TOKEN_ADDRESS" ]]; then
  echo "ℹ️   TOKEN_ADDRESS not set — resolving the native XLM SAC for ${NETWORK}…"
  TOKEN_ADDRESS=$(stellar contract id asset --asset native --network "$NETWORK")
fi
TOKEN_VAULT_MAX_LIMIT="${TOKEN_VAULT_MAX_LIMIT:-1000000000000}"

echo "⚙️   Initialising TokenVault…"
if state_has tokenVaultInitialized; then
  echo "    already done on a previous run — skipped."
else
  initialize_contract "TokenVault" tokenVaultInitialized \
    --id "$TOKEN_VAULT_ID" \
    -- initialize \
    --owner "$AUTHORITY" \
    --token "$TOKEN_ADDRESS" \
    --max_limit "$TOKEN_VAULT_MAX_LIMIT"
fi

# ── Write IDs (resume/rollback record, #663) ──────────────────────────────────
state_set tokenVaultToken "$TOKEN_ADDRESS"
state_set tokenVaultMaxLimit "$TOKEN_VAULT_MAX_LIMIT"

echo ""
echo "✅  Deployment complete → $IDS_FILE"
cat "$IDS_FILE"