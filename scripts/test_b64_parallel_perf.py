#!/usr/bin/env python3
"""10-way parallel b64 image API benchmark with CPU/memory time-series + plots."""
from __future__ import annotations

import argparse
import base64
import json
import re
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_http_headers import request_headers


@dataclass
class Sample:
    t: float  # seconds since run start
    cpu_pct: float
    mem_mb: float
    mem_limit_mb: float | None
    host_avail_mb: float | None
    proc_rss_mb: float | None


@dataclass
class ProcSnap:
    rss_kb: int
    threads: int
    voluntary_ctx: int
    nonvoluntary_ctx: int
    utime: int
    stime: int


def _parse_mb(s: str) -> float:
    s = s.strip().upper().replace(" ", "")
    if not s:
        return 0.0
    for suffix, mult in (
        ("GIB", 1024),
        ("GB", 1024),
        ("MIB", 1),
        ("MB", 1),
        ("KIB", 1 / 1024),
        ("KB", 1 / 1024),
        ("B", 1 / (1024 * 1024)),
    ):
        if s.endswith(suffix):
            return float(s[:-len(suffix)]) * mult
    return float(re.split(r"[A-Z]", s)[0])


def host_mem_avail_mb() -> float | None:
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) / 1024
    except OSError:
        return None
    return None


def docker_main_pid(container: str) -> int | None:
    try:
        out = subprocess.check_output(
            ["docker", "top", container, "-eo", "pid,comm,rss"],
            text=True,
        )
        rows = out.strip().splitlines()[1:]
        best_pid, best_rss = None, -1
        for row in rows:
            cols = row.split(None, 2)
            if len(cols) < 3:
                continue
            pid, comm, rss = int(cols[0]), cols[1], int(cols[2])
            if comm in ("uvicorn", "gateway", "tnexus-gateway"):
                return pid
            if rss > best_rss:
                best_rss, best_pid = rss, pid
        return best_pid
    except (subprocess.CalledProcessError, ValueError, IndexError):
        try:
            out = subprocess.check_output(
                ["docker", "inspect", "-f", "{{.State.Pid}}", container],
                text=True,
            ).strip()
            return int(out) if out and out != "0" else None
        except (subprocess.CalledProcessError, ValueError):
            return None


def read_proc_snap(pid: int) -> ProcSnap | None:
    try:
        status = open(f"/proc/{pid}/status").read().splitlines()
        stat = open(f"/proc/{pid}/stat").read().split()
        kv = {}
        for line in status:
            if ":" in line:
                k, v = line.split(":", 1)
                kv[k.strip()] = v.strip()
        return ProcSnap(
            rss_kb=int(kv.get("VmRSS", "0 kB").split()[0]),
            threads=int(kv.get("Threads", "0")),
            voluntary_ctx=int(kv.get("voluntary_ctxt_switches", "0")),
            nonvoluntary_ctx=int(kv.get("nonvoluntary_ctxt_switches", "0")),
            utime=int(stat[13]),
            stime=int(stat[14]),
        )
    except (OSError, IndexError, ValueError):
        return None


def proc_rss_mb(pid: int | None) -> float | None:
    if not pid:
        return None
    snap = read_proc_snap(pid)
    return snap.rss_kb / 1024 if snap else None


def parse_stats_line(line: str) -> tuple[float, float, float | None] | None:
    parts = line.strip().split("\t")
    if len(parts) < 3:
        return None
    cpu = float(parts[1].replace("%", "").strip() or 0)
    mem_part = parts[2].split("/")
    mem_mb = _parse_mb(mem_part[0])
    mem_lim = _parse_mb(mem_part[1]) if len(mem_part) > 1 else None
    return cpu, mem_mb, mem_lim


def percentile(vals: list[float], pct: float) -> float:
    if not vals:
        return 0.0
    xs = sorted(vals)
    if len(xs) == 1:
        return xs[0]
    idx = pct * (len(xs) - 1)
    lo, hi = int(idx), min(int(idx) + 1, len(xs) - 1)
    frac = idx - lo
    return xs[lo] + frac * (xs[hi] - xs[lo])


