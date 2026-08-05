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
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
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
            return {
                "ok": bool(img),
                "elapsed_s": round(elapsed, 2),
                "bytes": len(img),
                "dims": dims,
                "status": resp.status,
            }
    except urllib.error.HTTPError as e:
        elapsed = time.perf_counter() - t0
        err_body = e.read().decode("utf-8", errors="replace")[:300]
        return {
            "ok": False,
            "elapsed_s": round(elapsed, 2),
            "status": e.code,
            "error": err_body,
        }
    except Exception as e:
        return {
            "ok": False,
            "elapsed_s": round(time.perf_counter() - t0, 2),
            "error": str(e),
        }


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--base-url", default="http://127.0.0.1:8014")
    p.add_argument("--api-key", required=True)
    p.add_argument("--model", default="gpt-image-2")
    p.add_argument("--count", type=int, default=10)
    p.add_argument("--timeout", type=float, default=300.0)
    p.add_argument(
        "--prompt",
        default="A serene mountain lake at sunrise, soft watercolor style, no text",
    )
    args = p.parse_args()

    results = []
    for i in range(args.count):
        prompt = f"{args.prompt} (serial test {i + 1})"
        print(f"[{i + 1}/{args.count}] generating...", flush=True)
        r = one_image(args.base_url, args.api_key, prompt, args.model, args.timeout)
        results.append(r)
        mark = "OK" if r.get("ok") else "FAIL"
        print(f"  {mark} {r.get('elapsed_s')}s status={r.get('status', '-')} dims={r.get('dims')}", flush=True)
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
