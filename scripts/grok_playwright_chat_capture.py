#!/usr/bin/env python3
"""Playwright: inject SSO, send chat, capture POST x-statsig-id + A/B with node bundle."""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.parse import urlparse

from curl_cffi import requests as cr

ROOT = Path(__file__).resolve().parents[1]
GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
CANARY = Path(r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py")
PROXY = "http://127.0.0.1:7897"
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
)
CHAT_PATH = "/rest/app-chat/conversations/new"
SIGNER_IDS = (4629918, 2347272, 4629917, 4629919, 4629920)


def load_auth(email: str) -> dict:
    path = GROK_ROOT / "web_auths" / f"{email}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def cookie_header(auth: dict) -> str:
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso).strip()
    return f"sso={sso}; sso-rw={sso_rw}"


def import_canary():
    sys.path.insert(0, str(CANARY.parent))
    import importlib.util

    spec = importlib.util.spec_from_file_location("canary", CANARY)
    mod = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(mod)
    return mod


def node_sign(meta: str, path: str, method: str, bundle: Path) -> str | None:
    js = (
        bundle.read_text(encoding="utf-8")
        .replace("__GROK_META__", meta)
        .replace("__SIGN_PATH__", path)
        .replace("__SIGN_METHOD__", method)
    )
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
        tmp.write(js)
        tmp_path = tmp.name
    module = bundle.parent / "grok_sign_module_1645e3.js"
    env = {**os.environ, "GROK_SIGN_MODULE": str(module)}
    try:
        proc = subprocess.run(["node", tmp_path], capture_output=True, text=True, timeout=60, env=env)
        for line in (proc.stdout + proc.stderr).splitlines():
            if line.startswith("FULLSIG "):
                return line.split(" ", 2)[2].strip()
        if proc.returncode != 0:
            print("node_sign_err", proc.stderr[:400], flush=True)
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
    return None


def post_chat(canary, sso: str, sig: str, message: str) -> dict:
    t0 = time.time()
    r = canary.post_rest(sso, sig, canary.chat_payload(message))
    return {
        "http": r.get("http"),
        "kind": r.get("kind"),
        "elapsed_s": round(time.time() - t0, 2),
        "body_snip": str(r.get("body", ""))[:240],
        "sig_len": len(sig),
    }


def send_message_playwright(page, message: str) -> list[str]:
    notes: list[str] = []
    canary = import_canary()
    canary._dismiss_age_gate(page)
    page.wait_for_timeout(1200)

    # Focus composer (ProseMirror / textarea)
    for sel in (
        ".ProseMirror",
        "textarea",
        '[contenteditable="true"]',
        'div[role="textbox"]',
    ):
        loc = page.locator(sel).first
        try:
            if loc.count() == 0:
                continue
            loc.click(timeout=5000)
            loc.fill(message)
            notes.append(f"fill:{sel}")
            break
        except Exception as exc:
            notes.append(f"fill_fail:{sel}:{type(exc).__name__}")

    page.wait_for_timeout(400)

    # Prefer explicit send button near composer (avoid generic submit)
    sent = False
    for sel in (
        'button[aria-label="提交"]',
        'button[aria-label*="Send"]',
        'button[aria-label*="发送"]',
        'button[data-testid*="send"]',
    ):
        loc = page.locator(sel).last
        try:
            if loc.count() and loc.is_visible():
                loc.click(timeout=3000)
                notes.append(f"click:{sel}")
                sent = True
                break
        except Exception:
            pass

    if not sent:
        page.keyboard.press("Control+Enter")
        notes.append("ctrl+enter")
        page.wait_for_timeout(500)
        page.keyboard.press("Enter")
        notes.append("enter_fallback")

    return notes


