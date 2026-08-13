#!/usr/bin/env python3
"""Pro 生图 WebSocket 探测：直连 wss://grok.com/ws/imagine/listen（对齐 Go image.go）。

Lite Drawing（conversations/new + enableImageGeneration）走 HTTP SSE，不经过此 WS。

用法：
  py -3.12 scripts/grok_imagine_pro_ws_probe.py --email aclarkdc8c@yumail.co
"""
from __future__ import annotations

import argparse
import asyncio
import json
import sys
import time
import uuid
from pathlib import Path

import os

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
REPORT_DIR = GROK_ROOT / "reports" / "imagine_ws"
WS_URL = "wss://grok.com/ws/imagine/listen"
PROXY = os.environ.get("GROK_LOCAL_PROXY", "http://127.0.0.1:7897")


def load_auth(email: str) -> dict:
    return json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))


def new_id(prefix: str = "img") -> str:
    return f"{prefix}_{uuid.uuid4().hex[:12]}"


def reset_msg() -> dict:
    return {
        "type": "conversation.item.create",
        "timestamp": int(time.time() * 1000),
        "item": {"type": "message", "content": [{"type": "reset"}]},
    }


def request_msg(prompt: str, ratio: str = "1:1", *, pro: bool = True, generations: int = 2) -> dict:
    return {
        "type": "conversation.item.create",
        "timestamp": int(time.time() * 1000),
        "item": {
            "type": "message",
            "content": [
                {
                    "requestId": new_id(),
                    "text": prompt,
                    "type": "input_text",
                    "properties": {
                        "section_count": 0,
                        "is_kids_mode": False,
                        "enable_nsfw": False,
                        "skip_upsampler": False,
                        "enable_side_by_side": True,
                        "is_initial": False,
                        "aspect_ratio": ratio,
                        "enable_pro": pro,
                        "num_generations": generations,
                    },
                }
            ],
        },
    }


async def probe(email: str, prompt: str, timeout: int, proxy: str | None) -> dict:
    import websockets

    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    cookie = f"sso={sso}; sso-rw={auth.get('sso_rw') or sso}"
    headers = {
        "Origin": "https://grok.com",
        "Cookie": cookie,
        "User-Agent": (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
            "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
        ),
        "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
    }
    frames: list[dict] = []
    out: dict = {"email": email, "ws_url": WS_URL, "frames": frames}

    try:
        kw: dict = {"extra_headers": headers, "open_timeout": 30}
        if proxy:
            kw["proxy"] = proxy
        async with websockets.connect(WS_URL, **kw) as ws:
            await ws.send(json.dumps(reset_msg()))
            await ws.send(json.dumps(request_msg(prompt)))
            deadline = time.time() + timeout
            while time.time() < deadline:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=5)
                except asyncio.TimeoutError:
                    if frames:
                        break
                    continue
                preview = raw if isinstance(raw, str) else raw.decode("utf-8", "replace")
                frames.append({"t": int(time.time() * 1000), "len": len(preview), "preview": preview[:8000]})
                print(f">>> RECV len={len(preview)}", flush=True)
                if any(k in preview for k in ("imageUrl", "b64", "completed", "failed", "error")):
                    if "completed" in preview or "imageUrl" in preview:
                        out["ok"] = True
                        break
    except Exception as exc:
        out["error"] = f"{type(exc).__name__}:{exc}"
        out["ok"] = False

    out["n_frames"] = len(frames)
    out.setdefault("ok", len(frames) > 0)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="aclarkdc8c@yumail.co")
    ap.add_argument("--prompt", default="a simple red apple on white background")
    ap.add_argument("--proxy", default=PROXY)
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--json-out", type=Path, default=None)
    args = ap.parse_args()

    report = asyncio.run(probe(args.email, args.prompt, args.timeout, args.proxy))
    out_path = args.json_out or REPORT_DIR / f"imagine_pro_ws_{args.email.replace('@', '_at_')}_{time.strftime('%Y%m%d-%H%M%S')}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"ok": report.get("ok"), "n_frames": report.get("n_frames"), "json": str(out_path)}, ensure_ascii=False))
    return 0 if report.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
