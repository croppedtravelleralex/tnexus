#!/usr/bin/env bash
# Grok 号池额度体检：回答「额度是不是真的、有没有在自动刷新、谁被饿死了」。
#
# 用法（在 Panda 上执行，只读，不改任何数据）：
#   bash /root/TNexus/scripts/grok_quota_health.sh
#
# 判读要点：
# - source=upstream 才是上游真实回包；source=default 是从未探测过的占位值。
# - imagine 的 total 约 1.155e10 是上游「不限」哨兵值，不是真实张数。
# - 「陈旧」= fast 窗口 synced_at 超过 1 天；正常轮换下不应存在 active 陈旧账号。
set -euo pipefail

PG_CONTAINER="${PG_CONTAINER:-panda-postgres-1}"
GROK_CONTAINER="${GROK_CONTAINER:-panda-grok2api-rs-1}"
psql() { docker exec "$PG_CONTAINER" psql -U tnexus -d tnexus -P pager=off "$@"; }

echo "===== 1. 额度来源与真实性 ====="
psql -c "
SELECT mode, source, count(*) AS windows,
       sum(remaining) AS sum_remaining, sum(total) AS sum_total
FROM grok_quota_windows GROUP BY 1,2 ORDER BY 1,2;
"

echo "===== 2. 启用 grok_web 账号的 fast 窗口新鲜度 ====="
psql -c "
SELECT
  CASE
    WHEN w.synced_at IS NULL                     THEN 'z_无窗口'
    WHEN w.synced_at > now() - interval '1 hour' THEN 'a_1h内'
    WHEN w.synced_at > now() - interval '1 day'  THEN 'b_1d内'
    ELSE 'c_超1天_陈旧'
  END AS freshness,
  count(*)
FROM grok_accounts a
LEFT JOIN grok_quota_windows w ON w.account_id = a.id AND w.mode = 'fast'
WHERE a.provider = 'grok_web' AND a.enabled
GROUP BY 1 ORDER BY 1;
"

echo "===== 3. 被饿死的账号（active 但超 1 天没刷）====="
psql -c "
SELECT count(*) AS starved
FROM grok_accounts a
LEFT JOIN grok_quota_windows w ON w.account_id = a.id AND w.mode = 'fast'
WHERE a.provider = 'grok_web' AND a.enabled AND a.auth_status = 'active'
  AND (w.synced_at IS NULL OR w.synced_at < now() - interval '1 day');
"

echo "===== 4. 总额度（UI 顶部卡片的数据源）====="
psql -c "
SELECT w.mode, count(*) AS accounts,
       sum(w.remaining) AS remaining, sum(w.total) AS total,
       count(*) FILTER (WHERE w.remaining = 0 AND w.total > 0) AS exhausted,
       count(*) FILTER (WHERE w.synced_at IS NULL
                          OR w.synced_at < now() - interval '1 day') AS stale
FROM grok_quota_windows w
JOIN grok_accounts a ON a.id = w.account_id AND a.enabled
GROUP BY 1 ORDER BY 1;
"

echo "===== 5. 冷却状态（past 表示已过期、UI 不应再显示为冷却中）====="
psql -c "
SELECT CASE WHEN cooldown_until IS NULL THEN 'null'
            WHEN cooldown_until > now() THEN 'future_冷却中'
            ELSE 'past_已过期' END AS kind,
       count(*), max(cooldown_until) AS latest
FROM grok_accounts GROUP BY 1 ORDER BY 1;
"

echo "===== 6. 最近错误分布 ====="
psql -c "
SELECT left(coalesce(nullif(last_error, ''), '(空)'), 80) AS err, count(*)
FROM grok_accounts GROUP BY 1 ORDER BY count(*) DESC LIMIT 15;
"

echo "===== 7. 自动刷新任务是否在跑 ====="
docker logs "$GROK_CONTAINER" --since 30m 2>&1 | grep -E 'web_quota_refresh round' | tail -5 \
  || echo '(近 30 分钟无 web_quota_refresh 日志 — 任务可能未接线)'
docker logs "$GROK_CONTAINER" 2>&1 | grep -E 'web_quota_refresh (enabled|\()' | tail -2 || true
