#!/usr/bin/env bash
set -euo pipefail
# Export gptimage sqlite → TNexus pool JSON (read-only on accounts.db).
ROOT="${TNEXUS_ROOT:-/root/TNexus}"
POOL_DIR="${POOL_DIR:-/opt/tnexus/data/pool}"
ACCOUNTS_DB="${ACCOUNTS_DB:-/root/gptimage/data/accounts.db}"
mkdir -p "$POOL_DIR"
export ACCOUNTS_DB OUT_PATH="$POOL_DIR/accounts_pool.json"
python3 "$ROOT/scripts/export_accounts_pool_full.py"
COUNT=$(python3 -c "import json; print(len(json.load(open('$OUT_PATH'))))")
echo "exported $COUNT accounts → $OUT_PATH"
touch "$POOL_DIR/scheduling_state.json" 2>/dev/null || echo '{}' > "$POOL_DIR/scheduling_state.json"
touch "$POOL_DIR/usage_events.ndjson" 2>/dev/null || : > "$POOL_DIR/usage_events.ndjson"
