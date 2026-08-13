#!/usr/bin/env python3
"""webshare 代理 + Playwright Chromium 自动化登录/额度/对话探测。

读取：
  C:\\Users\\Lenovo\\Downloads\\Webshare 100 proxies (1).txt  （机房 dc）
  C:\\Users\\Lenovo\\Downloads\\Webshare 20 proxies.txt       （住宅 res）

用法：
  python scripts/grok_webshare_browser_probe.py --list
  python scripts/grok_webshare_browser_probe.py --kind dc --proxy-index 0 --email nancybaker2jyy@yumail.co
  python scripts/grok_webshare_browser_probe.py --kind res --proxy-index 2 --probe-chat
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grok_playwright_common import CAPTURE_HOOK, TURBOPACK_HOOK, UA, chat_payload

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
REPORT_DIR = GROK_ROOT / "reports" / "webshare_probe"

DC_FILE = Path(r"C:\Users\Lenovo\Downloads\Webshare 100 proxies (1).txt")
RES_FILE = Path(r"C:\Users\Lenovo\Downloads\Webshare 20 proxies.txt")


def parse_lines(path: Path) -> list[str]:
    if not path.exists():
        return []
    out: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.append(line)
    return out


def to_playwright_proxy(line: str) -> dict:
    if line.startswith("http://") or line.startswith("https://"):
        return {"server": line}
    if "@" in line:
        return {"server": f"http://{line}"}
    parts = line.split(":")
    if len(parts) == 4:
        host, port, user, pw = parts
        return {"server": f"http://{host}:{port}", "username": user, "password": pw}
    if len(parts) == 2:
        return {"server": f"http://{parts[0]}:{parts[1]}"}
    return {"server": f"http://{line}"}


def load_auth(email: str) -> dict:
    return json.loads((GROK_ROOT / "web_auths" / f"{email}.json").read_text(encoding="utf-8"))


def dismiss_modals(page) -> list[str]:
    notes: list[str] = []
    for label in ("忽略", "Dismiss", "Not now", "继续", "Continue"):
        try:
            loc = page.get_by_role("button", name=label)
            if loc.count() and loc.first.is_visible():
                loc.first.click(timeout=2000, force=True)
                notes.append(label)
                page.wait_for_timeout(600)
        except Exception:
            pass
    return notes


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--kind", choices=("dc", "res"), default="dc")
    ap.add_argument("--proxy-index", type=int, default=0)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--probe-chat", action="store_true", help="POST PONG via in-page fetch")
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--json-out", type=Path, default=None)
    args = ap.parse_args()

    proxies = parse_lines(DC_FILE if args.kind == "dc" else RES_FILE)
    if args.list:
        for i, p in enumerate(proxies[:25]):
            px = to_playwright_proxy(p)
            print(i, px.get("server", "")[:50], "auth=" + ("yes" if px.get("username") else "no"))
        print(f"total={len(proxies)} kind={args.kind}")
        return 0

    if not proxies:
        print("no proxy file", file=sys.stderr)
        return 1

    idx = max(0, min(args.proxy_index, len(proxies) - 1))
    pw_proxy = to_playwright_proxy(proxies[idx])
    auth = load_auth(args.email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)

    from playwright.sync_api import sync_playwright

    report: dict = {
        "email": args.email,
        "kind": args.kind,
        "proxy_index": idx,
        "proxy_server": pw_proxy.get("server"),
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "steps": [],
    }

    def step(name: str, fn):
        t0 = time.time()
        try:
            data = fn()
            row = {"name": name, "ok": True, "elapsed_s": round(time.time() - t0, 2), **(data or {})}
        except Exception as exc:
            row = {"name": name, "ok": False, "error": f"{type(exc).__name__}:{exc}", "elapsed_s": round(time.time() - t0, 2)}
        report["steps"].append(row)
        print(json.dumps({"step": name, **row}, ensure_ascii=False), flush=True)
        return row

    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not args.headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(proxy=pw_proxy, user_agent=UA, viewport={"width": 1400, "height": 900})
        ctx.add_init_script(TURBOPACK_HOOK)
        ctx.add_init_script(CAPTURE_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = ctx.new_page()

        def goto_grok():
            page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
            page.wait_for_timeout(4000)
            notes = dismiss_modals(page)
            title = page.title() or ""
            body_snip = (page.content() or "")[:1500].lower()
            cf = "just a moment" in title.lower() or "just a moment" in body_snip or "challenge" in body_snip
            logged = "sign in" not in body_snip and "log in" not in body_snip[:800]
            return {"title": title, "cf_challenge": cf, "likely_logged_in": logged and not cf, "modal_notes": notes}

        step("goto_grok", goto_grok)

        def rate_limits():
            captured: list[dict] = []

            def on_resp(resp) -> None:
                if "rate-limits" not in resp.url:
                    return
                try:
                    body = resp.text()
                    data = json.loads(body) if body else None
                    captured.append({"http": resp.status, "data": data})
                except Exception:
                    pass

            page.on("response", on_resp)
            page.reload(wait_until="domcontentloaded", timeout=60000)
            page.wait_for_timeout(5000)
            page.remove_listener("response", on_resp)
            ok_row = next((c for c in captured if c.get("http") == 200 and isinstance(c.get("data"), dict)), None)
            if not ok_row:
                return {"http": captured[-1]["http"] if captured else 0, "ok_quota": False}
            d = ok_row["data"]
            return {
                "http": 200,
                "remainingQueries": d.get("remainingQueries"),
                "totalQueries": d.get("totalQueries"),
                "waitTimeSeconds": d.get("waitTimeSeconds"),
                "ok_quota": True,
            }

        step("rate_limits", rate_limits)

        if args.probe_chat:
            payload = chat_payload("Reply with exactly: PONG")

            def chat_probe():
                res = page.evaluate(
                    """async (body) => {
                      const r = await fetch('/rest/app-chat/conversations/new', {
                        method: 'POST',
                        headers: {'content-type': 'application/json'},
                        body: JSON.stringify(body),
                        credentials: 'include',
                      });
                      const text = await r.text();
                      return {http: r.status, has_pong: text.includes('PONG'), preview: text.slice(0, 600)};
                    }""",
                    payload,
                )
                return {
                    "http": res.get("http"),
                    "has_pong": res.get("has_pong"),
                    "preview": res.get("preview"),
                    "chat_ok": res.get("http") == 200 and res.get("has_pong"),
                }

            step("chat_pong", chat_probe)

        report["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        report["ok"] = any(s.get("name") == "goto_grok" and s.get("ok") and not s.get("cf_challenge") for s in report["steps"])
        browser.close()

    out_path = args.json_out or REPORT_DIR / f"webshare_{args.kind}_{idx}_{args.email.replace('@', '_at_')}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"ok": report.get("ok"), "json": str(out_path)}, ensure_ascii=False, indent=2))
    return 0 if report.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
