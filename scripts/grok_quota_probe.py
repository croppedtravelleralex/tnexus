#!/usr/bin/env python3
"""Grok 额度探测：rate-limits + 多账号压测对话/上传上限。

用法：
  python scripts/grok_quota_probe.py --keys path/to/pure_http_keys.json
  python scripts/grok_quota_probe.py --stress-chat --max-rounds 50
  python scripts/grok_account_quota_scan.py --limit 30   # 无 keys 批量扫额度
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import KEYS_DIR, GrokPureHttpClient, load_auth  # noqa: E402


def probe_rate_limits(client: GrokPureHttpClient) -> dict:
    for body in [{}, None, {"modelName": "grok-3"}]:
        r = client.request("POST", "/rest/rate-limits", json_body=body if body is not None else {})
        if r.status_code == 200:
            try:
                return {"ok": True, "http": 200, "data": r.json()}
            except json.JSONDecodeError:
                return {"ok": True, "http": 200, "raw": r.text[:500]}
        last = {"ok": False, "http": r.status_code, "body": r.text[:300]}
    return last


def stress_chat(client: GrokPureHttpClient, max_rounds: int) -> dict:
    rounds = []
    conv_id = None
    parent = None
    for i in range(max_rounds):
        if i == 0:
            r = client.chat_new("Reply with exactly: PONG")
        else:
            r = client.chat_followup(conv_id, parent, f"Reply with exactly: PONG{i+1}")
        rounds.append({"i": i, "ok": r.get("ok"), "http": r.get("http"), "kind": r.get("kind"), "reply": (r.get("reply") or "")[:40]})
        if not r.get("ok"):
            break
        conv_id = r.get("conversation_id") or conv_id
        parent = r.get("response_id")
        time.sleep(0.3)
    return {"rounds": len(rounds), "last_ok": rounds[-1]["ok"] if rounds else False, "detail": rounds[-5:]}


def stress_upload(client: GrokPureHttpClient, image: Path, max_uploads: int) -> dict:
    if not image.exists():
        return {"ok": False, "error": f"missing image {image}"}
    ok = 0
    last = None
    for i in range(max_uploads):
        up = client.upload_file(image, mime="image/png")
        last = up
        if not up.get("ok"):
            break
        ok += 1
        time.sleep(0.2)
    return {"uploads_ok": ok, "last": last}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--keys")
    ap.add_argument("--stress-chat", action="store_true")
    ap.add_argument("--stress-upload", action="store_true")
    ap.add_argument("--max-rounds", type=int, default=40)
    ap.add_argument("--max-uploads", type=int, default=20)
    ap.add_argument("--image", type=Path)
    args = ap.parse_args()

    keys_path = Path(args.keys) if args.keys else KEYS_DIR / f"{args.email.replace('@', '_at_')}.json"
    keys = json.loads(keys_path.read_text(encoding="utf-8"))
    keys["email"] = args.email
    client = GrokPureHttpClient(keys, signer="auto")

    report = {"email": args.email, "rate_limits": probe_rate_limits(client)}
    if args.stress_chat:
        report["stress_chat"] = stress_chat(client, args.max_rounds)
    if args.stress_upload:
        img = args.image or Path(r"C:\Users\Lenovo\Downloads\image-1785287126849-88e3a45901dc98-1785287699703-649ee24e9542d8.png")
        report["stress_upload"] = stress_upload(client, img, args.max_uploads)

    out = KEYS_DIR / f"quota_probe_{args.email.replace('@', '_at_')}.json"
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
