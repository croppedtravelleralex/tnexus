#!/usr/bin/env python3
"""Grok mgw WebSocket chat probe — hybrid Playwright (castle) + Python WS client."""
from __future__ import annotations

import argparse
import asyncio
import importlib.util
import json
import re
import sys
import time
import uuid
from pathlib import Path
from typing import Any

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
CANARY = Path(r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py")
REPORT_DIR = GROK_ROOT / "reports"

CASTLE_JS = r"""
async () => {
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  for (let i = 0; i < 80; i++) {
    const candidates = [
      window.Castle,
      window.castle,
      globalThis.Castle,
      globalThis.castle,
    ].filter(Boolean);
    for (const c of candidates) {
      for (const fn of ['createRequestToken', 'createRequestHeader', 'getRequestToken', 'requestToken']) {
        if (typeof c[fn] === 'function') {
          try {
            const tok = await c[fn]();
            if (tok && String(tok).length > 20) return { via: 'Castle.' + fn, token: String(tok) };
          } catch (e) {}
        }
      }
    }
    try {
      const imp = globalThis.__turbopack_load__;
      if (typeof imp === 'function') {
        const mod = await imp(6037942);
        const sdk = mod?.default ?? mod;
        if (sdk?.configure && sdk?.createRequestToken) {
          const inst = sdk.configure({ pk: document.querySelector('meta[name="castle-pk"]')?.content || '' });
          const tok = await inst.createRequestToken();
          if (tok) return { via: 'turbopack.6037942', token: String(tok) };
        }
      }
    } catch (e) {}
    await sleep(300);
  }
  return { error: 'castle_not_found', keys: Object.keys(window).filter(k => /castle|Castle/i.test(k)).slice(0, 20) };
}
"""

UID_JS = r"""
() => {
  const fromLs = localStorage.getItem('mgw_uid') || localStorage.getItem('device_id') || localStorage.getItem('anon_id');
  if (fromLs) return fromLs;
  const m = document.cookie.match(/(?:^|;\s*)(?:xai_anon_id|anon_id)=([^;]+)/);
  if (m) return decodeURIComponent(m[1]);
  return null;
}
"""


def load_canary():
    spec = importlib.util.spec_from_file_location("canary", CANARY)
    mod = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(mod)
    return mod


def load_auth(email: str) -> dict:
    return json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))


def cookie_string(auth: dict) -> str:
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    parts = [f"sso={sso}", f"sso-rw={sso_rw}"]
    for c in auth.get("cookies") or []:
        name = c.get("name")
        val = c.get("value")
        domain = str(c.get("domain") or "")
        if not name or val is None:
            continue
        if "grok.com" in domain or domain in (".x.ai", "x.ai", ".grok.com"):
            parts.append(f"{name}={val}")
    # dedupe keep first
    seen: set[str] = set()
    out: list[str] = []
    for p in parts:
        k = p.split("=", 1)[0]
        if k in seen:
            continue
        seen.add(k)
        out.append(p)
    return "; ".join(out)


def evt_id(prefix: str) -> str:
    return f"{prefix}_{int(time.time() * 1000)}"


def session_create_payload(*, model: str = "fast", conversation_id: str | None = None) -> dict:
    x_grok: dict[str, Any] = {
        "protocol_capabilities": ["conversation_attached", "custom_methods_v1"],
        "use_chunk": True,
        "enable_side_by_side": True,
        "force_side_by_side": False,
        "enable_image_generation": True,
        "image_generation_count": 2,
        "disable_text_follow_ups": False,
        "disable_artifact": True,
        "force_concise": False,
    }
    if conversation_id:
        x_grok["conversation_id"] = conversation_id
    return {
        "event": {
            "type": "session.create",
            "event_id": evt_id("evt_init"),
            "session": {"model": model, "x_grok": x_grok},
        }
    }


def item_create_payload(session_id: str, message: str, *, parent_response_id: str | None = None) -> dict:
    item: dict[str, Any] = {
        "type": "message",
        "role": "user",
        "x_grok": {
            "client_message_id": str(uuid.uuid4()),
            "input_chunks": [{"text": {"text": message}}],
        },
    }
    if parent_response_id:
        item["parent_response_id"] = parent_response_id
    return {
        "session_id": session_id,
        "event": {
            "type": "conversation.item.create",
            "event_id": evt_id("evt_msg"),
            "item": item,
        },
    }


