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

set -euo pipefail

NETWORK="${1:-testnet}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT_DIR="$ROOT_DIR/.contract-ids"
IDS_FILE="$OUT_DIR/$NETWORK.json"

mkdir -p "$OUT_DIR"

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

# ── Upload WASMs ──────────────────────────────────────────────────────────────
echo "📤  Uploading DripStream WASM…"
STREAM_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/drip_stream.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)
echo "    DripStream WASM hash: $STREAM_WASM_HASH"

echo "📤  Uploading DripFactory WASM…"
FACTORY_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/drip_factory.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "📤  Uploading DripGovernor WASM…"
GOVERNOR_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/drip_governor.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "📤  Uploading TwapOracle WASM…"
ORACLE_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/drip_oracle.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "📤  Uploading BatchTransferProcessor WASM…"
BATCH_PROCESSOR_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/drip_batch_processor.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "📤  Uploading TokenVault WASM…"
TOKEN_VAULT_WASM_HASH=$(stellar contract upload \
  --wasm "$WASM_DIR/token_vault.wasm" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

# ── Deploy contracts ──────────────────────────────────────────────────────────
AUTHORITY=$(stellar keys address dev 2>/dev/null || stellar keys address alice)

echo "🚀  Deploying DripGovernor…"
GOVERNOR_ID=$(stellar contract deploy \
  --wasm-hash "$GOVERNOR_WASM_HASH" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "🚀  Deploying DripFactory…"
FACTORY_ID=$(stellar contract deploy \
  --wasm-hash "$FACTORY_WASM_HASH" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "🚀  Deploying TwapOracle…"
ORACLE_ID=$(stellar contract deploy \
  --wasm-hash "$ORACLE_WASM_HASH" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "🚀  Deploying BatchTransferProcessor…"
BATCH_PROCESSOR_ID=$(stellar contract deploy \
  --wasm-hash "$BATCH_PROCESSOR_WASM_HASH" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

echo "🚀  Deploying TokenVault…"
TOKEN_VAULT_ID=$(stellar contract deploy \
  --wasm-hash "$TOKEN_VAULT_WASM_HASH" \
  --network "$NETWORK" $IDENTITY \
  --quiet)

# ── Initialise contracts ──────────────────────────────────────────────────────
echo "⚙️   Initialising DripGovernor…"
stellar contract invoke \
  --id "$GOVERNOR_ID" \
  --network "$NETWORK" $IDENTITY \
  -- initialize \
  --authority "$AUTHORITY" \
  --fee_recipient "$AUTHORITY" \
  --factory_address "$FACTORY_ID"

echo "⚙️   Initialising DripFactory…"
stellar contract invoke \
  --id "$FACTORY_ID" \
  --network "$NETWORK" $IDENTITY \
  -- initialize \
  --stream_wasm_hash "$STREAM_WASM_HASH" \
  --governor "$GOVERNOR_ID"

echo "⚙️   Initialising TwapOracle…"
stellar contract invoke \
  --id "$ORACLE_ID" \
  --network "$NETWORK" $IDENTITY \
  -- initialize \
  --admin "$AUTHORITY"

# BatchTransferProcessor is stateless — it has no `initialize` entry point,
# so uploading + deploying is the whole setup.

TOKEN_ADDRESS="${TOKEN_ADDRESS:-}"
if [[ -z "$TOKEN_ADDRESS" ]]; then
  echo "ℹ️   TOKEN_ADDRESS not set — resolving the native XLM SAC for $NETWORK…"
  TOKEN_ADDRESS=$(stellar contract id asset --asset native --network "$NETWORK")
fi
TOKEN_VAULT_MAX_LIMIT="${TOKEN_VAULT_MAX_LIMIT:-1000000000000}"

echo "⚙️   Initialising TokenVault…"
stellar contract invoke \
  --id "$TOKEN_VAULT_ID" \
  --network "$NETWORK" $IDENTITY \
  -- initialize \
  --owner "$AUTHORITY" \
  --token "$TOKEN_ADDRESS" \
  --max_limit "$TOKEN_VAULT_MAX_LIMIT"

# ── Write IDs ─────────────────────────────────────────────────────────────────
cat > "$IDS_FILE" <<EOF
{
  "network":              "$NETWORK",
  "factory":              "$FACTORY_ID",
  "governor":              "$GOVERNOR_ID",
  "oracle":                "$ORACLE_ID",
  "batchProcessor":        "$BATCH_PROCESSOR_ID",
  "tokenVault":            "$TOKEN_VAULT_ID",
  "streamWasmHash":        "$STREAM_WASM_HASH",
  "tokenVaultToken":       "$TOKEN_ADDRESS",
  "tokenVaultMaxLimit":    "$TOKEN_VAULT_MAX_LIMIT"
}
EOF

echo ""
echo "✅  Deployment complete → $IDS_FILE"
cat "$IDS_FILE"
