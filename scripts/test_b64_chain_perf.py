#!/usr/bin/env python3
"""Serial b64 compare + container CPU/memory sampling (:8012 vs :8014)."""
import argparse
import base64
import json
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_http_headers import request_headers


@dataclass
class Sample:
    ts: float
    cpu_pct: float
    mem_mb: float
    mem_limit_mb: float | None
    net_rx_mb: float
    net_tx_mb: float
    block_read_mb: float
    block_write_mb: float


@dataclass
class ProcSnap:
    rss_kb: int
    threads: int
    voluntary_ctx: int
    nonvoluntary_ctx: int
    utime: int
    stime: int


@dataclass
class ChainPerf:
    label: str
    container: str
    requests: list[dict] = field(default_factory=list)
    samples: list[Sample] = field(default_factory=list)
    proc_before: ProcSnap | None = None
    proc_after: ProcSnap | None = None


def png_dims(data: bytes) -> tuple[int, int] | None:
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


def docker_main_pid(container: str) -> int | None:
    """Best-effort main workload PID (uvicorn for gptimage, container PID otherwise)."""
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
        return docker_pid(container)


def read_proc_snap(pid: int) -> ProcSnap | None:
    try:
        status = open(f"/proc/{pid}/status").read().splitlines()
        stat = open(f"/proc/{pid}/stat").read().split()
        kv = {}
        for line in status:
            if ":" in line:
                k, v = line.split(":", 1)
                kv[k.strip()] = v.strip()
        threads = int(kv.get("Threads", "0"))
        rss_kb = int(kv.get("VmRSS", "0 kB").split()[0])
        voluntary = int(kv.get("voluntary_ctxt_switches", "0"))
        nonvoluntary = int(kv.get("nonvoluntary_ctxt_switches", "0"))
        utime = int(stat[13])
        stime = int(stat[14])
        return ProcSnap(rss_kb, threads, voluntary, nonvoluntary, utime, stime)
    except OSError:
        return None


def parse_docker_stats_line(line: str) -> tuple[float, float, float | None, float, float, float, float] | None:
    parts = line.strip().split("\t")
    if len(parts) < 3:
        return None
    cpu_pct = float(parts[1].replace("%", "").strip() or 0)
    mem_part = parts[2].split("/")
    mem_mb = _parse_mb(mem_part[0])
    mem_limit = _parse_mb(mem_part[1]) if len(mem_part) > 1 else None
    net_rx, net_tx, blk_r, blk_w = 0.0, 0.0, 0.0, 0.0
    if len(parts) > 3 and parts[3]:
        net = parts[3].split("/")
        net_rx = _parse_mb(net[0]) if net else 0.0
        net_tx = _parse_mb(net[1]) if len(net) > 1 else 0.0
    if len(parts) > 4 and parts[4]:
        blk = parts[4].split("/")
        blk_r = _parse_mb(blk[0]) if blk else 0.0
        blk_w = _parse_mb(blk[1]) if len(blk) > 1 else 0.0
    return cpu_pct, mem_mb, mem_limit, net_rx, net_tx, blk_r, blk_w


def _parse_mb(s: str) -> float:
    s = s.strip().upper()
    if not s:
        return 0.0
    if s.endswith("GIB"):
        return float(s[:-3].strip()) * 1024
    if s.endswith("MIB"):
        return float(s[:-3].strip())
    if s.endswith("KIB"):
        return float(s[:-3].strip()) / 1024
    if s.endswith("B"):
        return float(s[:-1].strip()) / (1024 * 1024)
    return float(s.split()[0])


def sampler_loop(container: str, out: list[Sample], stop: threading.Event, interval: float):
    while not stop.is_set():
        try:
            raw = subprocess.check_output(
                [
                    "docker",
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}\t{{.BlockIO}}",
                    container,
                ],
                text=True,
                timeout=10,
            )
            for line in raw.splitlines():
                if not line.startswith(container):
                    continue
                parsed = parse_docker_stats_line(line)
                if not parsed:
                    continue
                cpu, mem, lim, nr, nt, br, bw = parsed
                out.append(
                    Sample(time.time(), cpu, mem, lim, nr, nt, br, bw)
                )
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            pass
        stop.wait(interval)


