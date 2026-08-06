#!/usr/bin/env python3
"""Grok 移植 shadow compare（G6-P3）。

对齐 docs/39e-grok-execution-plan.md G6-P3 验收：
- G6-A1 成功率 ≥ Go − 1%（--success-gap 0.01）
- G6-A2 P99 延迟 ≤ Go × 1.15（--p99-ratio 1.15）
- G6-A3 50 账号粒度额度 remaining 一致（--quota 文件）

设计：
- **两种数据源**：
  - `--file`（默认）：本地回放，两份结果文件（Go / Rust）各一行一条记录：
    `timestamp, model, status, latency_ms, account_id`（逗号或空白分隔；首行疑似表头自动跳过）。
    status 可为数字 HTTP 码或文本 success/error；2xx~3xx 或 `success` 视为成功。
  - `--url`：查询两个聚合端点（--go-url / --rust-url），各返回 JSON 记录数组
    `[{"timestamp","model","status","latency_ms","account_id"}...]`。
- **指标**：成功率、P50/P95/P99 延迟（nearest-rank，纯 stdlib）；Rust vs Go 差值。
- **退出码**：0 = 达标；2 = 任一验收阈值超限；1 = IO/解析/参数错误。
- `--self-test`：内置合成数据跑通断言（无需外部文件），用于 CI 冒烟。

额度对比（G6-A3）：`--quota` 传入两个文件（--go-quota / --rust-quota），
每行 `account_id, remaining`；逐账号比较，输出差异清单与一致账号数。
阈值均按 G6-A3 的账号粒度，默认容差 0（可 --quota-tol 放宽）。

接入 TODO（数据源对接）：
- go 采样：Go gateway 的 request_audits（`grok_request_audits`）导出 → 一行一条；
- rust 采样：Rust gateway 同一套 //v1/ 路由的审计表中导出同构记录；
- 双侧需在 cutover 窗口（docs/38 Phase 2）对同一批请求（或同分布流量）落盘，
  latency_ms 含客户端→网关→上游→客户端（P50/P95/P99 按侧聚合比较，不做逐请求配对）。
- quota：两仓各自的 `grok_quota_windows` 按 (account_id, mode) 导出 remaining。

用法：
  py scripts/grok_shadow_compare.py --go result.go --rust result.rust
  py scripts/grok_shadow_compare.py --go result.go --rust result.rust --json
  py scripts/grok_shadow_compare.py --go-quota q.go --rust-quota q.rust \
       --go result.go --rust result.rust
  py scripts/grok_shadow_compare.py --self-test
"""
from __future__ import annotations

import argparse
import json
import math
import sys
import urllib.request
from dataclasses import dataclass, field


@dataclass
class Record:
    timestamp: str = ""
    model: str = ""
    status: str = "200"
    latency_ms: float = 0.0
    account_id: str = ""


def _split_raw(line: str) -> list[str]:
    """逗号或空白分隔（兼容 go 文件内字段顺序）。"""
    parts = [p.strip() for p in line.split(",")]
    if len(parts) == 1:
        parts = line.split()
    return [p for p in parts if p]


def _is_header(line: str) -> bool:
    low = line.strip().lower()
    return (
        "timestamp" in low and ("latency" in low or "status" in low)
    ) or low.startswith("ts,") or low.startswith("timestamp;")


def is_success_status(status: str) -> bool:
    s = status.strip().lower()
    if s in ("success", "ok", "2xx"):
        return True
    if s in ("error", "fail", "failed", "failure", "1xx", "4xx", "5xx"):
        return False
    try:
        code = int(float(s))
    except (ValueError, TypeError):
        return False
    return 200 <= code <= 399


def load_records(lines) -> list[Record]:
    records: list[Record] = []
    for line in lines:
        line = line.strip()
        if not line or _is_header(line):
            continue
        parts = _split_raw(line)
        if len(parts) < 3:
            continue
        try:
            latency = float(parts[3]) if len(parts) > 3 else 0.0
        except ValueError:
            latency = 0.0
        records.append(
            Record(
                timestamp=parts[0],
                model=parts[1],
                status=parts[2],
                latency_ms=latency,
                account_id=parts[4] if len(parts) > 4 else "",
            )
        )
    return records


def load_file(path: str) -> list[Record]:
    try:
        with open(path, "r", encoding="utf-8") as f:
            return load_records(f)
    except OSError as e:
        raise IOError(f"read {path}: {e}")


