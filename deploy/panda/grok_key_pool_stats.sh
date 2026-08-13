#!/usr/bin/env bash
# Panda 侧统计：key 文件数、旁路文件、grok_accounts enabled 数量
set -uo pipefail

KEYS_DIR="${KEYS_DIR:-/opt/tnexus/pure_http_keys}"

echo "=== keys dir: $KEYS_DIR ==="
total=$(ls -1 "$KEYS_DIR" 2>/dev/null | wc -l)
valid=$(ls -1 "$KEYS_DIR" 2>/dev/null | grep -cE '^account_[0-9]+\.json$')
stray=$(ls -1 "$KEYS_DIR" 2>/dev/null | grep -vE '^account_[0-9]+\.json$' | wc -l)
echo "total_files=$total"
echo "valid_account_json=$valid"
echo "stray_files=$stray"

echo "=== stray file names ==="
ls -1 "$KEYS_DIR" 2>/dev/null | grep -vE '^account_[0-9]+\.json$' || true

echo "=== keys with non-empty fingerprint ==="
fp_ok=0
fp_bad=0
bad_ids=""
for f in "$KEYS_DIR"/account_[0-9]*.json; do
  [ -e "$f" ] || continue
  base=$(basename "$f")
  case "$base" in
    account_*_*) continue ;;
  esac
  aid="${base#account_}"; aid="${aid%.json}"
  case "$aid" in
    ''|*[!0-9]*) continue ;;
  esac
  fp=$(python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(len((d.get('fingerprint') or '').strip()))" "$f" 2>/dev/null || echo 0)
  if [ "${fp:-0}" -ge 8 ]; then
    fp_ok=$((fp_ok+1))
  else
    fp_bad=$((fp_bad+1))
    bad_ids="$bad_ids,$aid"
  fi
done
echo "panda_fp_usable=$fp_ok"
echo "panda_fp_unusable=$fp_bad"
echo "panda_fp_unusable_ids=${bad_ids#,}"

echo "=== DB stats ==="
set -a; source /opt/tnexus/.env 2>/dev/null; set +a
DSN="${GROK_DATABASE_URL:-${DATABASE_URL:-}}"
if [ -z "$DSN" ]; then
  echo "no DSN found in /opt/tnexus/.env"
  exit 0
fi
psql "$DSN" -At -c "SELECT 'total_accounts='||count(*) FROM grok_accounts;"
psql "$DSN" -At -c "SELECT 'enabled_accounts='||count(*) FROM grok_accounts WHERE enabled;"
psql "$DSN" -At -c "SELECT 'disabled_accounts='||count(*) FROM grok_accounts WHERE NOT enabled;"
