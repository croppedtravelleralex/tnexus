#!/usr/bin/env python3
"""Gray comparison: since launch + since remediation."""
from __future__ import annotations

import re
import subprocess
from collections import Counter

GRAY_TS = 1785806300
# 2026-08-07 01:35 CST (号池修复 + keepalive 护栏部署)
REMED_TS = 1786037700
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


def window_stats(start: int) -> list[list[str]]:
    sql = f"""
    SELECT c.id, COUNT(*)::bigint,
           SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)::bigint,
           ROUND(100.0*SUM(CASE WHEN l.type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1),
           ROUND((PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY l.use_time))::numeric,0),
           ROUND((PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY l.use_time))::numeric,0)
    FROM logs l JOIN channels c ON c.id=l.channel_id
    WHERE l.channel_id IN (84,114) AND l.model_name='{MODEL}' AND l.created_at>={start}
    GROUP BY c.id ORDER BY c.id;
    """
    return [ln.split("|") for ln in psql(sql).strip().splitlines() if ln.strip()]


def print_window(label: str, start: int) -> None:
    rows = window_stats(start)
    print(f"\n=== {label} ===")
    if not rows:
        print("  (无数据)")
        return
    total = sum(int(r[1]) for r in rows)
    for r in rows:
        name = "gptimage" if r[0] == "84" else "TNexus"
        vol = 100.0 * int(r[1]) / max(total, 1)
        print(
            f"  ch{r[0]} {name}: {r[1]} 请求 ({vol:.1f}%) | "
            f"成功 {r[2]} | 成功率 {r[3]}% | P50 {r[4]}s | P90 {r[5]}s"
        )
    if len(rows) == 2:
        d = float(rows[1][3]) - float(rows[0][3])
        print(f"  Δ成功率 TNexus vs gptimage: {d:+.1f}pp")


def main() -> None:
    print_window("灰测上线以来 (ch114 2026-08-04 09:18 CST)", GRAY_TS)
    print_window("号池修复以来 (2026-08-07 01:35 CST)", REMED_TS)

    day_start = psql(
        "SELECT extract(epoch from timestamptz '2026-08-07 00:00:00+08')::bigint;"
    ).strip()
    raw = psql(
        f"SELECT other FROM logs WHERE channel_id=114 AND model_name='{MODEL}' "
        f"AND type!=2 AND created_at >= {day_start}"
    )
    errs = Counter()
    for line in raw.splitlines():
        m = re.search(r'"error_code":"([^"]+)"', line)
        errs[m.group(1) if m else "unknown"] += 1
    print("\n=== 今日 ch114 失败 error_code ===")
    for e, c in errs.most_common(12):
        print(f"  {e}: {c}")

    # ch115 dedicated since launch
    sql = f"""
    SELECT COUNT(*)::bigint,
           SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)::bigint,
           ROUND(100.0*SUM(CASE WHEN type=2 THEN 1 ELSE 0 END)/NULLIF(COUNT(*),0),1)
    FROM logs WHERE channel_id=115 AND model_name='{MODEL}' AND created_at>={GRAY_TS};
    """
    r = psql(sql).strip().split("|")
    if r and r[0] != "0":
        print(f"\n=== ch115 专用通道 (非灰测分流) ===")
        print(f"  {r[0]} 请求 | 成功 {r[1]} | 成功率 {r[2]}%")


if __name__ == "__main__":
    main()