def run_capture(email: str, *, headed: bool, bundle: Path, timeout_s: int) -> dict:
    canary = import_canary()
    auth = load_auth(email)
    cookie = cookie_header(auth)
    result: dict = {
        "email": email,
        "send_notes": [],
        "captures": [],
        "meta": None,
        "browser_post_sig": None,
        "node_sig": None,
        "ab": {},
    }

    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=not headed)
        context = browser.new_context(
            proxy={"server": PROXY},
            user_agent=UA,
            viewport={"width": 1400, "height": 900},
        )
        context.add_init_script(canary.TURBOPACK_HOOK)
        context.add_init_script(canary.CAPTURE_HOOK)
        page = context.new_page()
        net: list[dict] = []

        def on_req(req):
            if "/rest/" not in req.url:
                return
            sig = req.headers.get("x-statsig-id") or ""
            if sig:
                net.append(
                    {
                        "path": urlparse(req.url).path,
                        "method": req.method,
                        "sig": sig,
                    }
                )

        page.on("request", on_req)
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=60000)
        # SSO cookies
        sso = str(auth.get("sso") or "").strip()
        sso_rw = str(auth.get("sso_rw") or sso)
        context.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=60000)
        page.wait_for_timeout(4000)
        canary._dismiss_age_gate(page)

        # meta
        result["meta"] = page.evaluate(
            """() => {
              const pick = (sel) => {
                const el = document.querySelector(sel);
                return el ? (el.getAttribute('content') || '').trim() : '';
              };
              return pick('meta[name^="gr"]')
                || pick('meta[name*="grok-site"]')
                || pick('meta[name*="verification"]')
                || null;
            }"""
        )

        # turbopack sign attempt
        signed = page.evaluate(
            """async ({path, method, ids}) => {
              if (!globalThis.__grokBridgeRuntime) return {error: 'no_runtime'};
              const runtime = globalThis.__grokBridgeRuntime;
              const errs = [];
              for (const id of ids) {
                try {
                  const mod = await runtime.A(id);
                  if (!mod || typeof mod.default !== 'function') { errs.push(id+':no-default'); continue; }
                  const signer = mod.default();
                  const sig = String(await signer(path, method) || '');
                  if (sig && sig.length > 20 && !sig.startsWith('x0:') && !sig.startsWith('eDA6')) {
                    return {statsigId: sig, moduleId: id};
                  }
                  errs.push(id+':bad:'+sig.slice(0,24));
                } catch (e) {
                  errs.push(id+':'+String(e && e.message || e).slice(0,80));
                }
              }
              return {error: 'no_sig', errs: errs.slice(0, 12)};
            }""",
            {"path": CHAT_PATH, "method": "POST", "ids": list(SIGNER_IDS)},
        )
        result["turbopack_sign"] = signed

        # send + wait for POST
        post_holder: dict = {}

        def on_resp(resp):
            if CHAT_PATH not in resp.url:
                return
            try:
                req = resp.request
                sig = req.headers.get("x-statsig-id") or ""
                post_holder["status"] = resp.status
                post_holder["sig"] = sig
                post_holder["body_snip"] = (resp.text() or "")[:300]
            except Exception:
                pass

        page.on("response", on_resp)
        result["send_notes"] = send_message_playwright(page, "Reply with exactly: PONG")
        deadline = time.time() + min(timeout_s, 45)
        while time.time() < deadline and "sig" not in post_holder:
            page.wait_for_timeout(500)

        js_caps = page.evaluate("() => (globalThis.__grokCapturedSigs || []).slice(-40)") or []
        result["captures"] = js_caps + net
        if post_holder.get("sig"):
            result["browser_post_sig"] = post_holder["sig"]
            result["browser_post_http"] = post_holder.get("status")
            result["browser_post_body"] = post_holder.get("body_snip", "")[:240]
        else:
            for item in reversed(result["captures"]):
                if item.get("path") == CHAT_PATH and item.get("method", "").upper() == "POST":
                    result["browser_post_sig"] = item.get("sig")
                    break

        browser.close()

    if result.get("meta"):
        result["node_sig"] = node_sign(result["meta"], CHAT_PATH, "POST", bundle)

    sso = str(auth.get("sso") or "").strip()
    if result.get("browser_post_sig"):
        result["ab"]["browser_post"] = post_chat(canary, sso, result["browser_post_sig"], "Reply with exactly: PONG")
    if result.get("node_sig"):
        result["ab"]["node_post"] = post_chat(canary, sso, result["node_sig"], "Reply with exactly: PONG")

    result["ok"] = any(
        v.get("http") == 200 and v.get("kind") == "chat_ok" for v in result.get("ab", {}).values()
    )
    return result


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="kevinthomas8oqg@yumail.co")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--json-out", type=Path, default=GROK_ROOT / "reports" / "playwright_chat_capture.json")
    ap.add_argument("--bundle", type=Path, default=ROOT / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js")
    args = ap.parse_args()

    out = run_capture(args.email, headed=args.headed, bundle=args.bundle, timeout_s=args.timeout)
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({k: out[k] for k in ("ok", "browser_post_http", "ab", "turbopack_sign") if k in out}, ensure_ascii=False))
    return 0 if out.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
