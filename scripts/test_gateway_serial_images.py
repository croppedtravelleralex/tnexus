#!/usr/bin/env python3
"""Serial gpt-image-2 generations against TNexus gateway (:8014)."""
import argparse
import base64
import json
import struct
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_http_headers import request_headers


def png_dims(data: bytes) -> tuple[int, int] | None:
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


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
            }
    except urllib.error.HTTPError as e:
        elapsed = time.perf_counter() - t0
        err_body = e.read().decode("utf-8", errors="replace")[:300]
        return {
            "ok": False,
            "elapsed_s": round(elapsed, 2),
            "status": e.code,
            "error": err_body,
            "retryable": e.code in {408, 429, 500, 502, 503, 524},
        }
    except Exception as e:
        msg = str(e)
        return {
            "ok": False,
            "elapsed_s": round(time.perf_counter() - t0, 2),
            "error": msg,
            "retryable": "closed connection" in msg.lower()
            or "timed out" in msg.lower(),
        }


def one_image_with_retry(
    base_url: str, api_key: str, prompt: str, model: str, timeout: float, retries: int
) -> dict:
    last: dict = {}
    for attempt in range(retries + 1):
        last = one_image(base_url, api_key, prompt, model, timeout)
        if last.get("ok") or not last.get("retryable") or attempt >= retries:
            last["attempts"] = attempt + 1
            return last
        wait = min(5 * (attempt + 1), 15)
        print(f"  retry {attempt + 1}/{retries} ({last.get('error', '')[:60]})", flush=True)
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
    p.add_argument(
        "--prompt",
        default="A serene mountain lake at sunrise, soft watercolor style, no text",
    )
    args = p.parse_args()

    results = []
    for i in range(args.count):
        prompt = f"{args.prompt} (serial test {i + 1})"
        print(f"[{i + 1}/{args.count}] generating...", flush=True)
        r = one_image_with_retry(
            args.base_url, args.api_key, prompt, args.model, args.timeout, args.retries
        )
        results.append(r)
        mark = "OK" if r.get("ok") else "FAIL"
        extra = ""
        if r.get("ok"):
            extra = f" out_tok={r.get('output_tokens')} email={r.get('email')}"
        print(
            f"  {mark} {r.get('elapsed_s')}s status={r.get('status', '-')}"
            f" dims={r.get('dims')}{extra}",
            flush=True,
        )
        if not r.get("ok"):
            print(f"  error: {r.get('error', '')[:200]}", flush=True)

    ok = sum(1 for r in results if r.get("ok"))
    print("---")
    print(f"success: {ok}/{args.count} ({100 * ok / args.count:.0f}%)")
    times = [r["elapsed_s"] for r in results if r.get("ok")]
    if times:
        print(f"ok latency: min={min(times):.1f}s max={max(times):.1f}s avg={sum(times)/len(times):.1f}s")
    return 0 if ok == args.count else 1


if __name__ == "__main__":
    sys.exit(main())