def post_b64(base: str, auth: str, slot: int, prompt: str, timeout: float) -> dict:
    body = json.dumps(
        {
            "model": "gpt-image-2",
            "prompt": f"{prompt} [parallel-b64:{slot}]",
            "n": 1,
            "size": "1024x1024",
            "response_format": "b64_json",
        }
    ).encode()
    req = urllib.request.Request(
        f"{base.rstrip('/')}/v1/images/generations",
        data=body,
        headers=request_headers(
            {
                "Content-Type": "application/json",
                "Authorization": f"Bearer {auth}",
            }
        ),
        method="POST",
    )
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
        wall = time.time() - t0
        item = (data.get("data") or [{}])[0]
        b64 = item.get("b64_json") or ""
        raw = base64.b64decode(b64) if b64 else b""
        pipe = data.get("_tnexus_pipeline") or {}
        timings = pipe.get("timings_ms") or {}
        return {
            "ok": True,
            "slot": slot,
            "start_offset_s": None,
            "wall_s": round(wall, 3),
            "end_offset_s": None,
            "bytes": len(raw),
            "email": pipe.get("account_email"),
            "gateway_wall_ms": timings.get("gateway_wall_ms"),
            "upstream_wall_ms": timings.get("upstream_wall_ms"),
        }
    except urllib.error.HTTPError as e:
        wall = time.time() - t0
        err = e.read().decode("utf-8", errors="replace")[:500]
        return {
            "ok": False,
            "slot": slot,
            "wall_s": round(wall, 3),
            "error": f"HTTP {e.code}: {err}",
        }
    except Exception as e:
        wall = time.time() - t0
        return {"ok": False, "slot": slot, "wall_s": round(wall, 3), "error": str(e)}


def sampler_loop(
    container: str,
    pid: int | None,
    run_start: float,
    out: list[Sample],
    stop: threading.Event,
    interval: float,
):
    while not stop.is_set():
        t = time.time() - run_start
        cpu, mem, lim = 0.0, 0.0, None
        try:
            raw = subprocess.check_output(
                [
                    "docker",
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}",
                    container,
                ],
                text=True,
                timeout=15,
            )
            for line in raw.splitlines():
                if line.startswith(container):
                    parsed = parse_stats_line(line)
                    if parsed:
                        cpu, mem, lim = parsed
                    break
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            pass
        out.append(
            Sample(
                t=round(t, 2),
                cpu_pct=cpu,
                mem_mb=mem,
                mem_limit_mb=lim,
                host_avail_mb=host_mem_avail_mb(),
                proc_rss_mb=proc_rss_mb(pid),
            )
        )
        stop.wait(interval)


def run_parallel(
    label: str,
    base: str,
    auth: str,
    container: str,
    n: int,
    prompt: str,
    timeout: float,
    sample_interval: float,
) -> dict:
    print(f"\n=== {label} parallel x{n} ({base}) ===")
    pid = docker_main_pid(container)
    proc_before = read_proc_snap(pid) if pid else None
    samples: list[Sample] = []
    stop = threading.Event()
    run_start = time.time()
    th = threading.Thread(
        target=sampler_loop,
        args=(container, pid, run_start, samples, stop, sample_interval),
        daemon=True,
    )
    th.start()
    time.sleep(0.3)

    results: list[dict] = []
    batch_t0 = time.time()
    with ThreadPoolExecutor(max_workers=n) as pool:
        futs = {
            pool.submit(post_b64, base, auth, i, prompt, timeout): i
            for i in range(n)
        }
        for fut in as_completed(futs):
            r = fut.result()
            r["end_offset_s"] = round(time.time() - run_start, 3)
            r["start_offset_s"] = round(r["end_offset_s"] - r["wall_s"], 3)
            results.append(r)
            slot = r.get("slot")
            if r.get("ok"):
                print(
                    f"  slot {slot}: OK wall={r['wall_s']}s bytes={r['bytes']} "
                    f"email={r.get('email')}"
                )
            else:
                print(f"  slot {slot}: FAIL {r.get('error', '')[:120]}")

    batch_wall = time.time() - batch_t0
    stop.set()
    th.join(timeout=10)
    proc_after = read_proc_snap(pid) if pid else None

    ok_walls = [r["wall_s"] for r in results if r.get("ok")]
    pct = {
        "p50": round(percentile(ok_walls, 0.50), 2),
        "p95": round(percentile(ok_walls, 0.95), 2),
        "p99": round(percentile(ok_walls, 0.99), 2),
        "min": round(min(ok_walls), 2) if ok_walls else 0,
        "max": round(max(ok_walls), 2) if ok_walls else 0,
        "samples": len(ok_walls),
    }
    hz = 100
    cpu_delta = None
    if proc_before and proc_after:
        cpu_delta = round(
            (proc_after.utime + proc_after.stime - proc_before.utime - proc_before.stime) / hz,
            3,
        )

    print(
        f"  batch_wall={batch_wall:.1f}s ok={pct['samples']}/{n} "
        f"p50={pct['p50']}s p95={pct['p95']}s p99={pct['p99']}s "
        f"cpu_delta={cpu_delta}s"
    )
    if samples:
        cpus = [s.cpu_pct for s in samples]
        mems = [s.mem_mb for s in samples]
        print(
            f"  docker cpu avg={sum(cpus)/len(cpus):.1f}% max={max(cpus):.1f}% "
            f"mem avg={sum(mems)/len(mems):.0f}MB max={max(mems):.0f}MB "
            f"samples={len(samples)}"
        )

    return {
        "label": label,
        "base": base,
        "container": container,
        "n": n,
        "batch_wall_s": round(batch_wall, 2),
        "percentiles_s": pct,
        "requests": sorted(results, key=lambda x: x.get("slot", 0)),
        "timeseries": [asdict(s) for s in samples],
        "proc_before": asdict(proc_before) if proc_before else None,
        "proc_after": asdict(proc_after) if proc_after else None,
        "proc_cpu_delta_s": cpu_delta,
    }


