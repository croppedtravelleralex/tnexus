#!/usr/bin/env python3
"""Headed Playwright capture: HAR + incremental JSON, HTTP + WebSocket frames."""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
CANARY = Path(r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py")
CHAT_PATH = "/rest/app-chat/conversations/new"
APP_CHAT = "/rest/app-chat/"
WS_INTEREST = (
    "grok.com",
    "x.ai",
    "grpc",
    "chat",
    "stream",
    "connect",
    "ws",
)


def load_canary():
    import importlib.util

    spec = importlib.util.spec_from_file_location("canary", CANARY)
    mod = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(mod)
    return mod


def dismiss_age_gate(page) -> list[str]:
    """Dismiss Grok age modal (保存 or 继续 + birth year)."""
    notes: list[str] = []
    for label in ("忽略", "Dismiss", "Not now"):
        try:
            loc = page.get_by_role("button", name=label)
            if loc.count() and loc.first.is_visible():
                loc.first.click(timeout=2000, force=True)
                notes.append(f"promo:{label}")
                page.wait_for_timeout(600)
        except Exception:
            pass

    seen = False
    for text in ("请确认你的年龄", "请确认您的出生年份", "出生年份", "Birth year", "confirm your birth"):
        try:
            if page.get_by_text(text, exact=False).count():
                notes.append(f"age-seen:{text[:20]}")
                seen = True
                break
        except Exception:
            pass

    if not seen:
        notes.append("age-absent")
        return notes

    try:
        for sel in ('select', 'input[type="number"]', '[role="combobox"]'):
            loc = page.locator(sel).first
            if loc.count() and loc.is_visible():
                try:
                    loc.select_option("2000")
                    notes.append("age-year:select_option")
                except Exception:
                    try:
                        loc.fill("2000")
                        notes.append("age-year:fill")
                    except Exception:
                        pass
                break
    except Exception:
        pass

    for name in ("继续", "Continue", "保存", "Save"):
        try:
            loc = page.get_by_role("button", name=name)
            if loc.count() and loc.first.is_visible():
                loc.first.click(timeout=4000, force=True)
                notes.append(f"age-click:{name}")
                page.wait_for_timeout(2000)
                return notes
        except Exception:
            notes.append(f"age-fail:{name}")

    return notes


def flush_out(path: Path, out: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp.replace(path)


def ws_interesting(url: str) -> bool:
    u = url.lower()
    if u.startswith("wss://") or u.startswith("ws://"):
        return True
    return any(k in u for k in WS_INTEREST) and ("ws" in u or "wss" in u or "grpc" in u)


def clip_text(s: str | None, limit: int = 4000) -> str:
    if not s:
        return ""
    return s if len(s) <= limit else s[:limit] + "…"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--json-out", type=Path, default=GROK_ROOT / "reports" / "grok_manual_capture.json")
    ap.add_argument("--har-out", type=Path, default=GROK_ROOT / "reports" / "grok_manual_capture.har")
    args = ap.parse_args()

    canary = load_canary()
    auth = json.loads((GROK_ROOT / "web_auths" / f"{args.email}.json").read_text(encoding="utf-8"))
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)

    from playwright.sync_api import sync_playwright

    out: dict = {
        "email": args.email,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "requests": [],
        "chat_posts": [],
        "app_chat_posts": [],
        "ws_frames": [],
        "ws_connections": [],
        "notes": [],
        "listener": "playwright_context+websocket",
    }
    seen_keys: set[str] = set()
    ws_seen: set[str] = set()

    def record_request(req, *, source: str = "request") -> None:
        url = req.url
        if "/rest/" not in url and "/api/" not in url and not url.startswith(("wss", "ws:")):
            return
        path = urlparse(url).path
        sig = req.headers.get("x-statsig-id") or ""
        key = f"{req.method}:{url}:{sig[:24]}"
        if key in seen_keys:
            return
        seen_keys.add(key)
        is_chat = CHAT_PATH in path or (APP_CHAT in path and req.method.upper() == "POST")
        row = {
            "t": int(time.time() * 1000),
            "source": source,
            "method": req.method,
            "path": path,
            "url": url[:300],
            "sig_len": len(sig),
            "sig": sig if is_chat else (sig[:48] + "…" if len(sig) > 48 else sig),
            "headers": {k: v for k, v in req.headers.items()} if is_chat else {},
            "post_data": clip_text(req.post_data) if is_chat else "",
        }
        out["requests"].append(row)
        if CHAT_PATH in path and req.method.upper() == "POST":
            out["chat_posts"].append(row)
            print(f"\n>>> CAPTURED POST {CHAT_PATH} sig_len={len(sig)}\n", flush=True)
        elif APP_CHAT in path and req.method.upper() == "POST":
            out["app_chat_posts"].append(row)
            print(f"\n>>> app-chat POST {path} sig_len={len(sig)}\n", flush=True)
        flush_out(args.json_out, out)

    def record_ws_frame(ws, event: str, payload: str | bytes | None) -> None:
        url = ws.url
        if not ws_interesting(url) and "grok" not in url.lower() and "x.ai" not in url.lower():
            return
        text = payload if isinstance(payload, str) else (payload.decode("utf-8", "replace") if payload else "")
        preview = clip_text(text, 8000)
        key = f"{event}:{url}:{hash(preview) & 0xFFFFFFFF:08x}"
        if key in ws_seen:
            return
        ws_seen.add(key)
        row = {
            "t": int(time.time() * 1000),
            "event": event,
            "url": url[:400],
            "len": len(text),
            "preview": preview,
        }
        out["ws_frames"].append(row)
        tag = "SEND" if event == "framesent" else "RECV"
        print(f"\n>>> WS {tag} {url[:80]} len={len(text)}\n", flush=True)
        flush_out(args.json_out, out)

    def attach_ws_listeners(ws) -> None:
        conn = {"t": int(time.time() * 1000), "url": ws.url[:400]}
        out["ws_connections"].append(conn)
        print(f"\n>>> WS OPEN {ws.url[:120]}\n", flush=True)

        def on_frame_sent(payload):
            record_ws_frame(ws, "framesent", payload)

        def on_frame_received(payload):
            record_ws_frame(ws, "framereceived", payload)

        def on_close():
            out["notes"].append(f"ws_close:{ws.url[:80]}")

        ws.on("framesent", on_frame_sent)
        ws.on("framereceived", on_frame_received)
        ws.on("close", on_close)
        flush_out(args.json_out, out)

    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=False,
            args=[
                "--disable-blink-features=AutomationControlled",
                "--start-maximized",
            ],
        )
        args.har_out.parent.mkdir(parents=True, exist_ok=True)
        ctx = browser.new_context(
            proxy={"server": canary.PROXY},
            user_agent=canary.UA,
            viewport={"width": 1400, "height": 900},
            record_har_path=str(args.har_out),
            record_har_content="attach",
        )
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_init_script(canary.CAPTURE_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )

        def on_ctx_request(req):
            record_request(req, source="context")

        def on_ctx_websocket(ws):
            attach_ws_listeners(ws)

        ctx.on("request", on_ctx_request)
        ctx.on("websocket", on_ctx_websocket)

        page = ctx.new_page()

        def on_page_websocket(ws):
            attach_ws_listeners(ws)

        page.on("websocket", on_page_websocket)

        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(3000)
        out["notes"].extend(dismiss_age_gate(page))
        out["meta"] = page.evaluate(
            """() => {
              const el = document.querySelector('meta[name^="gr"], meta[name*="grok-site"]');
              return el ? el.getAttribute('content') : null;
            }"""
        )
        flush_out(args.json_out, out)

        banner = f"""
{'=' * 70}
【重要】请在这个 Chromium 窗口里操作，不要用 Cursor 内置浏览器或系统 Chrome！
窗口标题通常含 "Chromium" 或 "Grok"
账号: {args.email}
已注入 SSO；年龄弹窗会自动点「继续」
HTTP + WebSocket 抓包实时写入:
  {args.json_out}
  {args.har_out}
发消息后无需关窗，每 5s 自动落盘；抓满或按 Ctrl+C 结束
{'=' * 70}
"""
        print(banner, flush=True)

        deadline = time.time() + args.timeout
        last_flush = time.time()
        while time.time() < deadline:
            if not browser.is_connected():
                out["notes"].append("browser_disconnected")
                break
            if time.time() - last_flush > 5:
                try:
                    js_caps = page.evaluate("() => (globalThis.__grokCapturedSigs || []).slice(-80)") or []
                    out["captured_sigs"] = js_caps
                except Exception:
                    pass
                flush_out(args.json_out, out)
                last_flush = time.time()
            page.wait_for_timeout(500)

        try:
            js_caps = page.evaluate("() => (globalThis.__grokCapturedSigs || []).slice(-80)") or []
            out["captured_sigs"] = js_caps
        except Exception:
            pass
        cookies = ctx.cookies("https://grok.com/")
        out["cookie_names"] = sorted({c["name"] for c in cookies})
        out["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        ctx.close()
        browser.close()

    flush_out(args.json_out, out)
    summary = {
        "ok": bool(out.get("chat_posts") or out.get("app_chat_posts") or out.get("ws_frames")),
        "n_req": len(out["requests"]),
        "n_chat_new": len(out.get("chat_posts") or []),
        "n_app_chat_post": len(out.get("app_chat_posts") or []),
        "n_ws_frames": len(out.get("ws_frames") or []),
        "n_ws_connections": len(out.get("ws_connections") or []),
        "json": str(args.json_out),
        "har": str(args.har_out),
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
