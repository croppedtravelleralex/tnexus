#!/usr/bin/env bash
# 修正被批量导入打乱的 BIGSERIAL 序列。
#
# 背景：ETL 灌数据时带显式 id 写入，不会推进 nextval。序列仍从低位发号，之后任何
# 新插入都撞主键。线上表现为 grok_request_audits 审计一行写不进去，报
# 「duplicate key value violates unique constraint」，而失败又被 sink 静默吞掉。
#
# 用法（Panda）：
#   bash scripts/fix_pg_sequences.sh          # 只体检，不修改
#   bash scripts/fix_pg_sequences.sh --apply  # 实际修正
set -uo pipefail

PG_CONTAINER="${PG_CONTAINER:-panda-postgres-1}"
DB_USER="${DB_USER:-tnexus}"
DB_NAME="${DB_NAME:-tnexus}"
APPLY="${1:-}"

# 列出所有「有序列的主键列」，比较序列当前值与表内 max(id)。
READ_SQL="
SELECT
  c.relname AS seq,
  t.relname AS tbl,
  a.attname AS col,
  COALESCE(s.last_value, 0) AS last_value
FROM pg_class c
JOIN pg_depend d   ON d.objid = c.oid AND d.deptype = 'a'
JOIN pg_class t    ON t.oid = d.refobjid
JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid
LEFT JOIN pg_sequences s ON s.sequencename = c.relname
WHERE c.relkind = 'S' AND t.relname LIKE 'grok\\_%'
ORDER BY t.relname;
"

echo '=== 序列体检（seq_last vs table_max）==='
docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -tAF'|' -c "$READ_SQL" \
| while IFS='|' read -r seq tbl col last_value; do
    [ -z "${seq:-}" ] && continue
    maxid=$(docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -tAc \
      "SELECT COALESCE(max($col), 0) FROM $tbl;")
    if [ "${maxid:-0}" -gt "${last_value:-0}" ]; then
      printf '  %-40s seq=%-10s max=%-10s  ** 落后，会撞主键 **\n' "$tbl.$col" "$last_value" "$maxid"
      if [ "$APPLY" = "--apply" ]; then
        docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -tAc \
          "SELECT setval('$seq', $maxid, true);" >/dev/null
        echo "     → 已置为 $maxid"
      fi
    else
      printf '  %-40s seq=%-10s max=%-10s  ok\n' "$tbl.$col" "$last_value" "$maxid"
    fi
  done

if [ "$APPLY" != "--apply" ]; then
  echo
  echo '（只读体检。加 --apply 实际修正）'
fi