def plot_results(runs: list[dict], outdir: Path):
    try:
        import matplotlib.pyplot as plt
        import matplotlib.ticker as ticker
    except ImportError:
        print("matplotlib not installed — skip plots (install: pip3 install matplotlib)")
        return

    outdir.mkdir(parents=True, exist_ok=True)
    colors = {"gptimage-8012": "#e45756", "tnexus-8014": "#4c78a8"}

    # 1) CPU time series
    fig, ax = plt.subplots(figsize=(12, 5))
    for run in runs:
        ts = run["timeseries"]
        if not ts:
            continue
        label = run["label"]
        ax.plot(
            [p["t"] for p in ts],
            [p["cpu_pct"] for p in ts],
            label=label,
            color=colors.get(label, None),
            linewidth=1.8,
            alpha=0.9,
        )
    ax.set_xlabel("Time since batch start (s)")
    ax.set_ylabel("Container CPU %")
    ax.set_title("10-way parallel b64 — CPU usage")
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.xaxis.set_major_locator(ticker.MaxNLocator(12))
    fig.tight_layout()
    fig.savefig(outdir / "cpu_timeseries.png", dpi=150)
    plt.close(fig)

    # 2) Memory time series (docker + proc rss)
    fig, axes = plt.subplots(2, 1, figsize=(12, 8), sharex=True)
    for run in runs:
        ts = run["timeseries"]
        if not ts:
            continue
        label = run["label"]
        c = colors.get(label, None)
        axes[0].plot(
            [p["t"] for p in ts],
            [p["mem_mb"] for p in ts],
            label=f"{label} docker",
            color=c,
            linewidth=1.8,
        )
        proc = [p["proc_rss_mb"] for p in ts if p.get("proc_rss_mb")]
        if proc:
            axes[1].plot(
                [p["t"] for p in ts if p.get("proc_rss_mb")],
                proc,
                label=f"{label} main proc",
                color=c,
                linewidth=1.8,
                linestyle="--",
            )
    axes[0].set_ylabel("Docker MEM (MiB)")
    axes[0].set_title("10-way parallel b64 — memory")
    axes[0].legend()
    axes[0].grid(True, alpha=0.3)
    axes[1].set_xlabel("Time since batch start (s)")
    axes[1].set_ylabel("Main process RSS (MiB)")
    axes[1].legend()
    axes[1].grid(True, alpha=0.3)
    fig.tight_layout()
    fig.savefig(outdir / "mem_timeseries.png", dpi=150)
    plt.close(fig)

    # 3) Host available memory
    fig, ax = plt.subplots(figsize=(12, 4))
    for run in runs:
        ts = run["timeseries"]
        avail = [(p["t"], p["host_avail_mb"]) for p in ts if p.get("host_avail_mb")]
        if avail:
            ax.plot(
                [a[0] for a in avail],
                [a[1] for a in avail],
                label=run["label"],
                color=colors.get(run["label"]),
                linewidth=1.8,
            )
    ax.set_xlabel("Time since batch start (s)")
    ax.set_ylabel("Host MemAvailable (MiB)")
    ax.set_title("Host memory pressure during parallel b64")
    ax.legend()
    ax.grid(True, alpha=0.3)
    fig.tight_layout()
    fig.savefig(outdir / "host_mem_timeseries.png", dpi=150)
    plt.close(fig)

    # 4) Per-slot latency Gantt-style
    fig, axes = plt.subplots(1, 2, figsize=(14, 6), sharey=True)
    for ax, run in zip(axes, runs):
        reqs = [r for r in run["requests"] if r.get("ok")]
        for r in reqs:
            slot = r.get("slot", 0)
            start = r.get("start_offset_s", 0)
            wall = r.get("wall_s", 0)
            ax.barh(
                slot,
                wall,
                left=start,
                height=0.6,
                color=colors.get(run["label"]),
                alpha=0.75,
                edgecolor="white",
            )
        ax.set_title(run["label"])
        ax.set_xlabel("Time (s)")
        ax.set_ylabel("Slot index")
        ax.grid(True, axis="x", alpha=0.3)
    fig.suptitle("Per-request wall time (parallel overlap)")
    fig.tight_layout()
    fig.savefig(outdir / "latency_gantt.png", dpi=150)
    plt.close(fig)

    # 5) Latency distribution + percentiles
    fig, axes = plt.subplots(1, 2, figsize=(14, 5))
    for ax, run in zip(axes, runs):
        walls = [r["wall_s"] for r in run["requests"] if r.get("ok")]
        if walls:
            ax.hist(walls, bins=min(10, len(walls)), color=colors.get(run["label"]), alpha=0.7, edgecolor="white")
        pct = run["percentiles_s"]
        for name, color, ls in [("p50", "black", "-"), ("p95", "orange", "--"), ("p99", "red", ":")]:
            v = pct.get(name)
            if v:
                ax.axvline(v, color=color, linestyle=ls, linewidth=2, label=f"{name}={v}s")
        ax.set_title(run["label"])
        ax.set_xlabel("Wall time (s)")
        ax.set_ylabel("Count")
        ax.legend(fontsize=8)
        ax.grid(True, alpha=0.3)
    fig.suptitle("Latency distribution (successful slots)")
    fig.tight_layout()
    fig.savefig(outdir / "latency_hist.png", dpi=150)
    plt.close(fig)

    # 6) Summary comparison bars
    fig, ax = plt.subplots(figsize=(10, 5))
    metrics = ["p50", "p95", "p99", "max"]
    x = range(len(metrics))
    w = 0.35
    for i, run in enumerate(runs):
        vals = [run["percentiles_s"].get(m, 0) for m in metrics]
        offset = -w/2 if i == 0 else w/2
        bars = ax.bar(
            [xi + offset for xi in x],
            vals,
            width=w,
            label=run["label"],
            color=colors.get(run["label"]),
            alpha=0.85,
        )
        for bar, val in zip(bars, vals):
            ax.text(bar.get_x() + bar.get_width()/2, bar.get_height() + 1, f"{val:.1f}s", ha="center", fontsize=9)
    ax.set_xticks(list(x))
    ax.set_xticklabels(metrics)
    ax.set_ylabel("Seconds")
    ax.set_title("Latency percentiles — 10 parallel b64")
    ax.legend()
    ax.grid(True, axis="y", alpha=0.3)
    fig.tight_layout()
    fig.savefig(outdir / "percentiles_summary.png", dpi=150)
    plt.close(fig)

    # 7) Combined dashboard
    fig = plt.figure(figsize=(14, 10))
    gs = fig.add_gridspec(2, 2, hspace=0.35, wspace=0.25)
    ax_cpu = fig.add_subplot(gs[0, 0])
    ax_mem = fig.add_subplot(gs[0, 1])
    ax_pct = fig.add_subplot(gs[1, 0])
    ax_tbl = fig.add_subplot(gs[1, 1])
    for run in runs:
        ts = run["timeseries"]
        lab = run["label"]
        c = colors.get(lab)
        if ts:
            ax_cpu.plot([p["t"] for p in ts], [p["cpu_pct"] for p in ts], label=lab, color=c)
            ax_mem.plot([p["t"] for p in ts], [p["mem_mb"] for p in ts], label=lab, color=c)
    ax_cpu.set_title("CPU %")
    ax_cpu.set_xlabel("s")
    ax_cpu.legend(fontsize=8)
    ax_cpu.grid(True, alpha=0.3)
    ax_mem.set_title("Docker MEM MiB")
    ax_mem.set_xlabel("s")
    ax_mem.legend(fontsize=8)
    ax_mem.grid(True, alpha=0.3)

    labels = [r["label"] for r in runs]
    p50s = [r["percentiles_s"]["p50"] for r in runs]
    p95s = [r["percentiles_s"]["p95"] for r in runs]
    p99s = [r["percentiles_s"]["p99"] for r in runs]
    xi = range(len(labels))
    ax_pct.bar([i - 0.2 for i in xi], p50s, 0.2, label="p50", color="#72b7b2")
    ax_pct.bar(xi, p95s, 0.2, label="p95", color="#eeca3b")
    ax_pct.bar([i + 0.2 for i in xi], p99s, 0.2, label="p99", color="#f58518")
    ax_pct.set_xticks(list(xi))
    ax_pct.set_xticklabels(labels, rotation=15, ha="right")
    ax_pct.set_ylabel("s")
    ax_pct.set_title("Latency percentiles")
    ax_pct.legend(fontsize=8)
    ax_pct.grid(True, axis="y", alpha=0.3)

    ax_tbl.axis("off")
    rows = []
    for run in runs:
        p = run["percentiles_s"]
        ts = run["timeseries"]
        cpu_avg = sum(s["cpu_pct"] for s in ts) / len(ts) if ts else 0
        mem_max = max(s["mem_mb"] for s in ts) if ts else 0
        rows.append([
            run["label"],
            f"{run['batch_wall_s']}s",
            f"{p['samples']}/{run['n']}",
            f"{p['p50']}/{p['p95']}/{p['p99']}",
            f"{cpu_avg:.1f}%",
            f"{mem_max:.0f}MB",
            f"{run.get('proc_cpu_delta_s')}s",
        ])
    table = ax_tbl.table(
        cellText=rows,
        colLabels=["chain", "batch_wall", "ok", "p50/p95/p99", "cpu_avg", "mem_max", "proc_cpu"],
        loc="center",
        cellLoc="center",
    )
    table.auto_set_font_size(False)
    table.set_fontsize(9)
    table.scale(1, 1.6)
    ax_tbl.set_title("Summary table")
    fig.suptitle("10-way parallel b64 — gptimage :8012 vs TNexus :8014", fontsize=14, y=0.98)
    fig.savefig(outdir / "dashboard.png", dpi=150, bbox_inches="tight")
    plt.close(fig)

    print(f"plots written to {outdir}/")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gptimage", default="http://127.0.0.1:8012")
    ap.add_argument("--tnexus", default="http://127.0.0.1:8014")
    ap.add_argument("--container-8012", default="chatgpt2api-local")
    ap.add_argument("--container-8014", default="panda-gateway-1")
    ap.add_argument("--auth-8012", required=True)
    ap.add_argument("--auth-8014", required=True)
    ap.add_argument("-n", type=int, default=10)
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument("--sample-interval", type=float, default=1.0)
    ap.add_argument("--cooldown", type=float, default=30.0)
    ap.add_argument("--outdir", default="/tmp/b64_parallel_perf")
    ap.add_argument("--skip-8012", action="store_true")
    ap.add_argument("--skip-8014", action="store_true")
    ap.add_argument(
        "--prompt",
        default="a yellow pyramid on white background, studio product photo",
    )
    args = ap.parse_args()

    outdir = Path(args.outdir)
    outdir.mkdir(parents=True, exist_ok=True)

    print("=== host ===")
    try:
        ncpu = open("/proc/cpuinfo").read().count("processor\t:")
        print(f"  vCPU: {ncpu}")
    except OSError:
        pass
    avail = host_mem_avail_mb()
    print(f"  MemAvailable: {avail:.0f} MiB" if avail else "  MemAvailable: ?")

    runs: list[dict] = []
    if not args.skip_8012:
        runs.append(
            run_parallel(
                "gptimage-8012",
                args.gptimage,
                args.auth_8012,
                args.container_8012,
                args.n,
                args.prompt,
                args.timeout,
                args.sample_interval,
            )
        )
        print(f"  cooldown {args.cooldown}s...")
        time.sleep(args.cooldown)

    if not args.skip_8014:
        runs.append(
            run_parallel(
                "tnexus-8014",
                args.tnexus,
                args.auth_8014,
                args.container_8014,
                args.n,
                args.prompt,
                args.timeout,
                args.sample_interval,
            )
        )

    report = {
        "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "n": args.n,
        "runs": runs,
    }
    json_path = outdir / "results.json"
    json_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"\nJSON: {json_path}")

    plot_results(runs, outdir)

    ok_all = all(r["percentiles_s"]["samples"] == args.n for r in runs)
    print(f"B64_PARALLEL_PERF rc={0 if ok_all else 1}")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
