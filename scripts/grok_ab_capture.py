#!/usr/bin/env python3
"""Browser fetch POST chat with auto x-statsig-id + node bundle A/B."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
CANARY = Path(r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py")
BUNDLE = ROOT / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js"
MODULE = BUNDLE.parent / "grok_sign_module_1645e3.js"
CHAT_PATH = "/rest/app-chat/conversations/new"


def load_canary():
    spec = importlib.util.spec_from_file_location("canary", CANARY)
    mod = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(mod)
    return mod


def load_auth(email: str) -> dict:
    return json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))


def node_sign(meta: str, path: str, method: str) -> str | None:
    js = (
        BUNDLE.read_text(encoding="utf-8")
        .replace("__GROK_META__", meta)
        .replace("__SIGN_PATH__", path)
        .replace("__SIGN_METHOD__", method)
    )
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
        tmp.write(js)
        p = tmp.name
    try:
        env = {**os.environ, "GROK_SIGN_MODULE": str(MODULE)}
        proc = subprocess.run(["node", p], capture_output=True, text=True, timeout=60, env=env)
        for line in proc.stdout.splitlines():
            if line.startswith("FULLSIG "):
                return line.split(" ", 2)[2].strip()
        if proc.returncode != 0:
            print(proc.stderr[-600:], file=sys.stderr)
    finally:
        os.unlink(p)
    return None


def browser_fetch_chat(email: str, *, headed: bool, timeout_s: int) -> dict:
    canary = load_canary()
    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    payload = canary.chat_payload("Reply with exactly: PONG")
    out: dict = {"email": email, "notes": []}

    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=not headed)
        ctx = browser.new_context(
            proxy={"server": canary.PROXY},
            user_agent=canary.UA,
            viewport={"width": 1400, "height": 900},
        )
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_init_script(canary.CAPTURE_HOOK)
        page = ctx.new_page()
        captured: dict = {}

        def on_req(req):
            if CHAT_PATH not in req.url:
                return
            captured["sig"] = req.headers.get("x-statsig-id") or ""
            captured["method"] = req.method

        page.on("request", on_req)
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=60000)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        # SPA keeps long-polling; networkidle often never settles.
        page.wait_for_load_state("load", timeout=30000)
        page.wait_for_timeout(5000)
        out["notes"].extend(canary._dismiss_age_gate(page))
        out["meta"] = page.evaluate(
            """() => {
              const el = document.querySelector('meta[name^=\"gr\"], meta[name*=\"grok-site\"]');
              return el ? el.getAttribute('content') : null;
            }"""
        )

        # In-page fetch: SPA fetch wrapper should attach path-bound x-statsig-id.
        result = page.evaluate(
            """async (payload) => {
              const path = '/rest/app-chat/conversations/new';
              const body = JSON.stringify(payload);
              try {
                const r = await fetch('https://grok.com' + path, {
                  method: 'POST',
                  credentials: 'include',
                  headers: {
                    'content-type': 'application/json',
                    'accept': 'application/json',
                  },
                  body,
                });
                const text = await r.text();
                return {http: r.status, body: text.slice(0, 500)};
              } catch (e) {
                return {error: String(e && e.message || e)};
              }
            }""",
            payload,
        )
        out["browser_fetch"] = result
        out["browser_post_sig"] = captured.get("sig")
        out["browser_post_sig_len"] = len(captured.get("sig") or "")
        browser.close()
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="kevinthomas8oqg@yumail.co")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--json-out", type=Path, default=GROK_ROOT / "reports" / "grok_ab_capture.json")
    args = ap.parse_args()

    canary = load_canary()
    auth = load_auth(args.email)
    sso = str(auth["sso"])

    row = browser_fetch_chat(args.email, headed=args.headed, timeout_s=90)
    meta = row.get("meta") or ""
    row["node_sig"] = node_sign(meta, CHAT_PATH, "POST") if meta else None
    row["node_sig_len"] = len(row.get("node_sig") or "")

    ab: dict = {}
    if row.get("browser_post_sig"):
        ab["browser_sig_curl"] = canary.post_rest(sso, row["browser_post_sig"], canary.chat_payload("Reply with exactly: PONG"))
    if row.get("node_sig"):
        ab["node_sig_curl"] = canary.post_rest(sso, row["node_sig"], canary.chat_payload("Reply with exactly: PONG"))
    row["ab"] = {
        k: {"http": v.get("http"), "kind": v.get("kind"), "body": str(v.get("body", ""))[:200]}
        for k, v in ab.items()
    }
    row["ok"] = row.get("browser_fetch", {}).get("http") == 200 or any(
        v.get("http") == 200 for v in row["ab"].values()
    )

    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(row, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"ok": row["ok"], "browser_fetch": row.get("browser_fetch"), "ab": row["ab"]}, ensure_ascii=False))
    return 0 if row["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
