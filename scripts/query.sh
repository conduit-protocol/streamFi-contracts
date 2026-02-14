#!/usr/bin/env bash
# query.sh — Read stream state from the CLI without signing a transaction.
#
# Usage:
#   ./scripts/query.sh testnet <stream_id>
#   ./scripts/query.sh testnet <stream_id> withdrawable

set -euo pipefail

NETWORK="${1:-testnet}"
STREAM_ID="${2?Usage: query.sh <network> <stream_id> [fn]}"
FN="${3:-info}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDS_FILE="$(cd "$SCRIPT_DIR/.." && pwd)/.contract-ids/$NETWORK.json"

FACTORY_ID=$(jq -r '.factory' "$IDS_FILE")

# Resolve stream address from factory
STREAM_ADDR=$(stellar contract invoke \
  --id "$FACTORY_ID" \
  --network "$NETWORK" --source alice \
  -- stream_address \
  --stream_id "$STREAM_ID" 2>/dev/null | tr -d '"')

if [[ -z "$STREAM_ADDR" || "$STREAM_ADDR" == "null" ]]; then
  echo "❌  Stream $STREAM_ID not found on $NETWORK." >&2
  exit 1
fi

echo "Stream address: $STREAM_ADDR"
echo ""

stellar contract invoke \
  --id "$STREAM_ADDR" \
  --network "$NETWORK" --source alice \
  -- "$FN"
