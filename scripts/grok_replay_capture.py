#!/usr/bin/env python3
"""Replay captured browser sigs vs node sig on any /rest/ path (load-responses, etc.)."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

from curl_cffi import requests as crequests

ROOT = Path(__file__).resolve().parents[1]
GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
CANARY = Path(r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py")
BUNDLE = ROOT / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js"
MODULE = BUNDLE.parent / "grok_sign_module_1645e3.js"


def load_canary():
    spec = importlib.util.spec_from_file_location("canary", CANARY)
    mod = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(mod)
    return mod


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
            print(proc.stderr[-800:], file=sys.stderr)
    finally:
        os.unlink(p)
    return None


def post_path(
    canary,
    sso: str,
    path: str,
    method: str,
    statsig: str | None,
    payload: dict | None,
    *,
    referer: str | None = None,
) -> dict:
    url = "https://grok.com" + path
    headers = {
        "accept": "*/*",
        "content-type": "application/json",
        "origin": "https://grok.com",
        "referer": referer or "https://grok.com/",
        "user-agent": canary.UA,
        "cookie": canary.cookie_header(sso),
        "cache-control": "no-cache",
        "pragma": "no-cache",
        "sec-fetch-dest": "empty",
        "sec-fetch-mode": "cors",
        "sec-fetch-site": "same-origin",
        "x-xai-request-id": str(uuid.uuid4()),
    }
    if statsig:
        headers["x-statsig-id"] = statsig
    t0 = time.time()
    if method.upper() == "GET":
        r = crequests.get(
            url,
            headers=headers,
            impersonate=canary.IMPERSONATE,
            proxies=canary.PROXIES,
            timeout=90,
        )
        body = r.text[:2000]
    else:
        r = crequests.post(
            url,
            headers=headers,
            json=payload or {},
            impersonate=canary.IMPERSONATE,
            proxies=canary.PROXIES,
            timeout=90,
        )
        body = r.text[:2000]
    kind = canary.classify_body(r.status_code, body, r.headers.get("cf-mitigated"))
    return {
        "http": r.status_code,
        "cf": r.headers.get("cf-mitigated"),
        "elapsed_s": round(time.time() - t0, 2),
        "kind": kind,
        "body_prefix": body[:240].replace("\n", " "),
    }


def pick_posts(cap: dict) -> list[dict]:
    posts = list(cap.get("app_chat_posts") or [])
    if not posts:
        posts = [
            r
            for r in cap.get("requests") or []
            if r.get("method", "").upper() == "POST"
            and "/rest/app-chat/" in r.get("path", "")
            and r.get("sig")
        ]
    # legacy single chat_post
    if cap.get("chat_post") and cap["chat_post"].get("sig"):
        posts.insert(0, cap["chat_post"])
    return posts


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--capture", type=Path, default=GROK_ROOT / "reports" / "grok_manual_capture.json")
    ap.add_argument("--email", default=None, help="defaults to capture email")
    ap.add_argument("--index", type=int, default=0, help="which app_chat_post to replay")
    ap.add_argument("--all", action="store_true", help="replay all captured app-chat POSTs")
    args = ap.parse_args()

    canary = load_canary()
    cap = json.loads(args.capture.read_text(encoding="utf-8"))
    email = args.email or cap.get("email") or "kevinthomas8oqg@yumail.co"
    auth = json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))
    sso = str(auth["sso"])
    meta = cap.get("meta") or ""

    posts = pick_posts(cap)
    if not posts:
        print("no app-chat POST sigs in capture", file=sys.stderr)
        return 1

    indices = range(len(posts)) if args.all else [args.index]
    results: list[dict] = []

    for i in indices:
        row_in = posts[i]
        path = row_in["path"]
        method = row_in.get("method", "POST")
        browser_sig = row_in["sig"]
        referer = (row_in.get("headers") or {}).get("referer")
        payload = {}
        if row_in.get("post_data"):
            try:
                payload = json.loads(row_in["post_data"])
            except json.JSONDecodeError:
                payload = {}

        node_sig = node_sign(meta, path, method) if meta else None
        ab = {
            "browser_sig": post_path(
                canary, sso, path, method, browser_sig, payload, referer=referer
            ),
            "node_sig": (
                post_path(canary, sso, path, method, node_sig, payload, referer=referer)
                if node_sig
                else {"error": "no node sig"}
            ),
        }
        results.append(
            {
                "index": i,
                "path": path,
                "method": method,
                "browser_sig_len": len(browser_sig),
                "node_sig_len": len(node_sig or ""),
                "browser_sig_prefix": browser_sig[:32],
                "node_sig_prefix": (node_sig or "")[:32],
                "payload_keys": list(payload.keys()) if isinstance(payload, dict) else [],
                "ab": ab,
                "ok": any(v.get("http") == 200 for v in ab.values() if isinstance(v, dict)),
            }
        )

    out = {
        "email": email,
        "meta_prefix": meta[:48] if meta else "",
        "results": results,
        "any_ok": any(r["ok"] for r in results),
    }
    out_path = args.capture.with_name(args.capture.stem + "_ab.json")
    out_path.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(out, ensure_ascii=False, indent=2))
    return 0 if out["any_ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
