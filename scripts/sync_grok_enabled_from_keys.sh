#!/usr/bin/env bash
# 按 pure_http_keys 同步 grok_accounts.enabled（仅需 psql，无 Python 依赖）。
set -euo pipefail

KEYS_DIR="${KEYS_DIR:-/opt/tnexus/pure_http_keys}"
DSN="${GROK_DATABASE_URL:-}"
APPLY=0
DRY_RUN=1

usage() {
  echo "Usage: GROK_DATABASE_URL=... $0 [--keys-dir DIR] [--apply]"
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --keys-dir) KEYS_DIR="$2"; shift 2 ;;
    --apply) APPLY=1; DRY_RUN=0; shift ;;
    --dry-run) DRY_RUN=1; APPLY=0; shift ;;
    -h|--help) usage ;;
    *) echo "unknown arg: $1"; usage ;;
  esac
done

[ -n "$DSN" ] || { echo "GROK_DATABASE_URL required" >&2; exit 1; }
[ -d "$KEYS_DIR" ] || { echo "keys dir missing: $KEYS_DIR" >&2; exit 1; }

enable_ids=()
for f in "$KEYS_DIR"/account_*.json; do
  [ -f "$f" ] || continue
  id="${f##*/account_}"; id="${id%.json}"
  if python3 -c "import json,sys; d=json.load(open(sys.argv[1])); fp=str(d.get('fingerprint','')).strip(); sys.exit(0 if fp else 1)" "$f" 2>/dev/null; then
    enable_ids+=("$id")
  fi
done

if [ ${#enable_ids[@]} -eq 0 ]; then
  echo "no valid keys in $KEYS_DIR" >&2
  exit 1
fi

ids_csv=$(IFS=,; echo "${enable_ids[*]}")
echo "keys_dir=$KEYS_DIR enable_count=${#enable_ids[@]} ids=${ids_csv}"

if [ "$DRY_RUN" -eq 1 ]; then
  echo "DRY-RUN: would ENABLE (${ids_csv})"
  psql "$DSN" -Atc "SELECT id,enabled FROM grok_accounts WHERE provider='grok_web' AND id IN (${ids_csv//,/,}) ORDER BY id LIMIT 20;"
  exit 0
fi

psql "$DSN" -v ON_ERROR_STOP=1 <<SQL
UPDATE grok_accounts SET enabled = true, updated_at = now()
 WHERE provider = 'grok_web' AND id IN (${ids_csv});
UPDATE grok_accounts SET enabled = false, updated_at = now()
 WHERE provider = 'grok_web' AND id NOT IN (${ids_csv});
SQL

echo "applied OK"
psql "$DSN" -Atc "SELECT count(*) FILTER (WHERE enabled) AS enabled, count(*) AS total FROM grok_accounts WHERE provider='grok_web';"