def post_b64(base: str, auth: str, prompt: str, timeout: float) -> dict:
    body = json.dumps(
        {
            "model": "gpt-image-2",
            "prompt": prompt,
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
        dims = png_dims(raw)
        return {
            "ok": True,
            "wall_s": round(wall, 2),
            "bytes": len(raw),
            "dims": f"{dims[0]}x{dims[1]}" if dims else None,
            "email": pipe.get("account_email"),
            "gateway_wall_ms": timings.get("gateway_wall_ms"),
            "upstream_wall_ms": timings.get("upstream_wall_ms"),
        }
    except urllib.error.HTTPError as e:
        wall = time.time() - t0
        err = e.read().decode("utf-8", errors="replace")[:400]
        return {"ok": False, "wall_s": round(wall, 2), "error": f"HTTP {e.code}: {err}"}
    except Exception as e:
        wall = time.time() - t0
        return {"ok": False, "wall_s": round(wall, 2), "error": str(e)}


def summarize_samples(samples: list[Sample]) -> dict:
    if not samples:
        return {}
    cpus = [s.cpu_pct for s in samples]
    mems = [s.mem_mb for s in samples]
    return {
        "samples": len(samples),
        "cpu_pct_avg": round(sum(cpus) / len(cpus), 2),
        "cpu_pct_max": round(max(cpus), 2),
        "mem_mb_avg": round(sum(mems) / len(mems), 1),
        "mem_mb_max": round(max(mems), 1),
        "mem_mb_min": round(min(mems), 1),
    }


def summarize_proc(before: ProcSnap | None, after: ProcSnap | None) -> dict:
    if not before or not after:
        return {}
    hz = 100  # linux USER_HZ typically 100 on x86
    cpu_s = (after.utime + after.stime - before.utime - before.stime) / hz
    return {
        "rss_kb_before": before.rss_kb,
        "rss_kb_after": after.rss_kb,
        "rss_delta_kb": after.rss_kb - before.rss_kb,
        "threads_before": before.threads,
        "threads_after": after.threads,
        "cpu_seconds_delta": round(cpu_s, 3),
        "voluntary_ctx_delta": after.voluntary_ctx - before.voluntary_ctx,
        "nonvoluntary_ctx_delta": after.nonvoluntary_ctx - before.nonvoluntary_ctx,
    }


def run_chain(
    perf: ChainPerf,
    base: str,
    auth: str,
    n: int,
    prompt: str,
    timeout: float,
    sample_interval: float,
) -> ChainPerf:
    print(f"\n=== {perf.label} ({base}) container={perf.container} serial x{n} ===")
    pid = docker_main_pid(perf.container)
    perf.proc_before = read_proc_snap(pid) if pid else None

    stop = threading.Event()
    samples: list[Sample] = []
    th = threading.Thread(
        target=sampler_loop,
        args=(perf.container, samples, stop, sample_interval),
        daemon=True,
    )
    th.start()
    time.sleep(0.5)

    for i in range(n):
        p = f"{prompt} [perf:{perf.label}:{i}]"
        r = post_b64(base, auth, p, timeout)
        perf.requests.append(r)
        if r.get("ok"):
            print(
                f"  [{i + 1}] OK wall={r['wall_s']}s bytes={r['bytes']} "
                f"dims={r.get('dims')} email={r.get('email')}"
            )
        else:
            print(f"  [{i + 1}] FAIL {r.get('error', '')[:140]}")

    stop.set()
    th.join(timeout=5)
    perf.samples = samples
    perf.proc_after = read_proc_snap(pid) if pid else None

    ok = sum(1 for r in perf.requests if r.get("ok"))
    walls = [r["wall_s"] for r in perf.requests if r.get("ok")]
    avg_wall = sum(walls) / len(walls) if walls else 0
    proc = summarize_proc(perf.proc_before, perf.proc_after)
    samp = summarize_samples(perf.samples)

    print(f"  latency: {ok}/{n} ok  wall_avg={avg_wall:.1f}s")
    if samp:
        print(
            f"  docker: cpu_avg={samp['cpu_pct_avg']}% cpu_max={samp['cpu_pct_max']}% "
            f"mem_avg={samp['mem_mb_avg']}MB mem_max={samp['mem_mb_max']}MB "
            f"(samples={samp['samples']})"
        )
    if proc:
        print(
            f"  proc: rss {proc['rss_kb_before']}→{proc['rss_kb_after']}KB "
            f"(Δ{proc['rss_delta_kb']}KB) threads {proc['threads_before']}→{proc['threads_after']} "
            f"cpu_delta={proc['cpu_seconds_delta']}s "
            f"ctx_vol+{proc['voluntary_ctx_delta']} nonvol+{proc['nonvoluntary_ctx_delta']}"
        )
        if ok and proc["cpu_seconds_delta"] > 0:
            print(f"  cpu_per_ok_req={proc['cpu_seconds_delta']/ok:.3f}s (gateway CPU only, upstream waits)")
    return perf


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gptimage", default="http://127.0.0.1:8012")
    ap.add_argument("--tnexus", default="http://127.0.0.1:8014")
    ap.add_argument("--container-8012", default="chatgpt2api-local")
    ap.add_argument("--container-8014", default="panda-gateway-1")
    ap.add_argument("--auth-8012", required=True)
    ap.add_argument("--auth-8014", required=True)
    ap.add_argument("-n", type=int, default=2)
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument("--sample-interval", type=float, default=1.0)
    ap.add_argument(
        "--prompt",
        default="a green cylinder on white background, studio product photo",
    )
    args = ap.parse_args()

    # idle baseline
    print("=== idle baseline (docker stats) ===")
    for name in (args.container_8012, args.container_8014):
        try:
            raw = subprocess.check_output(
                [
                    "docker",
                    "stats",
                    "--no-stream",
                    "--format",
                    "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}\t{{.BlockIO}}",
                    name,
                ],
                text=True,
            )
            print(f"  {raw.strip()}")
        except subprocess.CalledProcessError:
            print(f"  {name}: unavailable")

    p8012 = run_chain(
        ChainPerf("gptimage-8012", args.container_8012),
        args.gptimage,
        args.auth_8012,
        args.n,
        args.prompt,
        args.timeout,
        args.sample_interval,
    )
    time.sleep(3)  # cool-down between chains
    p8014 = run_chain(
        ChainPerf("tnexus-8014", args.container_8014),
        args.tnexus,
        args.auth_8014,
        args.n,
        args.prompt,
        args.timeout,
        args.sample_interval,
    )

    print("\n=== resource compare (under serial b64 load) ===")
    s12 = summarize_samples(p8012.samples)
    s14 = summarize_samples(p8014.samples)
    pr12 = summarize_proc(p8012.proc_before, p8012.proc_after)
    pr14 = summarize_proc(p8014.proc_before, p8014.proc_after)
    ok12 = sum(1 for r in p8012.requests if r.get("ok"))
    ok14 = sum(1 for r in p8014.requests if r.get("ok"))

    headers = [
        (
            "wall_avg_s",
            sum(r["wall_s"] for r in p8012.requests if r.get("ok")) / max(ok12, 1),
            sum(r["wall_s"] for r in p8014.requests if r.get("ok")) / max(ok14, 1),
        ),
        ("docker_cpu_avg%", s12.get("cpu_pct_avg"), s14.get("cpu_pct_avg")),
        ("docker_cpu_max%", s12.get("cpu_pct_max"), s14.get("cpu_pct_max")),
        ("docker_mem_avg_MB", s12.get("mem_mb_avg"), s14.get("mem_mb_avg")),
        ("docker_mem_max_MB", s12.get("mem_mb_max"), s14.get("mem_mb_max")),
        ("proc_rss_delta_KB", pr12.get("rss_delta_kb"), pr14.get("rss_delta_kb")),
        ("proc_cpu_delta_s", pr12.get("cpu_seconds_delta"), pr14.get("cpu_seconds_delta")),
        ("proc_threads", pr12.get("threads_after"), pr14.get("threads_after")),
    ]
    print(f"  {'metric':<22} {'8012':>12} {'8014':>12} {'8014/8012':>10}")
    for name, v12, v14 in headers:
        if v12 is None or v14 is None:
            continue
        ratio = f"{v14/v12:.2f}x" if v12 and isinstance(v12, (int, float)) and v12 != 0 else "—"
        print(f"  {name:<22} {v12:>12} {v14:>12} {ratio:>10}")

    print("\nnotes:")
    print("  - wall time dominated by upstream SSE (~25-40s); not gateway CPU bound")
    print("  - proc cpu_delta is container CPU during requests (mostly JSON/base64 + TLS upstream wait)")
    print("  - compare at equal concurrency after Phase-2 gray traffic stabilizes")

    rc = 0 if ok12 == args.n and ok14 == args.n else 1
    print(f"\nB64_CHAIN_PERF rc={rc}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
