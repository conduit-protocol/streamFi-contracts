#!/usr/bin/env bash
# upgrade.sh — Upload new WASM and upgrade any deployed contract.
#
# Usage:
#   ./scripts/upgrade.sh testnet factory
#   ./scripts/upgrade.sh testnet governor
#   ./scripts/upgrade.sh testnet oracle
#   ./scripts/upgrade.sh testnet batch-processor
#   ./scripts/upgrade.sh testnet token-vault
#
# Every deployed contract exposes an admin/owner-gated `upgrade` entry point
# (issue #651). The BatchTransferProcessor's gate is the admin recorded by its
# `initialize` (deploy.sh passes the deploy authority), so that contract must
# have been initialized before it can be upgraded.

set -euo pipefail

NETWORK="${1:-testnet}"
CONTRACT="${2:-factory}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IDS_FILE="$ROOT_DIR/.contract-ids/$NETWORK.json"

if [[ ! -f "$IDS_FILE" ]]; then
  echo "❌  $IDS_FILE not found. Run deploy.sh first." >&2
  exit 1
fi

FACTORY_ID=$(jq -r '.factory'  "$IDS_FILE")
GOVERNOR_ID=$(jq -r '.governor' "$IDS_FILE")
ORACLE_ID=$(jq -r '.oracle'  "$IDS_FILE")
BATCH_PROCESSOR_ID=$(jq -r '.batchProcessor' "$IDS_FILE")
TOKEN_VAULT_ID=$(jq -r '.tokenVault' "$IDS_FILE")

# The upgrade authorities: governor address for the factory, deploy/admin
# address for the oracle, batch processor, and token vault.
AUTHORITY=$(stellar keys address dev 2>/dev/null || stellar keys address alice)

echo "🔨  Building contracts…"
cd "$ROOT_DIR"
cargo build --target wasm32-unknown-unknown --release --quiet

WASM_DIR="$ROOT_DIR/target/wasm32-unknown-unknown/release"

if [[ "$CONTRACT" == "factory" ]]; then
  echo "📤  Uploading new DripFactory WASM…"
  NEW_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_factory.wasm" \
    --network "$NETWORK" --source dev --quiet)
  echo "    New hash: $NEW_HASH"
  stellar contract invoke \
    --id "$FACTORY_ID" \
    --network "$NETWORK" --source dev \
    -- upgrade --new_wasm_hash "$NEW_HASH"
  echo "✅  DripFactory upgraded."

elif [[ "$CONTRACT" == "governor" ]]; then
  echo "📤  Uploading new DripGovernor WASM…"
  NEW_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_governor.wasm" \
    --network "$NETWORK" --source dev --quiet)
  echo "    New hash: $NEW_HASH"
  stellar contract invoke \
    --id "$GOVERNOR_ID" \
    --network "$NETWORK" --source dev \
    -- upgrade --new_wasm_hash "$NEW_HASH"
  echo "✅  DripGovernor upgraded."

elif [[ "$CONTRACT" == "oracle" ]]; then
  echo "📤  Uploading new TwapOracle WASM…"
  NEW_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_oracle.wasm" \
    --network "$NETWORK" --source dev --quiet)
  echo "    New hash: $NEW_HASH"
  stellar contract invoke \
    --id "$ORACLE_ID" \
    --network "$NETWORK" --source dev \
    -- upgrade --caller "$AUTHORITY" --new_wasm_hash "$NEW_HASH"
  echo "✅  TwapOracle upgraded."

elif [[ "$CONTRACT" == "batch-processor" ]]; then
  echo "📤  Uploading new BatchTransferProcessor WASM…"
  NEW_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/drip_batch_processor.wasm" \
    --network "$NETWORK" --source dev --quiet)
  echo "    New hash: $NEW_HASH"
  stellar contract invoke \
    --id "$BATCH_PROCESSOR_ID" \
    --network "$NETWORK" --source dev \
    -- upgrade --caller "$AUTHORITY" --new_wasm_hash "$NEW_HASH"
  echo "✅  BatchTransferProcessor upgraded."

elif [[ "$CONTRACT" == "token-vault" ]]; then
  echo "📤  Uploading new TokenVault WASM…"
  NEW_HASH=$(stellar contract upload \
    --wasm "$WASM_DIR/token_vault.wasm" \
    --network "$NETWORK" --source dev --quiet)
  echo "    New hash: $NEW_HASH"
  stellar contract invoke \
    --id "$TOKEN_VAULT_ID" \
    --network "$NETWORK" --source dev \
    -- upgrade --caller "$AUTHORITY" --new_wasm_hash "$NEW_HASH"
  echo "✅  TokenVault upgraded."

else
  echo "❌  Unknown contract '$CONTRACT'. Use 'factory', 'governor', 'oracle'," \
    "'batch-processor', or 'token-vault'." >&2
  exit 1
fi