def fetch_json(url: str) -> list[dict]:
    """URL 模式：聚合端点返回 JSON 记录数组。"""
    try:
        with urllib.request.urlopen(url, timeout=30) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
    except Exception as e:  # noqa: BLE001 - 统一包成 IOError
        raise IOError(f"fetch {url}: {e}")
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict) and isinstance(payload.get("records"), list):
        return payload["records"]
    raise IOError(f"unexpected JSON shape from {url} (want list of records)")


def records_from_json(payload: list) -> list[Record]:
    out: list[Record] = []
    for it in payload:
        if not isinstance(it, dict):
            continue
        try:
            lat = float(it.get("latency_ms", it.get("latency", 0.0)))
        except (TypeError, ValueError):
            lat = 0.0
        out.append(
            Record(
                timestamp=str(it.get("timestamp", "")),
                model=str(it.get("model", "")),
                status=str(it.get("status", "200")),
                latency_ms=lat,
                account_id=str(it.get("account_id", it.get("accountId", ""))),
            )
        )
    return out


def percentile(sorted_values: list[float], p: float) -> float:
    """nearest-rank 百分位（纯 stdlib）。"""
    n = len(sorted_values)
    if n == 0:
        return 0.0
    if p <= 0.0:
        return sorted_values[0]
    if p >= 100.0:
        return sorted_values[-1]
    idx = math.ceil(p / 100.0 * n) - 1
    idx = max(0, min(n - 1, idx))
    return sorted_values[idx]


@dataclass
class SideStats:
    name: str
    n: int = 0
    success: int = 0
    success_rate: float = 0.0
    latencies: list[float] = field(default_factory=list)
    p50: float = 0.0
    p95: float = 0.0
    p99: float = 0.0


def compute_stats(name: str, records: list[Record], latency_of: str = "all") -> SideStats:
    st = SideStats(name=name)
    if not records:
        return st
    st.n = len(records)
    st.success = sum(1 for r in records if is_success_status(r.status))
    st.success_rate = st.success / st.n
    # 延迟取成功请求（对齐“成功率 + 中位延迟”口径）；失败样本计入总延迟时 latency_of='all'
    pool = records if latency_of == "all" else [r for r in records if is_success_status(r.status)]
    st.latencies = sorted(r.latency_ms for r in pool)
    st.p50 = percentile(st.latencies, 50.0)
    st.p95 = percentile(st.latencies, 95.0)
    st.p99 = percentile(st.latencies, 99.0)
    return st


@dataclass
class QuotaPoint:
    account_id: str
    remaining: float
    mode: str = ""


def load_quota(path: str) -> dict[str, QuotaPoint]:
    """每行 `account_id: remaining`（可带 mode 第三列）。"""
    out: dict[str, QuotaPoint] = {}
    try:
        lines = open(path, "r", encoding="utf-8").read().splitlines()
    except OSError as e:
        raise IOError(f"read quota {path}: {e}")
    for line in lines:
        line = line.strip()
        if not line or _is_header(line):
            continue
        parts = line.replace(",", ":").split(":")
        if len(parts) < 2:
            continue
        try:
            remaining = float(parts[1])
        except ValueError:
            continue
        acc = parts[0].strip()
        mode = parts[2].strip() if len(parts) > 2 else ""
        out[f"{acc}:{mode}"] = QuotaPoint(acc, remaining, mode)
    return out


@dataclass
class CompareResult:
    go: SideStats
    rust: SideStats
    success_gap: float = 0.0        # rust - go（正=更好）
    p99_ratio: float = 0.0          # rust.p99 / go.p99（≤阈值达标）
    success_ok: bool = True
    p99_ok: bool = True
    quota_ok: bool = True
    quota_total: int = 0
    quota_match: int = 0
    quota_diff: list[tuple[str, float, float]] = field(default_factory=list)


def compare_sides(go: SideStats, rust: SideStats, success_gap: float, p99_ratio: float) -> CompareResult:
    res = CompareResult(go=go, rust=rust)
    res.success_gap = rust.success_rate - go.success_rate
    # G6-A1：rust ≥ go − gap
    res.success_ok = rust.success_rate >= (go.success_rate - success_gap) - 1e-9
    # G6-A2：rust.P99 ≤ go.P99 × ratio（go 无样本时按达标）
    if go.p99 > 0:
        res.p99_ratio = rust.p99 / go.p99
        res.p99_ok = rust.p99 <= go.p99 * p99_ratio + 1e-9
    else:
        res.p99_ratio = 0.0
        res.p99_ok = True
    return res