def response_create_payload(session_id: str, castle_token: str) -> dict:
    return {
        "session_id": session_id,
        "event": {
            "type": "response.create",
            "event_id": evt_id("evt_resp"),
            "castle_request_token": castle_token,
        },
    }


def extract_text_from_event(data: dict) -> str:
    ev = data.get("event") or {}
    et = ev.get("type") or ""
    if et == "response.chunk":
        chunk = ev.get("chunk") or {}
        text = chunk.get("text") or {}
        if isinstance(text, dict) and text.get("text"):
            return str(text["text"])
    if et == "response.output_text.done":
        return str(ev.get("text") or "")
    return ""


def fetch_castle_and_uid(email: str, *, headed: bool) -> dict:
    canary = load_canary()
    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)

    from playwright.sync_api import sync_playwright

    out: dict[str, Any] = {"email": email, "notes": []}
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=not headed)
        ctx = browser.new_context(
            proxy={"server": canary.PROXY},
            user_agent=canary.UA,
            viewport={"width": 1400, "height": 900},
        )
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = ctx.new_page()
        captured_castle: list[str] = []

        def on_ws(ws):
            def on_sent(payload):
                try:
                    text = payload if isinstance(payload, str) else payload.decode("utf-8", "replace")
                    if "castle_request_token" in text:
                        m = re.search(r'"castle_request_token":"([^"]+)"', text)
                        if m:
                            captured_castle.append(m.group(1))
                except Exception:
                    pass

            ws.on("framesent", on_sent)

        ctx.on("websocket", on_ws)
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(8000)

        castle = page.evaluate(CASTLE_JS)
        uid = page.evaluate(UID_JS) or str(uuid.uuid4())
        out["castle"] = castle
        out["uid"] = uid
        out["captured_castle_from_ws"] = captured_castle[:1]
        browser.close()
    return out


async def ws_chat(
    *,
    cookie: str,
    uid: str,
    castle_token: str,
    message: str,
    model: str,
    timeout_s: float,
    proxy: str | None,
) -> dict:
    import websockets

    ws_url = f"wss://grok.com/ws/mgw/?uid={uid}"
    headers = {
        "Cookie": cookie,
        "Origin": "https://grok.com",
        "User-Agent": (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
            "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
        ),
    }

    texts: list[str] = []
    events: list[str] = []
    session_id: str | None = None
    parent_response_id: str | None = None
    err: str | None = None
    sent = False

    connect_kw: dict[str, Any] = dict(
        additional_headers=headers,
        open_timeout=30,
        close_timeout=5,
        max_size=8 * 1024 * 1024,
    )
    if proxy:
        connect_kw["proxy"] = proxy

    try:
        async with websockets.connect(ws_url, **connect_kw) as ws:
            await ws.send(json.dumps(session_create_payload(model=model)))

            deadline = time.time() + timeout_s
            while time.time() < deadline:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=min(15, deadline - time.time()))
                except asyncio.TimeoutError:
                    break
                data = json.loads(raw)
                ev = (data.get("event") or {}).get("type") or ""
                events.append(ev)

                if ev == "error" or data.get("error"):
                    err = raw[:500]
                    break

                if ev == "session.created":
                    session_id = data.get("session_id")
                    client_evt = (data.get("event") or {}).get("client_event_id") or ""
                    if client_evt.startswith("evt_init") and session_id and not sent:
                        sent = True
                        msg_evt = item_create_payload(session_id, message, parent_response_id=parent_response_id)
                        await ws.send(json.dumps(msg_evt))
                        await ws.send(json.dumps(response_create_payload(session_id, castle_token)))
                    continue

                if ev == "response.created":
                    resp = (data.get("event") or {}).get("response") or {}
                    parent_response_id = resp.get("id") or parent_response_id

                piece = extract_text_from_event(data)
                if piece:
                    texts.append(piece)

                if ev == "response.done":
                    break

    except Exception as exc:
        err = f"{type(exc).__name__}: {exc}"

    return {
        "ws_url": ws_url,
        "session_id": session_id,
        "reply": "".join(texts),
        "reply_len": len("".join(texts)),
        "events": events[-30:],
        "error": err,
        "ok": bool("".join(texts)) and not err,
    }


