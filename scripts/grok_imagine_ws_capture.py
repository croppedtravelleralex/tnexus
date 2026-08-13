#!/usr/bin/env python3
"""Playwright 自动触发 Lite 生图并抓取 wss://grok.com/ws/imagine/listen 帧。

用法：
  python scripts/grok_imagine_ws_capture.py --email nancybaker2jyy@yumail.co
  python scripts/grok_imagine_ws_capture.py --email foo@bar.com --headed --timeout 120
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grok_playwright_common import CAPTURE_HOOK, PROXY, TURBOPACK_HOOK, UA, chat_payload

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
REPORT_DIR = GROK_ROOT / "reports" / "imagine_ws"
IMAGINE_WS = "imagine/listen"
DEFAULT_PROMPT = "a simple red apple on white background, minimal"


def load_auth(email: str) -> dict:
    return json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))


def dismiss_age_gate(page) -> list[str]:
    notes: list[str] = []
    for label in ("忽略", "Dismiss", "Not now", "继续", "Continue"):
        try:
            loc = page.get_by_role("button", name=label)
            if loc.count() and loc.first.is_visible():
                loc.first.click(timeout=2000, force=True)
                notes.append(f"age:{label}")
                page.wait_for_timeout(800)
        except Exception:
            pass
    return notes


def clip_text(text: str, limit: int = 12000) -> str:
    if len(text) <= limit:
        return text
    return text[: limit // 2] + "\n…\n" + text[-limit // 2 :]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--proxy", default=None, help="override proxy, default canary PROXY")
    ap.add_argument("--prompt", default=DEFAULT_PROMPT)
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--timeout", type=int, default=180)
    ap.add_argument("--hybrid-http", action="store_true", help="用 pure_http keys 发 POST，Playwright 只抓 WS")
    ap.add_argument("--keys", type=Path, default=None)
    ap.add_argument("--json-out", type=Path, default=None)
    args = ap.parse_args()

    auth = load_auth(args.email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    proxy = args.proxy or PROXY
    keys_path = args.keys or (GROK_ROOT / "reports" / "pure_http_keys" / f"{args.email.replace('@', '_at_')}.json")
    out_path = args.json_out or REPORT_DIR / f"imagine_ws_{args.email.replace('@', '_at_')}_{time.strftime('%Y%m%d-%H%M%S')}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    from playwright.sync_api import sync_playwright

    message = "Drawing: " + args.prompt.strip()
    payload = chat_payload(message, enable_image=True)

    out: dict = {
        "email": args.email,
        "proxy": proxy,
        "prompt": message,
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "imagine_ws_frames": [],
        "all_ws_frames": [],
        "ws_connections": [],
        "http_posts": [],
        "chat_trigger": None,
        "notes": [],
    }

    def flush() -> None:
        out_path.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")

    ws_seen: set[str] = set()

    def record_ws(ws, event: str, data: str | bytes | None) -> None:
        text = data if isinstance(data, str) else (data.decode("utf-8", "replace") if data else "")
        preview = clip_text(text, 16000)
        key = f"{event}:{ws.url}:{hash(preview) & 0xFFFFFFFF:08x}"
        if key in ws_seen:
            return
        ws_seen.add(key)
        row = {
            "t": int(time.time() * 1000),
            "event": event,
            "url": ws.url[:400],
            "len": len(text),
            "preview": preview,
            "is_imagine": IMAGINE_WS in ws.url,
        }
        out["all_ws_frames"].append(row)
        if row["is_imagine"]:
            out["imagine_ws_frames"].append(row)
            tag = "SEND" if event == "framesent" else "RECV"
            print(f">>> IMAGINE WS {tag} len={len(text)} url={ws.url[:80]}", flush=True)
        flush()

    def attach_ws(ws) -> None:
        out["ws_connections"].append({"t": int(time.time() * 1000), "url": ws.url[:400]})
        print(f">>> WS OPEN {ws.url[:120]}", flush=True)
        ws.on("framesent", lambda p: record_ws(ws, "framesent", p))
        ws.on("framereceived", lambda p: record_ws(ws, "framereceived", p))
        flush()

    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not args.headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(proxy={"server": proxy}, user_agent=UA, viewport={"width": 1400, "height": 900})
        ctx.add_init_script(TURBOPACK_HOOK)
        ctx.add_init_script(CAPTURE_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        ctx.on("websocket", attach_ws)

        page = ctx.new_page()
        page.on("websocket", attach_ws)

        def on_req(req):
            if req.method.upper() != "POST":
                return
            path = urlparse(req.url).path
            if "/rest/app-chat/" not in path:
                return
            out["http_posts"].append(
                {
                    "t": int(time.time() * 1000),
                    "path": path,
                    "sig_len": len(req.headers.get("x-statsig-id") or ""),
                    "post_preview": clip_text(req.post_data or "", 2000),
                }
            )
            flush()

        page.on("request", on_req)

        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(4000)
        out["notes"].extend(dismiss_age_gate(page))
        flush()

        # 预热：modes + rate-limits（让站点 JS 完成初始化）
        page.evaluate(
            """async () => {
              try { await fetch('/rest/modes', {credentials:'include'}); } catch(e) {}
              try {
                const r = await fetch('/rest/rate-limits', {method:'POST', headers:{'content-type':'application/json'}, body:'{}', credentials:'include'});
                return {rate_http: r.status, rate_body: await r.text()};
              } catch(e) { return {error: String(e)}; }
            }"""
        )
        page.wait_for_timeout(2000)

        trigger: dict
        if args.hybrid_http and keys_path.exists():
            import threading

            from grok_pure_http_client import GrokPureHttpClient

            keys = json.loads(keys_path.read_text(encoding="utf-8"))
            keys["email"] = args.email
            client = GrokPureHttpClient(keys, signer="auto")
            http_result: dict = {}

            def fire_http() -> None:
                r = client.request("POST", "/rest/app-chat/conversations/new", json_body=payload)
                http_result["http"] = r.status_code
                http_result["body_preview"] = (r.text or "")[:4000]
                http_result["len"] = len(r.text or "")
                http_result["via"] = "pure_http"

            t = threading.Thread(target=fire_http, daemon=True)
            t.start()
            trigger = {"via": "pure_http_pending"}
            out["notes"].append("hybrid_http_trigger")
            flush()
        else:
            trigger = page.evaluate(
                """async (body) => {
                  const r = await fetch('/rest/app-chat/conversations/new', {
                    method: 'POST',
                    headers: {'content-type': 'application/json'},
                    body: JSON.stringify(body),
                    credentials: 'include',
                  });
                  const text = await r.text();
                  return {http: r.status, body_preview: text.slice(0, 4000), len: text.length, via: 'browser_fetch'};
                }""",
                payload,
            )
            http_result = {}
            t = None

        if t is not None:
            t.join(timeout=60)
            trigger = http_result or {"error": "http_thread_timeout"}

        out["chat_trigger"] = trigger
        print(json.dumps({"event": "chat_trigger", **{k: trigger.get(k) for k in ("http", "len")}}, ensure_ascii=False), flush=True)
        flush()

        deadline = time.time() + args.timeout
        last_n = 0
        while time.time() < deadline:
            n = len(out["imagine_ws_frames"])
            if n > last_n:
                last_n = n
            if n >= 3 and any(f["event"] == "framereceived" for f in out["imagine_ws_frames"]):
                out["notes"].append("captured_imagine_frames")
                break
            if trigger.get("http") == 429:
                out["notes"].append("quota_exhausted_429")
                break
            page.wait_for_timeout(1000)

        out["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        out["summary"] = {
            "n_imagine_frames": len(out["imagine_ws_frames"]),
            "n_ws_connections": len(out["ws_connections"]),
            "n_http_posts": len(out["http_posts"]),
            "chat_http": trigger.get("http"),
        }
        flush()
        browser.close()

    summary = out["summary"]
    print(json.dumps({"ok": summary["n_imagine_frames"] > 0, **summary, "json": str(out_path)}, ensure_ascii=False, indent=2))
    return 0 if summary["n_imagine_frames"] > 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