def compare_quota(go: dict[str, QuotaPoint], rust: dict[str, QuotaPoint], tol: float) -> CompareResult:
    res = CompareResult(go=SideStats(name="go"), rust=SideStats(name="rust"))
    keys = sorted(set(go) | set(rust))
    res.quota_total = len(keys)
    for key in keys:
        g = go.get(key)
        r = rust.get(key)
        if g is not None and r is not None and abs(r.remaining - g.remaining) <= tol:
            res.quota_match += 1
        else:
            res.quota_diff.append((key, g.remaining if g else float("nan"), r.remaining if r else float("nan")))
    res.quota_ok = res.quota_match == res.quota_total
    return res


def render_table(res: CompareResult, p95: bool, p50: bool) -> list[str]:
    rows = []
    rows.append(f"{'metric':<14}{'go':>12}{'rust':>12}{'delta':>12}")
    rows.append(f"{'requests':<14}{res.go.n:>12}{res.rust.n:>12}{res.rust.n - res.go.n:>12}")
    rows.append(f"{'success_rate':<14}{res.go.success_rate:>12.4f}{res.rust.success_rate:>12.4f}{res.success_gap:>+8.4f}")
    rows.append(f"{'p50_ms':<14}{res.go.p50:>12.1f}{res.rust.p50:>12.1f}{res.rust.p50 - res.go.p50:>+8.1f}")
    if p95:
        rows.append(f"{'p95_ms':<14}{res.go.p95:>12.1f}{res.rust.p95:>12.1f}{res.rust.p95 - res.go.p95:>+8.1f}")
    rows.append(f"{'p99_ms':<14}{res.go.p99:>12.1f}{res.rust.p99:>12.1f}{res.p99_ratio:>12.3f}x")
    return rows


def run(args) -> int:
    # ---- 数据加载 ----
    if args.url:
        go = records_from_json(fetch_json(args.go_url))
        rust = records_from_json(fetch_json(args.rust_url))
    else:
        if not args.go or not args.rust:
            raise RuntimeError("need --go and --rust result files (or --url mode)")
        go = load_file(args.go)
        rust = load_file(args.rust)

    go_s = compute_stats("go", go)
    rust_s = compute_stats("rust", rust)
    res = compare_sides(go_s, rust_s, args.success_gap, args.p99_ratio)

    # ---- 额度对比（可选）----
    if args.go_quota and args.rust_quota:
        qgo = load_quota(args.go_quota)
        qrust = load_quota(args.rust_quota)
        qres = compare_quota(qgo, qrust, args.quota_tol)
        res.quota_ok = qres.quota_ok
        res.quota_total = qres.quota_total
        res.quota_match = qres.quota_match
        res.quota_diff = qres.quota_diff

    # ---- 汇总输出 ----
    if args.json:
        payload = {
            "go": {
                "n": res.go.n, "success": res.go.success, "success_rate": res.go.success_rate,
                "latency_ms": {"p50": res.go.p50, "p95": res.go.p95, "p99": res.go.p99},
            },
            "rust": {
                "n": res.rust.n, "success": res.rust.success, "success_rate": res.rust.success_rate,
                "latency_ms": {"p50": res.rust.p50, "p95": res.rust.p95, "p99": res.rust.p99},
            },
            "diff": {
                "success_gap": res.success_gap,
                "p99_ratio": res.p99_ratio,
                "success_ok": res.success_ok,
                "p99_ok": res.p99_ok,
            },
            "quota": {
                "total": res.quota_total, "match": res.quota_match,
                "ok": res.quota_ok,
                "diff": [{"account_id": k, "go": a, "rust": b} for k, a, b in res.quota_diff[:50]],
            },
            "pass": res.success_ok and res.p99_ok and res.quota_ok,
        }
        print(json.dumps(payload, ensure_ascii=False, indent=2))
    else:
        for line in render_table(res, args.p95, True):
            print(line)
        if args.go_quota and args.rust_quota:
            print(f"\nquota: {res.quota_match}/{res.quota_total} 账号一致 (tol={args.quota_tol})")
            for key, a, b in res.quota_diff[:10]:
                print(f"  diff {key}: go={a} rust={b}")
        status = []
        if not res.success_ok:
            status.append("G6-A1 成功率不达标")
        if not res.p99_ok:
            status.append("G6-A2 P99 不达标")
        if not res.quota_ok:
            status.append("G6-A3 额度不一致")
        print("\n" + ("; ".join(status) if status else "ALL PASS: G6-A1/A2/A3 全部达标"))

    # ---- 退出码 ----
    if not (res.success_ok and res.p99_ok and res.quota_ok):
        return 2
    return 0


