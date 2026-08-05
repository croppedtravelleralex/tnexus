#!/usr/bin/env python3
"""Serial gpt-image-2 generations (TNexus / NewAPI / sub2api).

Avoids duplicate_prompt 429 by unique suffix per attempt; retries always use a fresh prompt.
"""
from __future__ import annotations

import argparse
import base64
import json
import secrets
import struct
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_http_headers import request_headers

PROMPT_SHORT = "A single red apple on a plain white background, product photo."
PROMPT_MEDIUM = (
    "A cozy bookstore interior at dusk, warm amber lighting, wooden shelves lined with "
    "colorful books, a reading chair by the window, soft watercolor illustration style, "
    "no text, no people."
)
PROMPT_LONG = (
    "An expansive alpine valley at golden hour after a summer rain: distant snow-capped peaks "
    "catch pink sunlight, a winding river reflects the sky, wildflowers in the foreground "
    "(purple lupines and yellow buttercups), scattered pine trees on rolling hills, thin mist "
    "in the middle distance, cinematic wide composition, painterly realism, crisp details in "
    "the midground, soft atmospheric perspective toward the mountains, no buildings, no text, "
    "no logos, no watermarks."
)

PROFILE_ORDER = ("short", "medium", "long")
PROFILE_PROMPTS = {
    "short": PROMPT_SHORT,
    "medium": PROMPT_MEDIUM,
    "long": PROMPT_LONG,
}


def png_dims(data: bytes) -> tuple[int, int] | None:
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


def unique_suffix() -> str:
    return f"uid-{time.time_ns()}-{secrets.token_hex(4)}"


def build_prompt(base: str, profile: str, slot: int, attempt: int) -> str:
    return f"{base} [{profile} slot={slot} attempt={attempt} {unique_suffix()}]"


def one_image(base_url: str, api_key: str, prompt: str, model: str, timeout: float) -> dict:
    url = f"{base_url.rstrip('/')}/v1/images/generations"
    body = json.dumps(
        {
            "model": model,
            "prompt": prompt,
            "size": "1024x1024",
            "response_format": "b64_json",
            "n": 1,
        }
    ).encode()
    req = urllib.request.Request(
        url,
        data=body,
        headers=request_headers(
            {
                "Authorization": f"Bearer {api_key}",
                "Content-Type": "application/json",
            }
        ),
        method="POST",
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            elapsed = time.perf_counter() - t0
            data = json.loads(raw)
            item = (data.get("data") or [{}])[0]
            b64 = item.get("b64_json") or ""
            img = base64.b64decode(b64) if b64 else b""
            dims = png_dims(img)
            usage = data.get("usage") or {}
            pipe = data.get("_tnexus_pipeline") or {}
            return {
                "ok": bool(img),
                "elapsed_s": round(elapsed, 2),
                "bytes": len(img),
                "dims": dims,
                "status": resp.status,
                "usage": usage,
                "email": pipe.get("account_email"),
                "output_tokens": usage.get("output_tokens"),
                "prompt_chars": len(prompt),
            }
    except urllib.error.HTTPError as e:
        elapsed = time.perf_counter() - t0
        err_body = e.read().decode("utf-8", errors="replace")[:400]
        retryable = e.code in {408, 500, 502, 503, 524} or (
            e.code == 429 and "duplicate" not in err_body.lower()
        )
        return {
            "ok": False,
            "elapsed_s": round(elapsed, 2),
            "status": e.code,
            "error": err_body,
            "retryable": retryable,
            "prompt_chars": len(prompt),
        }
    except Exception as e:
        msg = str(e)
        low = msg.lower()
        return {
            "ok": False,
            "elapsed_s": round(time.perf_counter() - t0, 2),
            "error": msg,
            "retryable": any(
                x in low
                for x in (
                    "closed connection",
                    "timed out",
                    "incompleteread",
                    "connection reset",
                )
            ),
            "prompt_chars": len(prompt),
        }


def one_image_with_retry(
    base_url: str,
    api_key: str,
    profile: str,
    base_prompt: str,
    slot: int,
    model: str,
    timeout: float,
    retries: int,
) -> dict:
    last: dict = {}
    for attempt in range(retries + 1):
        prompt = build_prompt(base_prompt, profile, slot, attempt + 1)
        last = one_image(base_url, api_key, prompt, model, timeout)
        last["profile"] = profile
        last["attempts"] = attempt + 1
        if last.get("ok") or not last.get("retryable") or attempt >= retries:
            return last
        wait = min(5 * (attempt + 1), 15)
        print(
            f"  retry {attempt + 1}/{retries} ({last.get('error', '')[:70]}) wait {wait}s",
            flush=True,
        )
        time.sleep(wait)
    return last


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--base-url", default="http://127.0.0.1:8014")
    p.add_argument("--api-key", required=True)
    p.add_argument("--model", default="gpt-image-2")
    p.add_argument("--count", type=int, default=10)
    p.add_argument("--timeout", type=float, default=300.0)
    p.add_argument("--retries", type=int, default=2)
    p.add_argument("--gap", type=float, default=2.0, help="seconds between serial requests")
    p.add_argument(
        "--profiles",
        default="short,medium,long",
        help="comma-separated prompt length profiles to cycle",
    )
    p.add_argument("--prompt", default="", help="optional prefix prepended to every prompt")
    args = p.parse_args()

    profiles = [x.strip() for x in args.profiles.split(",") if x.strip()]
    for name in profiles:
        if name not in PROFILE_PROMPTS:
            print(f"unknown profile {name!r}, choose from {list(PROFILE_PROMPTS)}", file=sys.stderr)
            return 2

    results = []
    for i in range(args.count):
        profile = profiles[i % len(profiles)]
        base_prompt = args.prompt + PROFILE_PROMPTS[profile]
        print(f"[{i + 1}/{args.count}] profile={profile} chars≈{len(base_prompt)}", flush=True)
        r = one_image_with_retry(
            args.base_url,
            args.api_key,
            profile,
            base_prompt,
            i + 1,
            args.model,
            args.timeout,
            args.retries,
        )
        results.append(r)
        mark = "OK" if r.get("ok") else "FAIL"
        extra = ""
        if r.get("ok"):
            extra = f" out_tok={r.get('output_tokens')} email={r.get('email')}"
        print(
            f"  {mark} {r.get('elapsed_s')}s status={r.get('status', '-')}"
            f" dims={r.get('dims')} attempts={r.get('attempts')}{extra}",
            flush=True,
        )
        if not r.get("ok"):
            print(f"  error: {r.get('error', '')[:220]}", flush=True)
        if i + 1 < args.count and args.gap > 0:
            time.sleep(args.gap)

    ok = sum(1 for r in results if r.get("ok"))
    print("---")
    print(f"success: {ok}/{args.count} ({100 * ok / args.count:.0f}%)")
    times = [r["elapsed_s"] for r in results if r.get("ok")]
    if times:
        print(f"ok latency: min={min(times):.1f}s max={max(times):.1f}s avg={sum(times)/len(times):.1f}s")
    by_profile: dict[str, list[dict]] = {}
    for r in results:
        by_profile.setdefault(r.get("profile", "?"), []).append(r)
    for name, rows in by_profile.items():
        ok_n = sum(1 for r in rows if r.get("ok"))
        print(f"  {name}: {ok_n}/{len(rows)} ok")
    return 0 if ok == args.count else 1


if __name__ == "__main__":
    sys.exit(main())
