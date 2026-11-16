#!/usr/bin/env bash
# query.sh — Read protocol state from the CLI without signing a transaction.
#
# Usage:
#   ./scripts/query.sh [--json] <network> <stream_id> [fn] [source]
#   ./scripts/query.sh [--json] <network> factory <fn> [source]
#   ./scripts/query.sh [--json] <network> governor <fn> [source]
#   ./scripts/query.sh [--json] <network> token-vault <fn> [source]
#
# The source identity defaults to alice for compatibility with local setups.

set -euo pipefail

JSON_OUTPUT=false
ARGS=()
for arg in "$@"; do
  if [[ "$arg" == "--json" ]]; then
    JSON_OUTPUT=true
  else
    ARGS+=("$arg")
  fi
done

NETWORK="${ARGS[0]:-testnet}"
TARGET="${ARGS[1]?Usage: query.sh [--json] <network> <stream_id|factory|governor|token-vault> [fn] [source]}"
FN="${ARGS[2]:-info}"
SOURCE="${ARGS[3]:-alice}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IDS_FILE="$(cd "$SCRIPT_DIR/.." && pwd)/.contract-ids/$NETWORK.json"

FACTORY_ID=$(jq -r '.factory' "$IDS_FILE")

output_result() {
  local raw_output="$1"
  if [[ "$JSON_OUTPUT" == "true" ]]; then
    if echo "$raw_output" | jq . >/dev/null 2>&1; then
      echo "$raw_output" | jq -c .
    else
      jq -n --arg res "$raw_output" '{result: $res}'
    fi
  else
    echo "$raw_output"
  fi
}

case "$TARGET" in
  factory)
    RESULT=$(stellar contract invoke \
      --id "$FACTORY_ID" \
      --network "$NETWORK" --source "$SOURCE" \
      -- "$FN")
    output_result "$RESULT"
    exit 0
    ;;
  governor)
    GOVERNOR_ID=$(jq -r '.governor' "$IDS_FILE")
    RESULT=$(stellar contract invoke \
      --id "$GOVERNOR_ID" \
      --network "$NETWORK" --source "$SOURCE" \
      -- "$FN")
    output_result "$RESULT"
    exit 0
    ;;
  token-vault)
    TOKEN_VAULT_ID=$(jq -r '.token_vault' "$IDS_FILE")
    RESULT=$(stellar contract invoke \
      --id "$TOKEN_VAULT_ID" \
      --network "$NETWORK" --source "$SOURCE" \
      -- "$FN")
    output_result "$RESULT"
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
  if [[ "$JSON_OUTPUT" == "true" ]]; then
    jq -n --arg id "$STREAM_ID" --arg net "$NETWORK" '{"error": "Stream \($id) not found on \($net)"}' >&2
  else
    echo "❌  Stream $STREAM_ID not found on $NETWORK." >&2
  fi
  exit 1
fi

if [[ "$JSON_OUTPUT" != "true" ]]; then
  echo "Stream address: $STREAM_ADDR"
  echo ""
fi

RESULT=$(stellar contract invoke \
  --id "$STREAM_ADDR" \
  --network "$NETWORK" --source "$SOURCE" \
  -- "$FN")
output_result "$RESULT"
