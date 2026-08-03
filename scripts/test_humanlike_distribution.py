#!/usr/bin/env python3
"""Compare account pick distribution: TNexus gateway :8014 vs gptimage :8012.

gptimage sync JSON does not include account_email — we correlate via logs.jsonl
(task_id + request_text + schedule_trace.account_email).

Usage (on Panda):
  python3 scripts/test_humanlike_distribution.py --n 12 --concurrency 3
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed

DEFAULT_PROMPT = "a tiny blue square on white background"


def load_auth_key() -> str:
    key = os.environ.get("GATEWAY_AUTH_KEY") or os.environ.get("UPSTREAM_API_KEY") or ""
    if key:
        return key.strip()
    env_path = os.environ.get("TNEXUS_ENV", "/opt/tnexus/.env")
    if os.path.isfile(env_path):
        for line in open(env_path, encoding="utf-8"):
            if line.startswith("GATEWAY_AUTH_KEY="):
                return line.split("=", 1)[1].strip().strip('"')
    return ""


def load_gptimage_auth() -> str:
    key = os.environ.get("GPTIMAGE_AUTH_KEY") or os.environ.get("GPTIMAGE_API_KEY") or ""
    if key:
        return key.strip()
    config_path = os.environ.get("GPTIMAGE_CONFIG", "/root/gptimage/config.json")
    if os.path.isfile(config_path):
        with open(config_path, encoding="utf-8") as f:
            cfg = json.load(f)
        return str(cfg.get("auth-key") or cfg.get("auth_key") or "").strip()
    return ""


def file_size(path: str) -> int:
    try:
        return os.path.getsize(path)
    except OSError:
        return 0


def tail_json_lines(path: str, since: int, max_bytes: int = 8_000_000) -> list[str]:
    if not os.path.isfile(path):
        return []
    size = os.path.getsize(path)
    start = max(since, size - max_bytes)
    with open(path, "r", encoding="utf-8", errors="replace") as f:
        f.seek(start)
        return [ln for ln in f.read().splitlines() if ln.strip()]


def pick_email_from_detail(detail: dict) -> str | None:
    email = str(detail.get("account_email") or "").strip()
    if email:
        return email
    trace = detail.get("schedule_trace")
    if isinstance(trace, dict):
        email = str(trace.get("account_email") or "").strip()
        if email:
            return email
    return None


def extract_from_response(data: dict) -> str | None:
    for key in ("pipeline", "tnexus_pipeline", "_tnexus_pipeline"):
        pipe = data.get(key)
        if isinstance(pipe, dict):
            email = pipe.get("account_email") or pipe.get("email")
            if email:
                return str(email).strip()
    if isinstance(data.get("data"), list) and data["data"]:
        email = data["data"][0].get("account_email")
        if email:
            return str(email).strip()
    return None


def extract_gptimage_from_logs(
    data: dict,
    prompt: str,
    since_pos: int,
    log_path: str,
) -> str | None:
    task_id = str(data.get("task_id") or "").strip()
    prompt = prompt.strip()
    for line in reversed(tail_json_lines(log_path, since_pos)):
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if rec.get("type") != "call":
            continue
        detail = rec.get("detail") or {}
        if task_id and str(detail.get("task_id") or "") == task_id:
            return pick_email_from_detail(detail)
        req = str(detail.get("request_text") or "")
        if prompt and prompt in req:
            email = pick_email_from_detail(detail)
            if email:
                return email
    return None


def extract_tnexus_from_pipeline(since_pos: int, pipeline_path: str) -> str | None:
    for line in reversed(tail_json_lines(pipeline_path, since_pos)):
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if rec.get("kind") == "gateway_image" and rec.get("ok"):
            email = str(rec.get("email") or "").strip()
            if email:
                return email
    return None


def post_image(
    base: str,
    auth: str,
    prompt: str,
    timeout: float,
    *,
    gptimage_logs: str | None = None,
    tnexus_pipeline: str | None = None,
) -> tuple[str, str | None]:
    url = f"{base.rstrip('/')}/v1/images/generations"
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
        url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {auth}",
        },
        method="POST",
    )
    log_since = file_size(gptimage_logs) if gptimage_logs else 0
    pipe_since = file_size(tnexus_pipeline) if tnexus_pipeline else 0
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", errors="replace")[:200]
        return "error", f"HTTP {e.code}: {text}"
    except Exception as e:
        return "error", str(e)

    email = extract_from_response(data)
    if not email and gptimage_logs:
        email = extract_gptimage_from_logs(data, prompt, log_since, gptimage_logs)
    if not email and tnexus_pipeline:
        email = extract_tnexus_from_pipeline(pipe_since, tnexus_pipeline)
    return "ok", email or "unknown"


def run_batch(
    label: str,
    base: str,
    auth: str,
    n: int,
    concurrency: int,
    prompt: str,
    timeout: float,
    unique_prompts: bool,
    gptimage_logs: str | None,
    tnexus_pipeline: str | None,
) -> Counter:
    counter: Counter = Counter()
    errors: list[str] = []
    lock = threading.Lock()
    is_gptimage = ":8012" in base or label.endswith("8012")

    def one(i: int) -> None:
        p = f"{prompt} #{i}" if unique_prompts else prompt
        status, info = post_image(
            base,
            auth,
            p,
            timeout,
            gptimage_logs=gptimage_logs if is_gptimage else None,
            tnexus_pipeline=tnexus_pipeline if not is_gptimage else None,
        )
        with lock:
            if status == "ok":
                counter[info or "unknown"] += 1
            else:
                errors.append(info or "unknown error")
                counter["__error__"] += 1

    t0 = time.time()
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(one, i) for i in range(n)]
        for f in as_completed(futures):
            f.result()
    elapsed = time.time() - t0
    print(f"\n[{label}] base={base} n={n} concurrency={concurrency} elapsed={elapsed:.1f}s")
    print(f"  accounts: {dict(counter)}")
    unknown = counter.get("unknown", 0)
    if unknown:
        print(f"  warn: {unknown} requests could not resolve account_email")
    if errors:
        print(f"  sample errors: {errors[:3]}")
    return counter


def deviation_pct(a: Counter, b: Counter) -> float:
    """Max per-account share delta between two histograms (excluding errors)."""
    keys = set(a) | set(b)
    keys.discard("__error__")
    keys.discard("unknown")
    if not keys:
        return 100.0
    ta = sum(v for k, v in a.items() if k not in ("__error__", "unknown")) or 1
    tb = sum(v for k, v in b.items() if k not in ("__error__", "unknown")) or 1
    max_delta = 0.0
    for k in keys:
        pa = a.get(k, 0) / ta
        pb = b.get(k, 0) / tb
        max_delta = max(max_delta, abs(pa - pb))
    return max_delta * 100.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tnexus", default="http://127.0.0.1:8014", help="TNexus gateway base")
    ap.add_argument("--gptimage", default="http://127.0.0.1:8012", help="gptimage :8012 base")
    ap.add_argument("--n", type=int, default=30, help="requests per endpoint")
    ap.add_argument("--concurrency", type=int, default=6)
    ap.add_argument("--prompt", default=DEFAULT_PROMPT)
    ap.add_argument("--timeout", type=float, default=180.0)
    ap.add_argument("--max-deviation", type=float, default=15.0, help="pass threshold %%")
    ap.add_argument(
        "--unique-prompts",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="unique prompt per request (avoid duplicate_prompt 429)",
    )
    ap.add_argument("--skip-gptimage", action="store_true", help="only run TNexus side")
    ap.add_argument(
        "--gptimage-logs",
        default=os.environ.get("GPTIMAGE_LOGS", "/root/gptimage/data/logs.jsonl"),
    )
    ap.add_argument(
        "--tnexus-pipeline",
        default=os.environ.get("PIPELINE_EVENTS_FILE", "/opt/tnexus/data/pool/pipeline_events.ndjson"),
    )
    args = ap.parse_args()

    auth = load_auth_key()
    if not auth:
        print("GATEWAY_AUTH_KEY not set", file=sys.stderr)
        return 1

    c8014 = run_batch(
        "tnexus-8014",
        args.tnexus,
        auth,
        args.n,
        args.concurrency,
        args.prompt,
        args.timeout,
        args.unique_prompts,
        args.gptimage_logs,
        args.tnexus_pipeline,
    )

    ok8014 = sum(v for k, v in c8014.items() if k not in ("__error__", "unknown"))
    if ok8014 < max(3, args.n // 4):
        print(f"FAIL: tnexus too few successes ({ok8014}/{args.n})", file=sys.stderr)
        return 3

    if args.skip_gptimage:
        print("SKIP gptimage (--skip-gptimage)")
        return 0

    gauth = load_gptimage_auth()
    if not gauth:
        print("WARN: gptimage auth-key not found; use --skip-gptimage or set GPTIMAGE_CONFIG", file=sys.stderr)
        return 4

    if not os.path.isfile(args.gptimage_logs):
        print(f"WARN: gptimage logs missing: {args.gptimage_logs}", file=sys.stderr)
        return 4

    c8012 = run_batch(
        "gptimage-8012",
        args.gptimage,
        gauth,
        args.n,
        args.concurrency,
        args.prompt,
        args.timeout,
        args.unique_prompts,
        args.gptimage_logs,
        args.tnexus_pipeline,
    )

    ok8012 = sum(v for k, v in c8012.items() if k not in ("__error__", "unknown"))
    if ok8012 < max(3, args.n // 4):
        print(f"FAIL: gptimage too few account resolutions ({ok8012}/{args.n})", file=sys.stderr)
        return 5

    dev = deviation_pct(c8014, c8012)
    print(f"\nmax per-account share deviation: {dev:.1f}% (threshold {args.max_deviation}%)")
    if dev > args.max_deviation:
        print("FAIL: humanlike distribution diverges from :8012")
        return 2
    print("PASS: within threshold")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
