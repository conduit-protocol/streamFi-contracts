#!/usr/bin/env bash
# query.sh — Read protocol state from the CLI without signing a transaction.
#
# Usage:
#   ./scripts/query.sh <network> <stream_id> [fn] [source]
#   ./scripts/query.sh <network> factory <fn> [source]
#   ./scripts/query.sh <network> governor <fn> [source]
#
# The source identity defaults to alice for compatibility with local setups.

set -euo pipefail

NETWORK="${1:-testnet}"
TARGET="${2?Usage: query.sh <network> <stream_id|factory|governor> [fn] [source]}"
FN="${3:-info}"
SOURCE="${4:-alice}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDS_FILE="$(cd "$SCRIPT_DIR/.." && pwd)/.contract-ids/$NETWORK.json"

FACTORY_ID=$(jq -r '.factory' "$IDS_FILE")

case "$TARGET" in
  factory)
    stellar contract invoke \
      --id "$FACTORY_ID" \
      --network "$NETWORK" --source "$SOURCE" \
      -- "$FN"
    exit 0
    ;;
  governor)
    GOVERNOR_ID=$(jq -r '.governor' "$IDS_FILE")
    stellar contract invoke \
      --id "$GOVERNOR_ID" \
      --network "$NETWORK" --source "$SOURCE" \
      -- "$FN"
    exit 0
    ;;
esac

STREAM_ID="$TARGET"

# Resolve stream address from factory
STREAM_ADDR=$(stellar contract invoke \
  --id "$FACTORY_ID" \
  --network "$NETWORK" --source "$SOURCE" \
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
  --network "$NETWORK" --source "$SOURCE" \
  -- "$FN"
