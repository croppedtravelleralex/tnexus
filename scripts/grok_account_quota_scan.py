#!/usr/bin/env python3
"""批量探测 Grok 账号对话额度（浏览器 fetch /rest/rate-limits，无需预提取 session keys）。

用法：
  python scripts/grok_account_quota_scan.py --limit 30
  python scripts/grok_account_quota_scan.py --email foo@bar.com
  python scripts/grok_account_quota_scan.py --limit 50 --min-remaining 1
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grok_playwright_common import PROXY, TURBOPACK_HOOK, UA

GROK_ROOT = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao")
KEYS_DIR = GROK_ROOT / "reports" / "pure_http_keys"
WEB_AUTHS = GROK_ROOT / "web_auths"


def list_emails(limit: int, offset: int, *, domain: str | None = None) -> list[str]:
    files = sorted(WEB_AUTHS.glob("*.json"))
    out: list[str] = []
    for f in files:
        email = f.stem
        if "@" not in email:
            continue
        if domain and not email.endswith(domain):
            continue
        out.append(email)
    return out[offset : offset + limit]


def load_auth(email: str) -> dict:
    return json.loads((WEB_AUTHS / f"{email}.json").read_text(encoding="utf-8"))


def probe_one_browser(page, auth: dict) -> dict:
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    if not sso:
        return {"ok": False, "error": "missing_sso"}

    ctx = page.context
    ctx.clear_cookies()
    ctx.add_cookies(
        [
            {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
            {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
        ]
    )

    captured: list[dict] = []

    def on_resp(resp) -> None:
        if "rate-limits" not in resp.url:
            return
        try:
            body = resp.text()
            data = json.loads(body) if body else None
            captured.append({"http": resp.status, "data": data, "raw": body[:300]})
        except Exception as exc:
            captured.append({"http": resp.status, "error": str(exc)})

    page.on("response", on_resp)
    try:
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=60000)
        page.wait_for_timeout(6000)
        for label in ("忽略", "Dismiss", "继续", "Continue"):
            try:
                loc = page.get_by_role("button", name=label)
                if loc.count() and loc.first.is_visible():
                    loc.first.click(timeout=1500, force=True)
                    page.wait_for_timeout(500)
            except Exception:
                pass

        title = page.title() or ""
        if "moment" in title.lower() or "just a moment" in (page.content() or "").lower()[:2000]:
            return {"ok": False, "error": "cf_challenge", "title": title}

        # 优先用页面自动发起的 rate-limits（带站点签名）；手动 fetch 会 404
        ok_row = next((c for c in captured if c.get("http") == 200 and isinstance(c.get("data"), dict)), None)
        if ok_row:
            d = ok_row["data"]
            return {
                "ok": True,
                "http": 200,
                "remainingQueries": d.get("remainingQueries"),
                "totalQueries": d.get("totalQueries"),
                "waitTimeSeconds": d.get("waitTimeSeconds"),
                "windowSizeSeconds": d.get("windowSizeSeconds"),
                "source": "page_auto",
            }
        last = captured[-1] if captured else {}
        return {"ok": False, "http": last.get("http"), "raw": last.get("raw") or last.get("error"), "source": "page_auto"}
    except Exception as exc:
        return {"ok": False, "error": f"{type(exc).__name__}:{exc}"}
    finally:
        page.remove_listener("response", on_resp)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", help="single account")
    ap.add_argument("--limit", type=int, default=25)
    ap.add_argument("--domain", default=None, help="filter emails by domain e.g. yumail.co")
    ap.add_argument("--offset", type=int, default=0)
    ap.add_argument("--proxy", default=None)
    ap.add_argument("--min-remaining", type=int, default=0, help="only print accounts with remaining >= N")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--json-out", type=Path, default=None)
    args = ap.parse_args()

    proxy = args.proxy or PROXY
    emails = [args.email] if args.email else list_emails(args.limit, args.offset, domain=args.domain)
    if not emails:
        print("no accounts", file=sys.stderr)
        return 1

    from playwright.sync_api import sync_playwright

    results: list[dict] = []
    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not args.headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(proxy={"server": proxy}, user_agent=UA)
        ctx.add_init_script(TURBOPACK_HOOK)
        page = ctx.new_page()

        for i, email in enumerate(emails):
            auth = load_auth(email)
            row = {"email": email, "i": i}
            row.update(probe_one_browser(page, auth))
            results.append(row)
            rem = row.get("remainingQueries")
            flag = "✓" if row.get("ok") and (rem is None or rem >= args.min_remaining) else "·"
            print(
                f"{flag} [{i+1}/{len(emails)}] {email} "
                f"ok={row.get('ok')} rem={rem} total={row.get('totalQueries')} err={row.get('error','')}",
                flush=True,
            )
            time.sleep(0.3)

        browser.close()

    with_quota = [r for r in results if r.get("ok") and (r.get("remainingQueries") or 0) >= max(1, args.min_remaining)]
    report = {
        "scanned": len(results),
        "with_quota": len(with_quota),
        "proxy": proxy,
        "offset": args.offset,
        "limit": args.limit,
        "results": results,
        "top_quota": sorted(
            [r for r in results if r.get("ok")],
            key=lambda x: int(x.get("remainingQueries") or 0),
            reverse=True,
        )[:10],
    }
    out_path = args.json_out or KEYS_DIR / f"quota_scan_{time.strftime('%Y%m%d-%H%M%S')}.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps({"scanned": report["scanned"], "with_quota": report["with_quota"], "json": str(out_path)}, ensure_ascii=False))
    if with_quota:
        print("BEST:", with_quota[0]["email"], "remaining=", with_quota[0].get("remainingQueries"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
