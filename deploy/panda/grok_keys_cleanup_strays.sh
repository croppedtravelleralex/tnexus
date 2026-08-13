#!/usr/bin/env bash
# 清理 pure_http_keys 中非 account_{数字}.json 的旁路文件。
# 运行时（session_store.rs）与 sync_grok_enabled_from_keys.sh 都只认 account_{数字}.json，
# 这些旁路文件（account_N_at_oldpool.local.json / gate_account_*.json）不会被加载。
# 先打 tar 备份再删除，可回滚。
set -euo pipefail

KEYS_DIR="${KEYS_DIR:-/opt/tnexus/pure_http_keys}"
BACKUP_DIR="${BACKUP_DIR:-/root/grok_keys_strays_backup}"
APPLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --keys-dir) KEYS_DIR="$2"; shift 2 ;;
    --apply) APPLY=1; shift ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

[ -d "$KEYS_DIR" ] || { echo "keys dir missing: $KEYS_DIR" >&2; exit 1; }

mapfile -t strays < <(cd "$KEYS_DIR" && ls -1 | grep -vE '^account_[0-9]+\.json$' || true)

echo "keys_dir=$KEYS_DIR stray_count=${#strays[@]}"
if [ "${#strays[@]}" -eq 0 ]; then
  echo "nothing to clean"
  exit 0
fi

printf '%s\n' "${strays[@]}"

if [ "$APPLY" -ne 1 ]; then
  echo "DRY-RUN: rerun with --apply to archive+delete"
  exit 0
fi

mkdir -p "$BACKUP_DIR"
stamp=$(date +%Y%m%d_%H%M%S)
tar -C "$KEYS_DIR" -czf "$BACKUP_DIR/strays_$stamp.tar.gz" "${strays[@]}"
echo "backup=$BACKUP_DIR/strays_$stamp.tar.gz"

for f in "${strays[@]}"; do
  rm -f -- "$KEYS_DIR/$f"
done

echo "deleted=${#strays[@]}"
echo "remaining_total=$(ls -1 "$KEYS_DIR" | wc -l)"
echo "remaining_valid=$(ls -1 "$KEYS_DIR" | grep -cE '^account_[0-9]+\.json$')"
