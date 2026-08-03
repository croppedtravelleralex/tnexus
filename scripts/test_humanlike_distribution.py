#!/usr/bin/env python3
"""Compare account pick distribution: TNexus gateway :8014 vs gptimage :8012.

Usage (on Panda):
  export GATEWAY_AUTH_KEY=...   # or read from /opt/tnexus/.env
  python3 scripts/test_humanlike_distribution.py --n 20 --concurrency 4
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


def post_image(base: str, auth: str, prompt: str, timeout: float) -> tuple[str, str | None]:
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
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", errors="replace")[:200]
        return "error", f"HTTP {e.code}: {text}"
    except Exception as e:
        return "error", str(e)

    email = None
    pipe = data.get("pipeline") or data.get("tnexus_pipeline") or data.get("_tnexus_pipeline")
    if isinstance(pipe, dict):
        email = pipe.get("account_email") or pipe.get("email")
    if not email and isinstance(data.get("data"), list) and data["data"]:
        email = data["data"][0].get("account_email")
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
) -> Counter:
    counter: Counter = Counter()
    errors: list[str] = []
    lock = threading.Lock()

    def one(i: int) -> None:
        p = f"{prompt} #{i}" if unique_prompts else prompt
        status, info = post_image(base, auth, p, timeout)
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

    c8012 = run_batch(
        "gptimage-8012",
        args.gptimage,
        gauth,
        args.n,
        args.concurrency,
        args.prompt,
        args.timeout,
        args.unique_prompts,
    )

    ok8012 = sum(v for k, v in c8012.items() if k not in ("__error__", "unknown"))
    if ok8012 < max(3, args.n // 4):
        print(f"WARN: gptimage too few successes ({ok8012}/{args.n}); deviation check skipped")
        return 0

    dev = deviation_pct(c8014, c8012)
    print(f"\nmax per-account share deviation: {dev:.1f}% (threshold {args.max_deviation}%)")
    if dev > args.max_deviation:
        print("FAIL: humanlike distribution diverges from :8012")
        return 2
    print("PASS: within threshold")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