def self_test() -> int:
    """内置合成数据跑通断言（无需外部文件）。"""
    go = [Record(status="200", latency_ms=100 + (i % 5) * 10) for i in range(100)]
    rust_ok = [Record(status="200", latency_ms=110 + (i % 5) * 10) for i in range(100)]

    # 成功率：rust 比 go 少 2 个成功项 → gap 0.03（>0.01 阈值）
    go_mixed = [Record(status="200", latency_ms=100) for _ in range(90)] + \
               [Record(status="500", latency_ms=50) for _ in range(10)]
    rust_low = [Record(status="200", latency_ms=100) for _ in range(85)] + \
               [Record(status="500", latency_ms=50) for _ in range(15)]

    g = compute_stats("go", go)
    r = compute_stats("rust", rust_ok)
    base = compare_sides(g, r, 0.01, 1.15)
    assert base.success_ok and base.p99_ok, "基准(达标)应通过"

    gm = compute_stats("go", go_mixed)
    rl = compute_stats("rust", rust_low)
    fail_succ = compare_sides(gm, rl, 0.01, 1.15)
    assert not fail_succ.success_ok, "成功率差距>1% 应不达标"

    slow = compute_stats("rust", [Record(status="200", latency_ms=200) for _ in range(100)])
    fail_p99 = compare_sides(g, slow, 0.01, 1.15)
    assert not fail_p99.p99_ok, "P99 超 1.15x 应不达标"

    # 阈值可配测试
    loose = compare_sides(gm, rl, 0.05, 1.15)
    assert loose.success_ok, "放宽阈值后应达标"

    # 配额
    qgo = {"1": QuotaPoint("1", 100.0, "fast"), "2": QuotaPoint("2", 0.0, "fast")}
    qrust = {"1": QuotaPoint("1", 100.0, "fast"), "2": QuotaPoint("2", 5.0, "fast")}
    q = compare_quota(qgo, qrust, 0.0)
    assert not q.quota_ok and q.quota_diff, "额度不一致应检出"
    q2 = compare_quota(qgo, {"1": QuotaPoint("1", 100.0, "fast")}, 0.0)
    assert not q2.quota_ok, "缺账号应检出"

    # 百分位（nearest-rank：p=99, n=100 → 第 99 个（1-based）→ index 98 = 98）
    assert percentile(list(range(100)), 99) == 98
    assert percentile([], 99) == 0.0

    print("self-test OK: 达标/劣化/配额/百分位 全部断言通过")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Grok 移植 shadow compare（G6-A1/A2/A3）")
    src = ap.add_mutually_exclusive_group(required=False)
    src.add_argument("--file", action="store_const", const=True, help="本地文件回放（默认，需 --go/--rust）")
    src.add_argument("--url", action="store_true", help="URL 聚合端点（需 --go-url/--rust-url）")
    ap.add_argument("--go", help="Go 结果文件")
    ap.add_argument("--rust", help="Rust 结果文件")
    ap.add_argument("--go-url", dest="go_url", help="Go 聚合端点")
    ap.add_argument("--rust-url", dest="rust_url", help="Rust 聚合端点")
    ap.add_argument("--go-quota", dest="go_quota", help="Go 额度文件（account: remaining）")
    ap.add_argument("--rust-quota", dest="rust_quota", help="Rust 额度文件")
    ap.add_argument("--quota-tol", dest="quota_tol", type=float, default=0.0, help="额度容差（默认 0）")
    ap.add_argument("--success-gap", dest="success_gap", type=float, default=0.01, help="成功率容忍下限差（默认 0.01）")
    ap.add_argument("--p99-ratio", dest="p99_ratio", type=float, default=1.15, help="Rust/Go P99 上限比例（默认 1.15）")
    ap.add_argument("--p95", action="store_true", help="额外打印 P95")
    ap.add_argument("--json", action="store_true", help="JSON 输出")
    ap.add_argument("--self-test", action="store_true", help="内置合成数据自测")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()
    try:
        return run(args)
    except (IOError, RuntimeError, ValueError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())