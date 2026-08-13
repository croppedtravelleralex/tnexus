#!/usr/bin/env bash
# GPT 号池 access_token 到期盘点：解出 JWT 的 exp，按剩余天数分桶。
#
# 用法（Panda，只读）：bash /root/TNexus/scripts/gpt_pool_token_expiry.sh
#
# 背景：号池凭据是 OpenAI OAuth JWT（约 10 天有效期），account-ops 里没有周期性
# 刷新 loop，refresh 只能手动触发。全部过期 = 生图与对话整体 401。
set -uo pipefail

PG_CONTAINER="${PG_CONTAINER:-panda-postgres-1}"
DB_USER="${DB_USER:-tnexus}"
DB_NAME="${DB_NAME:-tnexus}"

# access_token 是顶层列，不在 data JSONB 里。
docker exec "$PG_CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -tAc \
  "SELECT email || '|' || access_token FROM tnexus_accounts;" \
  > /tmp/_pool_tokens.tsv 2>/dev/null || {
    echo "读取 tnexus_accounts 失败" >&2; exit 1; }

python3 - /tmp/_pool_tokens.tsv <<'PY'
import base64, json, sys, time
from collections import Counter

now = int(time.time())
buckets = Counter()
soonest = []
total = 0
no_token = 0
undecodable = 0

for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    line = line.rstrip("\n")
    if not line:
        continue
    total += 1
    email, _, token = line.partition("|")
    token = token.strip()
    if not token:
        no_token += 1
        continue
    parts = token.split(".")
    if len(parts) != 3:
        undecodable += 1
        continue
    try:
        payload = parts[1] + "=" * (-len(parts[1]) % 4)
        exp = json.loads(base64.urlsafe_b64decode(payload)).get("exp")
    except Exception:
        undecodable += 1
        continue
    if not exp:
        undecodable += 1
        continue
    days = (exp - now) / 86400
    if days < 0:      buckets["a_已过期"] += 1
    elif days < 1:    buckets["b_24h内"] += 1
    elif days < 3:    buckets["c_3天内"] += 1
    elif days < 7:    buckets["d_7天内"] += 1
    else:             buckets["e_7天以上"] += 1
    soonest.append((exp, email))

print(f"账号总数 {total}  无 token {no_token}  无法解析 {undecodable}")
print("--- 按剩余有效期分桶 ---")
for k in sorted(buckets):
    print(f"  {k:12s} {buckets[k]}")

soonest.sort()
if soonest:
    import datetime
    def fmt(ts):
        return datetime.datetime.utcfromtimestamp(ts).strftime("%Y-%m-%d %H:%M UTC")
    print(f"--- 最早到期 5 个 ---")
    for exp, email in soonest[:5]:
        print(f"  {fmt(exp)}  {email}")
    print(f"--- 最晚到期（全池归零时刻）---")
    exp, email = soonest[-1]
    print(f"  {fmt(exp)}  {email}")
    left = (exp - now) / 86400
    print(f"  距今 {left:.1f} 天")
PY
