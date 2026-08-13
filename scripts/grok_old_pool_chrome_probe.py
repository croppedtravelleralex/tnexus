#!/usr/bin/env python3
"""Inject grok2api old-pool SSO into local Chrome and probe web chat (HTTP + WS)."""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
from pathlib import Path

CHAT_HTTP = "/rest/app-chat/conversations/new"
WS_HINT = "/ws/mgw/"


def run_probe(auth: dict, *, headed: bool, timeout_s: int, json_out: Path | None) -> dict:
    from playwright.sync_api import sync_playwright

    sso = str(auth.get("sso") or "").strip()
    account_id = auth.get("account_id")
    identity = auth.get("identity_key") or auth.get("email") or ""
    message = str(auth.get("message") or "Reply with exactly: PONG")
    proxy = str(auth.get("proxy") or "http://127.0.0.1:7897")

    result: dict = {
        "account_id": account_id,
        "identity_key": identity,
        "headed": headed,
        "proxy": proxy,
        "ui_send": False,
        "ui_login_wall": False,
        "ui_reply_seen": False,
        "http_chat": [],
        "ws_events": [],
        "ws_reply": "",
        "final_url": "",
        "error": None,
        "ok": False,
    }
    ws_done = {"flag": False}

    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        context = browser.new_context(
            proxy={"server": proxy},
            viewport={"width": 1400, "height": 900},
            user_agent=(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
                "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
            ),
        )
        context.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = context.new_page()

        def on_request(req) -> None:
            url = req.url or ""
            if CHAT_HTTP in url:
                result["http_chat"].append(
                    {
                        "phase": "request",
                        "method": req.method,
                        "url": url,
                        "sig_len": len(req.headers.get("x-statsig-id", "")),
                    }
                )

        def on_response(resp) -> None:
            url = resp.url or ""
            if CHAT_HTTP in url:
                body = ""
                try:
                    body = resp.text()[:300]
                except Exception:
                    body = ""
                result["http_chat"].append(
                    {
                        "phase": "response",
                        "status": resp.status,
                        "url": url,
                        "body_snip": body.replace("\n", " "),
                    }
                )

        def on_ws(ws) -> None:
            if WS_HINT not in (ws.url or ""):
                return

            def on_frame(payload, *, direction: str) -> None:
                try:
                    raw = payload if isinstance(payload, str) else payload.decode("utf-8", "replace")
                    data = json.loads(raw)
                    ev = (data.get("event") or {}).get("type") or ""
                    if direction == "recv":
                        result["ws_events"].append(ev)
                        piece = ""
                        if isinstance(data.get("event"), dict):
                            piece = str(data["event"].get("delta") or data["event"].get("text") or "")
                        if piece:
                            result["ws_reply"] += piece
                        if ev == "response.done":
                            ws_done["flag"] = True
                    if re.search(r"\bPONG\b", raw, re.I):
                        result["ui_reply_seen"] = True
                except Exception:
                    if isinstance(payload, str) and re.search(r"\bPONG\b", payload, re.I):
                        result["ui_reply_seen"] = True

            ws.on("framereceived", lambda p: on_frame(p, direction="recv"))
            ws.on("framesent", lambda p: on_frame(p, direction="sent"))

        page.on("request", on_request)
        page.on("response", on_response)
        page.on("websocket", on_ws)
        context.on("websocket", on_ws)

        try:
            page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
            page.wait_for_timeout(4000)
            for name in ("继续", "Continue", "保存", "Save", "忽略", "Skip"):
                try:
                    btn = page.get_by_role("button", name=name)
                    if btn.count() and btn.first.is_visible():
                        btn.first.click(timeout=3000, force=True)
                        page.wait_for_timeout(1500)
                        break
                except Exception:
                    pass

            page.goto("https://grok.com/chat", wait_until="domcontentloaded", timeout=90000)
            page.wait_for_timeout(3000)
            result["final_url"] = page.url
            html = page.content()
            if re.search(r"sign in|log in|登录", html, re.I):
                result["ui_login_wall"] = True

            for label in ("新建聊天", "New chat"):
                try:
                    btn = page.get_by_role("button", name=label)
                    if btn.count() and btn.first.is_visible():
                        btn.first.click(timeout=3000)
                        page.wait_for_timeout(1500)
                        break
                except Exception:
                    pass

            editor = page.locator('[contenteditable="true"]').last
            editor.click(timeout=15000)
            page.keyboard.type(message, delay=20)
            page.wait_for_timeout(500)
            page.keyboard.press("Control+Enter")
            page.keyboard.press("Enter")
            result["ui_send"] = True

            deadline = time.time() + max(30, timeout_s)
            while time.time() < deadline:
                if ws_done["flag"] or result["ui_reply_seen"]:
                    break
                if any(
                    int(x.get("status") or 0) == 200
                    for x in result["http_chat"]
                    if x.get("phase") == "response"
                ):
                    break
                page.wait_for_timeout(400)

            if not result["ui_reply_seen"] and re.search(r"\bPONG\b", result["ws_reply"], re.I):
                result["ui_reply_seen"] = True
            if not result["ui_reply_seen"]:
                html = page.content()
                if re.search(r"\bPONG\b", html, re.I):
                    result["ui_reply_seen"] = True
                if "anti-bot" in html.lower():
                    result["ui_anti_bot"] = True
        except Exception as exc:
            result["error"] = f"{type(exc).__name__}: {exc}"
        finally:
            browser.close()

    result["ok"] = result["ui_reply_seen"] or bool(
        result["ws_reply"].strip()
    ) or any(
        int(x.get("status") or 0) == 200
        for x in result["http_chat"]
        if x.get("phase") == "response"
    )
    text = json.dumps(result, ensure_ascii=False, indent=2)
    print(text)
    if json_out:
        json_out.parent.mkdir(parents=True, exist_ok=True)
        json_out.write_text(text, encoding="utf-8")
    return result


def main() -> int:
    ap = argparse.ArgumentParser(description="Old grok2api pool SSO → local Chrome chat probe")
    ap.add_argument("--auth-json", required=True, help="JSON with sso (+ optional account_id)")
    ap.add_argument("--headed", action="store_true", help="visible Chrome window")
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--json-out", default="")
    args = ap.parse_args()

    auth_path = Path(args.auth_json)
    auth = json.loads(auth_path.read_text(encoding="utf-8"))
    out = Path(args.json_out) if args.json_out.strip() else None
    result = run_probe(auth, headed=args.headed, timeout_s=args.timeout, json_out=out)
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
