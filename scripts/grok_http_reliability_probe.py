#!/usr/bin/env python3
"""反复探测纯 HTTP：chat / OCR / rate-limits / Lite 生图 成功率。

用法：
  py -3.12 scripts/grok_http_reliability_probe.py --email aclarkdc8c@yumail.co --rounds 5
  GROK_UPSTREAM_PROXY=<udeal> py -3.12 scripts/grok_http_reliability_probe.py --email ... --rounds 3
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import (  # noqa: E402
    DEFAULT_OCR_IMAGE,
    DEFAULT_OCR_PROMPT,
    KEYS_DIR,
    GrokPureHttpClient,
    chat_payload,
)


def extract_image_urls(body: str) -> list[str]:
    found: list[str] = []
    for m in re.finditer(r'"imageUrl"\s*:\s*"([^"]+)"', body):
        u = m.group(1)
        if u and u not in found:
            found.append(u)
    for m in re.finditer(r'users/[^"\s]+/generated/[^"\s]+', body):
        u = m.group(0)
        if u not in found:
            found.append(u)
    return found


def probe_rate_limits(client: GrokPureHttpClient) -> dict:
    last: dict = {"ok": False, "http": 0}
    for body in [{}, {"modelName": "grok-3"}]:
        r = client.request("POST", "/rest/rate-limits", json_body=body)
        try:
            data = r.json() if r.status_code == 200 else None
        except json.JSONDecodeError:
            data = None
        row = {
            "ok": r.status_code == 200 and isinstance(data, dict) and "remainingQueries" in (data or {}),
            "http": r.status_code,
            "remainingQueries": (data or {}).get("remainingQueries"),
            "totalQueries": (data or {}).get("totalQueries"),
            "body": body,
        }
        if row["ok"]:
            return row
        last = row
    return last


def probe_chat(client: GrokPureHttpClient) -> dict:
    r = client.chat_new("Reply with exactly: PONG")
    return {"ok": bool(r.get("ok")), "http": r.get("http"), "kind": r.get("kind"), "reply": (r.get("reply") or "")[:40]}


def probe_ocr(client: GrokPureHttpClient, image: Path, prompt: str) -> dict:
    if not image.exists():
        return {"ok": False, "skipped": f"missing {image}"}
    up = client.upload_file(image, mime="image/png")
    if not up.get("ok"):
        return {"ok": False, "step": "upload", **up}
    r = client.chat_new(prompt, file_ids=[up["fileMetadataId"]])
    return {
        "ok": bool(r.get("ok")),
        "http": r.get("http"),
        "kind": r.get("kind"),
        "reply_len": len(r.get("reply") or ""),
    }


def probe_lite_image(client: GrokPureHttpClient, prompt: str = "a simple red apple on white background, minimal") -> dict:
    message = "Drawing: " + prompt.strip()
    body = chat_payload(message, enable_image=True)
    path = "/rest/app-chat/conversations/new"
    r = client.request("POST", path, json_body=body, stream=True, timeout=180)
    chunks: list[bytes] = []
    total = 0
    for chunk in r.iter_content(8192):
        if not chunk:
            continue
        chunks.append(chunk)
        total += len(chunk)
        if total > 2_000_000:
            break
    text = b"".join(chunks).decode("utf-8", errors="replace")
    urls = extract_image_urls(text)
    ok = r.status_code == 200 and bool(urls)
    return {
        "ok": ok,
        "http": r.status_code,
        "image_urls": urls[:3],
        "n_images": len(urls),
        "body_len": len(text),
    }


def run_round(client: GrokPureHttpClient, image: Path, ocr_prompt: str, lite_prompt: str) -> dict[str, Any]:
    return {
        "rate_limits": probe_rate_limits(client),
        "chat": probe_chat(client),
        "ocr": probe_ocr(client, image, ocr_prompt),
        "lite_image": probe_lite_image(client, lite_prompt),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="aclarkdc8c@yumail.co")
    ap.add_argument("--keys")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--sleep", type=float, default=2.0)
    ap.add_argument("--image", type=Path, default=DEFAULT_OCR_IMAGE)
    ap.add_argument("--ocr-prompt", default=DEFAULT_OCR_PROMPT)
    ap.add_argument("--lite-prompt", default="a simple red apple on white background, minimal")
    ap.add_argument("--signer", default="auto")
    ap.add_argument("--json-out", type=Path)
    args = ap.parse_args()

    keys_path = Path(args.keys) if args.keys else KEYS_DIR / f"{args.email.replace('@', '_at_')}.json"
    if not keys_path.exists():
        print(f"keys missing: {keys_path}; run --extract first", file=sys.stderr)
        return 1
    keys = json.loads(keys_path.read_text(encoding="utf-8"))
    keys["email"] = args.email
    client = GrokPureHttpClient(keys, signer=args.signer, upstream_proxy=os.environ.get("GROK_UPSTREAM_PROXY", ""))

    rounds: list[dict] = []
    for i in range(args.rounds):
        row = {"i": i, **run_round(client, args.image, args.ocr_prompt, args.lite_prompt)}
        rounds.append(row)
        print(json.dumps({"round": i, **{k: v.get("ok") for k, v in row.items() if isinstance(v, dict)}}, ensure_ascii=False), flush=True)
        if i + 1 < args.rounds:
            time.sleep(args.sleep)

    def rate(name: str) -> dict:
        ok = sum(1 for r in rounds if (r.get(name) or {}).get("ok"))
        return {"ok": ok, "total": len(rounds), "pct": round(100 * ok / len(rounds), 1) if rounds else 0}

    report = {
        "email": args.email,
        "rounds": args.rounds,
        "upstream_proxy": os.environ.get("GROK_UPSTREAM_PROXY", "")[:30],
        "local_proxy": os.environ.get("GROK_LOCAL_PROXY", ""),
        "success_rates": {
            "rate_limits": rate("rate_limits"),
            "chat": rate("chat"),
            "ocr": rate("ocr"),
            "lite_image": rate("lite_image"),
        },
        "detail": rounds,
    }
    out = args.json_out or KEYS_DIR / f"reliability_{args.email.replace('@', '_at_')}_{time.strftime('%Y%m%d-%H%M%S')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"success_rates": report["success_rates"], "json": str(out)}, ensure_ascii=False, indent=2))
    return 0 if all(report["success_rates"][k]["ok"] == report["success_rates"][k]["total"] for k in report["success_rates"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