def playwright_send_chat(email: str, message: str, *, headed: bool, timeout_s: float) -> dict:
    """Send via Grok UI; capture reply from existing mgw WebSocket (no second WS)."""
    canary = load_canary()
    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)

    from playwright.sync_api import sync_playwright

    texts: list[str] = []
    events: list[str] = []
    done = False

    def on_frame(payload: str | bytes, *, direction: str) -> None:
        nonlocal done
        try:
            raw = payload if isinstance(payload, str) else payload.decode("utf-8", "replace")
            data = json.loads(raw)
            ev = (data.get("event") or {}).get("type") or ""
            if direction == "recv":
                events.append(ev)
                piece = extract_text_from_event(data)
                if piece:
                    texts.append(piece)
                if ev == "response.done":
                    done = True
        except Exception:
            pass

    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(
            proxy={"server": canary.PROXY},
            user_agent=canary.UA,
            viewport={"width": 1400, "height": 900},
        )
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )

        def attach_ws(ws) -> None:
            ws.on("framereceived", lambda p: on_frame(p, direction="recv"))
            ws.on("framesent", lambda p: on_frame(p, direction="sent"))

        ctx.on("websocket", attach_ws)

        page = ctx.new_page()
        page.on("websocket", attach_ws)
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(4000)

        for name in ("继续", "Continue", "保存", "Save"):
            try:
                btn = page.get_by_role("button", name=name)
                if btn.count() and btn.first.is_visible():
                    btn.first.click(timeout=3000, force=True)
                    page.wait_for_timeout(1500)
                    break
            except Exception:
                pass

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
        if not done:
            page.keyboard.press("Enter")

        deadline = time.time() + timeout_s
        while time.time() < deadline and not done:
            page.wait_for_timeout(400)

        browser.close()

    reply = "".join(texts)
    return {
        "reply": reply,
        "reply_len": len(reply),
        "events_tail": events[-40:],
        "ok": bool(reply.strip()),
        "error": None if reply.strip() else "no_reply_text",
    }


