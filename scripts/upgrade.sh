#!/usr/bin/env bash
# upgrade.sh — Upload new WASM and upgrade factory or governor.
#
# Usage:
#   ./scripts/upgrade.sh testnet factory
#   ./scripts/upgrade.sh testnet governor
#
# oracle, batch-processor, and token-vault are deployed by deploy.sh (#295)
# but are NOT supported here: none of their contracts currently expose an
# `upgrade` entry point (only DripFactory and DripGovernor do — see
# `pub fn upgrade` in contracts/factory/src/lib.rs and
# contracts/governor/src/lib.rs). Adding upgrade support for them means
# adding an admin-gated upgrade() function to each contract first, which is
# a contract-logic change, not a scripting one — tracked separately rather
# than faked here.

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

elif [[ "$CONTRACT" == "oracle" || "$CONTRACT" == "batch-processor" || "$CONTRACT" == "token-vault" ]]; then
  echo "❌  '$CONTRACT' has no on-chain 'upgrade' entry point yet — only 'factory' and 'governor' do." >&2
  echo "    See the note at the top of this script." >&2
  exit 1

else
  echo "❌  Unknown contract '$CONTRACT'. Use 'factory' or 'governor'." >&2
  exit 1
fi
