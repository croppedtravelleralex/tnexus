#!/usr/bin/env python3
"""Gray comparison since TNexus weight went live (ch114 created_time)."""
from __future__ import annotations

import re
import subprocess
from collections import Counter
from datetime import datetime, timezone

MODEL = "gpt-image-2"


def psql(sql: str) -> str:
    return subprocess.check_output(
        [
            "docker", "exec", "new-api-postgres",
            "psql", "-U", "newapi", "-d", "new-api", "-tAc", sql,
        ],
        text=True,
    )


def main() -> None:
    gray_ts = int(psql("SELECT created_time FROM channels WHERE id=114;").strip())
    now = int(psql("SELECT extract(epoch from now())::bigint;").strip())
    weights = psql(
        "SELECT id||':'||name||' w='||weight FROM channels WHERE id IN (84,114,115) ORDER BY id;"
    ).strip()
    start_cst = datetime.fromtimestamp(gray_ts, tz=timezone.utc).astimezone()
    print("window_start_utc", datetime.fromtimestamp(gray_ts, tz=timezone.utc).isoformat())
    print("window_start_local", start_cst.strftime("%Y-%m-%d %H:%M %Z"))
    print("window_end_now", datetime.fromtimestamp(now, tz=timezone.utc).isoformat())
    print("channels", weights.replace("\n", " | "))

    sql = f"""
    SELECT c.id||'|'||c.name||'|'||c.weight||'|'||
           COUNT(*)||'|'||
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)||'|'||
           SUM(CASE WHEN l.type!=2 THEN 1 ELSE 0 END)||'|'||
           COALESCE(ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1)::text,'0')||'|'||
           COALESCE(ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')||'|'||
           COALESCE(ROUND((PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY l.use_time))::numeric,1)::text,'0')||'|'||
           COALESCE(ROUND(AVG(l.use_time)::numeric,1)::text,'0')
    FROM logs l JOIN channels c ON c.id=l.channel_id
    WHERE l.channel_id IN (84,114) AND l.model_name='{MODEL}' AND l.created_at>={gray_ts}
    GROUP BY c.id,c.name,c.weight ORDER BY c.id;
    """
    lines = [ln for ln in psql(sql).strip().splitlines() if ln.strip()]
    total_req = sum(int(ln.split("|")[3]) for ln in lines if "|" in ln)
    print(f"\n=== gpt-image-2 灰测对比 ch84 vs ch114（自 ch114 上线至今，权重未再调整仍为 70/30）===")
    print(f"总请求 {total_req}")
    for line in lines:
        p = line.split("|")
        if len(p) < 10:
            continue
        cid, name, w, req, ok, fail, ok_pct, p50, p90, avg = p[:10]
        vol = 100.0 * int(req) / max(total_req, 1)
        print(
            f"  ch{cid} {name} w={w}: req={req} ({vol:.1f}%流量) ok={ok} fail={fail} "
            f"成功率={ok_pct}% p50={p50}s p90={p90}s avg={avg}s"
        )

    # daily
    print("\n=== 按日（CST 日期）===")
    sql = f"""
    SELECT to_char(to_timestamp(l.created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD')||'|'||
           l.channel_id||'|'||COUNT(*)||'|'||
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)||'|'||
           ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/COUNT(*),1)
    FROM logs l
    WHERE l.model_name='{MODEL}' AND l.channel_id IN (84,114) AND l.created_at>={gray_ts}
    GROUP BY 1,l.channel_id ORDER BY 1,l.channel_id;
    """
    for line in psql(sql).strip().splitlines():
        if not line.strip():
            continue
        day, ch, req, ok, pct = line.split("|")
        label = "gptimage" if ch == "84" else "TNexus"
        print(f"  {day} ch{ch}({label}): {req} req, ok={ok}, {pct}%")

    # errors
    print("\n=== 错误码对比（ch84 vs ch114，同期）===")
    raw = psql(
        f"SELECT channel_id||'|'||other FROM logs WHERE channel_id IN (84,114) "
        f"AND model_name='{MODEL}' AND type!=2 AND created_at>={gray_ts};"
    )
    by_ch: dict[int, Counter[str]] = {84: Counter(), 114: Counter()}
    for line in raw.splitlines():
        if "|" not in line:
            continue
        ch_s, other = line.split("|", 1)
        try:
            ch = int(ch_s)
        except ValueError:
            continue
        m = re.search(r'"error_code":"([^"]+)"', other)
        code = m.group(1) if m else "unknown"
        by_ch.setdefault(ch, Counter())[code] += 1

    all_codes = sorted(set(by_ch[84]) | set(by_ch[114]), key=lambda c: -(by_ch[84][c] + by_ch[114][c]))
    print(f"{'error_code':<28} {'ch84':>6} {'ch114':>6} {'合计':>6}")
    for code in all_codes[:15]:
        a, b = by_ch[84][code], by_ch[114][code]
        print(f"{code:<28} {a:>6} {b:>6} {a+b:>6}")

    # ch115 separate (not gray)
    sql115 = f"""
    SELECT COUNT(*), SUM(CASE WHEN type=2 THEN 1 ELSE 0 END), SUM(CASE WHEN type!=2 THEN 1 ELSE 0 END)
    FROM logs WHERE channel_id=115 AND model_name='{MODEL}' AND created_at>={gray_ts};
    """
    r = psql(sql115).strip()
    if r:
        req, ok, fail = r.split("|")
        print(f"\n=== ch115 dedicated（非灰测流量，同期参考）req={req} ok={ok} fail={fail} ===")


if __name__ == "__main__":
    main()