def browser_ws_chat(email: str, message: str, *, headed: bool, timeout_s: float) -> dict:
    """Legacy: open a fresh WebSocket in-page (conflicts if page already has mgw)."""
    canary = load_canary()
    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)

    from playwright.sync_api import sync_playwright

    js = f"""
    async (message) => {{
      const sleep = ms => new Promise(r => setTimeout(r, ms));
      const evtId = p => p + '_' + Date.now();
      const getCastle = async () => {{
        for (let i = 0; i < 60; i++) {{
          const c = window.Castle || window.castle;
          if (c && typeof c.createRequestToken === 'function') {{
            const t = await c.createRequestToken();
            if (t) return String(t);
          }}
          await sleep(200);
        }}
        throw new Error('castle_not_ready');
      }};
      const uid = (() => {{
        const m = document.cookie.match(/(?:^|;\\s*)xai_anon_id=([^;]+)/);
        if (m) return decodeURIComponent(m[1]);
        return crypto.randomUUID();
      }})();
      const ws = new WebSocket('wss://grok.com/ws/mgw/?uid=' + uid);
      const events = [];
      const texts = [];
      let sessionId = null;
      let done = false;
      let err = null;
      await new Promise((resolve, reject) => {{
        const t = setTimeout(() => reject(new Error('ws_open_timeout')), 20000);
        ws.onopen = () => {{ clearTimeout(t); resolve(); }};
        ws.onerror = () => {{ clearTimeout(t); reject(new Error('ws_error')); }};
      }});
      let sent = false;
      const maybeSend = () => {{
        if (!sessionId || sent) return;
        sent = true;
        ws.send(JSON.stringify({{
          session_id: sessionId,
          event: {{
            type: 'conversation.item.create',
            event_id: evtId('evt_msg'),
            item: {{
              type: 'message', role: 'user',
              x_grok: {{ client_message_id: crypto.randomUUID(), input_chunks: [{{ text: {{ text: message }} }}] }},
            }},
          }},
        }}));
        getCastle().then(tok => {{
          ws.send(JSON.stringify({{
            session_id: sessionId,
            event: {{ type: 'response.create', event_id: evtId('evt_resp'), castle_request_token: tok }},
          }}));
        }}).catch(e => {{ err = String(e); done = true; ws.close(); }});
      }};
      ws.onmessage = (ev) => {{
        try {{
          const data = JSON.parse(ev.data);
          const type = data?.event?.type || '';
          events.push(type);
          if (type === 'session.created' || type === 'session.moved') {{
            sessionId = data.session_id;
            maybeSend();
          }}
          if (type === 'conversation.attached') {{
            sessionId = data.session_id || sessionId;
            maybeSend();
          }}
          if (type === 'response.chunk') {{
            const t = data?.event?.chunk?.text?.text;
            if (t) texts.push(t);
          }}
          if (type === 'response.done') {{ done = true; ws.close(); }}
          if (type === 'error') {{ err = ev.data.slice(0, 400); done = true; ws.close(); }}
        }} catch (e) {{ err = String(e); done = true; }}
      }};
      ws.send(JSON.stringify({{
        event: {{
          type: 'session.create',
          event_id: evtId('evt_init'),
          session: {{
            model: 'fast',
            x_grok: {{
              protocol_capabilities: ['conversation_attached', 'custom_methods_v1'],
              use_chunk: true, enable_side_by_side: true, force_side_by_side: false,
              enable_image_generation: true, image_generation_count: 2,
              disable_text_follow_ups: false, disable_artifact: true, force_concise: false,
            }},
          }},
        }},
      }}));
      const deadline = Date.now() + {int(timeout_s * 1000)};
      while (!done && Date.now() < deadline) await sleep(200);
      if (!done) err = err || 'timeout';
      return {{ uid, sessionId, events: events.slice(-40), reply: texts.join(''), error: err, ok: texts.join('').length > 0 && !err }};
    }}
    """

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=not headed)
        ctx = browser.new_context(
            proxy={"server": canary.PROXY},
            user_agent=canary.UA,
            viewport={"width": 1400, "height": 900},
        )
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = ctx.new_page()
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(5000)
        result = page.evaluate(js, message)
        browser.close()
        return result


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--message", default="Reply with exactly: PONG")
    ap.add_argument("--mode", choices=("ui", "browser", "hybrid"), default="ui")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--timeout", type=float, default=90)
    ap.add_argument("--out", type=Path, default=REPORT_DIR / "grok_ws_probe.json")
    args = ap.parse_args()

    if args.mode == "ui":
        result = playwright_send_chat(args.email, args.message, headed=args.headed, timeout_s=args.timeout)
        row = {"mode": "ui", "email": args.email, "message": args.message, **result}
    elif args.mode == "browser":
        result = browser_ws_chat(args.email, args.message, headed=args.headed, timeout_s=args.timeout)
        row = {"mode": "browser", "email": args.email, "message": args.message, **result}
    else:
        prep = fetch_castle_and_uid(args.email, headed=args.headed)
        token = None
        if isinstance(prep.get("castle"), dict) and prep["castle"].get("token"):
            token = prep["castle"]["token"]
        elif prep.get("captured_castle_from_ws"):
            token = prep["captured_castle_from_ws"][0]
        if not token:
            row = {"mode": "hybrid", "ok": False, "error": "no_castle_token", "prep": prep}
        else:
            canary = load_canary()
            auth = load_auth(args.email)
            chat = asyncio.run(
                ws_chat(
                    cookie=cookie_string(auth),
                    uid=str(prep.get("uid") or uuid.uuid4()),
                    castle_token=token,
                    message=args.message,
                    model="fast",
                    timeout_s=args.timeout,
                    proxy=canary.PROXY,
                )
            )
            row = {"mode": "hybrid", "email": args.email, "message": args.message, "prep": prep, **chat}

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(row, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(row, ensure_ascii=False, indent=2))
    return 0 if row.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
