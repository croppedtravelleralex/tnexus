#!/usr/bin/env python3
"""Gray comparison ch84 vs ch114 since ch114 launch."""
from __future__ import annotations

import json
import re
import subprocess
from collections import Counter
from datetime import datetime, timezone

GRAY_TS = 1785806300
MODEL = "gpt-image-2"


def psql(sql: str) -> str:
    return subprocess.check_output(
        [
            "docker",
            "exec",
            "new-api-postgres",
            "psql",
            "-U",
            "newapi",
            "-d",
            "new-api",
            "-tAc",
            sql,
        ],
        text=True,
    )


def main() -> None:
    now = int(psql("SELECT extract(epoch from now())::bigint;").strip())
    start = datetime.fromtimestamp(GRAY_TS, tz=timezone.utc).astimezone()

    print("=" * 62)
    print("灰测两侧对比（ch84 gptimage :8012  vs  ch114 TNexus :8014）")
    print(f"窗口: {start.strftime('%Y-%m-%d %H:%M CST')} → 现在")
    print("权重: ch84=70 / ch114=30（上线后未调整）")
    print("=" * 62)

    sql = f"""
    SELECT c.id, c.name, c.weight,
           COUNT(*)::bigint,
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)::bigint,
           SUM(CASE WHEN l.type!=2 THEN 1 ELSE 0 END)::bigint,
           ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1),
           ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,1),
           ROUND((PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY l.use_time))::numeric,1),
           ROUND(AVG(l.use_time)::numeric,1)
    FROM logs l JOIN channels c ON c.id=l.channel_id
    WHERE l.channel_id IN (84,114) AND l.model_name='{MODEL}' AND l.created_at>={GRAY_TS}
    GROUP BY c.id, c.name, c.weight ORDER BY c.id;
    """
    rows = [ln.split("|") for ln in psql(sql).strip().splitlines() if ln.strip()]
    total_req = sum(int(r[3]) for r in rows)
    print("\n【总体】")
    hdr = f"{'渠道':<20} {'权':>3} {'请求':>6} {'流量':>6} {'成功':>6} {'失败':>6} {'成功率':>7} {'P50':>6} {'P90':>6} {'均值':>6}"
    print(hdr)
    print("-" * len(hdr))
    for r in rows:
        cid, _name, w, req, ok, fail, pct, p50, p90, avg = r
        vol = 100.0 * int(req) / max(total_req, 1)
        label = "gptimage" if cid == "84" else "TNexus"
        print(
            f"ch{cid} {label:<13} {w:>3} {req:>6} {vol:>5.1f}% {ok:>6} {fail:>6} {pct:>6}% "
            f"{p50:>5}s {p90:>5}s {avg:>5}s"
        )
    if len(rows) == 2:
        p84, p114 = float(rows[0][6]), float(rows[1][6])
        lat84, lat114 = float(rows[0][7]), float(rows[1][7])
        print(f"\n  Δ成功率: TNexus {p114 - p84:+.1f}pp | ΔP50: TNexus {lat114 - lat84:+.1f}s（负=更快）")

    sql = f"""
    SELECT to_char(to_timestamp(l.created_at) AT TIME ZONE 'Asia/Shanghai','MM-DD') as day,
           l.channel_id,
           COUNT(*)::bigint,
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)::bigint,
           ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/COUNT(*),1),
           ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,0)
    FROM logs l
    WHERE l.model_name='{MODEL}' AND l.channel_id IN (84,114) AND l.created_at>={GRAY_TS}
    GROUP BY day, l.channel_id ORDER BY day, l.channel_id;
    """
    print("\n【按日（CST）】")
    by_day: dict[str, dict[str, tuple]] = {}
    for ln in psql(sql).strip().splitlines():
        if not ln.strip():
            continue
        day, ch, req, ok, pct, p50 = ln.split("|")
        by_day.setdefault(day, {})[ch] = (req, ok, pct, p50)
    print(f"{'日期':<6} {'gptimage 请求/成功率/P50':<28} {'TNexus 请求/成功率/P50':<28}")
    for day in sorted(by_day):
        g = by_day[day].get("84", ("-", "-", "-", "-"))
        t = by_day[day].get("114", ("-", "-", "-", "-"))
        gs = f"{g[0]}次 {g[2]}% P50={g[3]}s" if g[0] != "-" else "—"
        ts = f"{t[0]}次 {t[2]}% P50={t[3]}s" if t[0] != "-" else "—"
        print(f"  {day}  {gs:<28} {ts}")

    raw = psql(
        f"SELECT channel_id||chr(124)||other FROM logs WHERE channel_id IN (84,114) "
        f"AND model_name='{MODEL}' AND type!=2 AND created_at>={GRAY_TS}"
    )
    by: dict[int, Counter[str]] = {84: Counter(), 114: Counter()}
    for line in raw.splitlines():
        if "|" not in line:
            continue
        ch_s, other = line.split("|", 1)
        ch = int(ch_s)
        m = re.search(r'"error_code":"([^"]+)"', other)
        by[ch][m.group(1) if m else "unknown"] += 1

    print("\n【错误码 TOP】")
    codes = sorted(set(by[84]) | set(by[114]), key=lambda c: -(by[84][c] + by[114][c]))
    print(f"{'error_code':<28} {'ch84':>6} {'ch114':>6} {'计':>6}")
    for c in codes[:12]:
        a, b = by[84][c], by[114][c]
        print(f"{c:<28} {a:>6} {b:>6} {a+b:>6}")

    try:
        out = subprocess.check_output(
            [
                "docker",
                "exec",
                "chatgpt2api-local",
                "/app/.venv/bin/python",
                "-c",
                "import sys;sys.path.insert(0,'/app');from services.account_service import account_service as s;a=s.list_accounts();print(len(a),sum(1 for x in a if s._is_image_account_schedulable(x)),sum(1 for x in a if str(x.get('status'))=='异常'))",
            ],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip().split()
        print(f"\n【号池】总数 {out[0]} | 可调度 {out[1]} | 异常 {out[2]}")
    except Exception:
        pass

    path = "/opt/tnexus/data/pool/pipeline_events.ndjson"
    cut = now - 7 * 86400
    ok_n = fail_n = 0
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                try:
                    o = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if o.get("kind") != "gateway_image":
                    continue
                ts = o.get("ts", "")
                try:
                    t = datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()
                except ValueError:
                    continue
                if t < cut:
                    continue
                if o.get("ok"):
                    ok_n += 1
                else:
                    fail_n += 1
        tot = ok_n + fail_n
        print(f"【Gateway 直连 pipeline 近7天】{tot} 次 | 成功 {ok_n} | 失败 {fail_n} | {100*ok_n/max(tot,1):.1f}%")
    except OSError:
        pass


if __name__ == "__main__":
    main()
