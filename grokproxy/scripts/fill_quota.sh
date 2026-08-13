#!/usr/bin/env bash
# Measure every schedulable Build account's entitlement.
#
# A probe costs a few hundred tokens out of a million, so measuring the whole
# pool is cheap; the payoff is a budget figure that is not mostly "unknown".
set -u
set -a; . /opt/grokproxy/.env; set +a
base=http://127.0.0.1:8110
A=(-H "Authorization: Bearer $GROKPROXY_ADMIN_KEY")

budget() {
  curl -s --max-time 30 "$base/api/v1/stats" "${A[@]}" > /tmp/st.json
  python3 - <<'PY'
import json
s = json.load(open('/tmp/st.json'))
for provider in ('build', 'web'):
    p = s.get(provider) or {}
    h, q = p.get('health', {}), p.get('quota', {})
    total = sum(h.values())
    mix = " ".join(f"{k}={v}" for k, v in sorted(h.items()))
    print(f"  {provider:6} {total:>5} 个  [{mix}]")
    if provider == 'build':
        m = q.get('measured_accounts', 0)
        print(f"         已探测 {m}/{total}  授权 {q.get('entitled_tokens',0):,}  "
              f"剩余 {q.get('remaining_tokens',0):,}  已用 {q.get('spent_tokens',0):,}")
PY
}

echo "=== before ==="; budget

echo
echo "=== probing (unmeasured accounts first) ==="
for round in 1 2 3; do
  echo "--- round $round ---"
  curl -s --max-time 1800 -X POST "$base/api/v1/quota?limit=200&concurrency=10" "${A[@]}" \
    | python3 -c 'import json,sys; r=json.load(sys.stdin).get("report",{}); print("   ", {k:v for k,v in r.items() if v})'
done

echo
echo "=== after ==="; budget
