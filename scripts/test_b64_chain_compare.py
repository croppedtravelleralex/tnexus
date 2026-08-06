#!/usr/bin/env python3
"""Serial b64 image generation compare: gptimage :8012 vs TNexus gateway :8014."""
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
            "b64_len": len(b64),
            "bytes": len(raw),
            "dims": f"{dims[0]}x{dims[1]}" if dims else None,
            "email": pipe.get("account_email") or data.get("account_email"),
            "gateway_wall_ms": timings.get("gateway_wall_ms"),
            "upstream_wall_ms": timings.get("upstream_wall_ms"),
            "revised_prompt": item.get("revised_prompt"),
        }
    except urllib.error.HTTPError as e:
        wall = time.time() - t0
        err = e.read().decode("utf-8", errors="replace")[:400]
        return {"ok": False, "wall_s": round(wall, 2), "error": f"HTTP {e.code}: {err}"}
    except Exception as e:
        wall = time.time() - t0
        return {"ok": False, "wall_s": round(wall, 2), "error": str(e)}


def run_chain(label: str, base: str, auth: str, n: int, prompt: str, timeout: float) -> list[dict]:
    print(f"\n=== {label} ({base}) serial x{n} ===")
    out: list[dict] = []
    for i in range(n):
        p = f"{prompt} [b64-compare:{label}:{i}]"
        r = post_b64(base, auth, p, timeout)
        out.append(r)
        if r.get("ok"):
            print(
                f"  [{i + 1}] OK wall={r['wall_s']}s bytes={r['bytes']} "
                f"dims={r.get('dims')} email={r.get('email')} "
                f"gw_ms={r.get('gateway_wall_ms')} up_ms={r.get('upstream_wall_ms')}"
            )
        else:
            print(f"  [{i + 1}] FAIL wall={r['wall_s']}s {r.get('error', '')[:160]}")
    ok = sum(1 for r in out if r.get("ok"))
    ok_walls = [r["wall_s"] for r in out if r.get("ok")]
    avg = sum(ok_walls) / len(ok_walls) if ok_walls else 0
    print(f"  summary: {ok}/{n} ok  wall_avg={avg:.1f}s")
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--gptimage", default="http://127.0.0.1:8012")
    ap.add_argument("--tnexus", default="http://127.0.0.1:8014")
    ap.add_argument("--auth-8012", required=True)
    ap.add_argument("--auth-8014", required=True)
    ap.add_argument("-n", type=int, default=2, help="serial requests per chain")
    ap.add_argument("--timeout", type=float, default=300)
    ap.add_argument(
        "--prompt",
        default="a blue sphere on white background, studio product photo",
    )
    args = ap.parse_args()

    r8012 = run_chain("gptimage-8012", args.gptimage, args.auth_8012, args.n, args.prompt, args.timeout)
    r8014 = run_chain("tnexus-8014", args.tnexus, args.auth_8014, args.n, args.prompt, args.timeout)

    ok12 = sum(1 for r in r8012 if r.get("ok"))
    ok14 = sum(1 for r in r8014 if r.get("ok"))
    print("\n=== compare ===")
    print(f"  :8012 success {ok12}/{args.n}")
    print(f"  :8014 success {ok14}/{args.n}")

    if ok12 and ok14:
        w12 = sum(r["wall_s"] for r in r8012 if r.get("ok")) / ok12
        w14 = sum(r["wall_s"] for r in r8014 if r.get("ok")) / ok14
        b12 = [r["bytes"] for r in r8012 if r.get("ok")]
        b14 = [r["bytes"] for r in r8014 if r.get("ok")]
        print(f"  avg wall: 8012={w12:.1f}s  8014={w14:.1f}s  ratio={w14/w12:.2f}x")
        print(f"  bytes: 8012={b12}  8014={b14}")

    rc = 0 if ok12 == args.n and ok14 == args.n else 1
    print(f"\nB64_CHAIN_COMPARE rc={rc}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
