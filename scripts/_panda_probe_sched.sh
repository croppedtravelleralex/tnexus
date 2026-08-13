#!/usr/bin/env bash
set -u

echo "=== tnexus_account_runtime contents ==="
docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "select email, scheduling_state, image_inflight, quota, image_quota_unknown from tnexus_account_runtime order by scheduling_state, email;" 2>&1 | head -40

echo
echo "=== how gateway computes image_schedulable ==="
grep -rn 'image_schedulable' /root/TNexus/crates/ --include=*.rs | head -20

echo
echo "=== schedulable decision fn ==="
grep -rn -B4 -A 30 'fn .*schedulable' /root/TNexus/crates/tnexus-accounts-db/src/*.rs /root/TNexus/crates/gateway/src/*.rs 2>/dev/null | head -70

echo
echo "=== fail streak / cooldown gating ==="
grep -rn 'image_fail_streak\|image_fail_cooldown_until\|image_next_ok_ts' /root/TNexus/crates/ --include=*.rs | head -20
