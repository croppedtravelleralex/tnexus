#!/usr/bin/env python3
"""Extract statsig fingerprint from grok.com (no SSO required)."""
import os
import re
import sys

DIGEST_HOOK = """
(() => {
  const cap = (d) => {
    try {
      const t = new TextDecoder().decode(d);
      if (t.includes('obfiowerehiring')) globalThis.__grokDigestInputs.push(t);
    } catch (e) {}
  };
  globalThis.__grokDigestInputs = [];
  const crypto = globalThis.crypto;
  if (crypto?.subtle?.digest) {
    const original = crypto.subtle.digest.bind(crypto.subtle);
    crypto.subtle.digest = (a, d) => { cap(d); return original(a, d); };
  }
})();
"""

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
)


def main() -> int:
    proxy = os.environ.get("GROK_EGRESS_PROXY", "") or os.environ.get("GROK_LOCAL_PROXY", "")
    if not proxy:
        print("set GROK_LOCAL_PROXY or GROK_EGRESS_PROXY", file=sys.stderr)
        return 2
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        ctx = browser.new_context(proxy={"server": proxy}, user_agent=UA)
        ctx.add_init_script(DIGEST_HOOK)
        page = ctx.new_page()
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=120000)
        page.wait_for_timeout(10000)
        page.evaluate(
            "async () => { try { await fetch('/rest/modes', {credentials:'include'}); } catch(e){} }"
        )
        page.wait_for_timeout(4000)
        digests = page.evaluate("() => (globalThis.__grokDigestInputs || []).slice(-80)") or []
        browser.close()

    best = max(digests, key=len) if digests else ""
    m = re.match(r"^([A-Z]+)!([^!]+)!(\d+)obfiowerehiring(.*)$", best)
    if not m:
        print(f"failed digests={len(digests)} sample={best[:120]!r}", file=sys.stderr)
        return 1
    print(m.group(4))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
