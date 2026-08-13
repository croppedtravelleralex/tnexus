#!/usr/bin/env python3
"""One-shot: extract GROK_STATSIG_FINGERPRINT from grok.com via Playwright (Panda)."""
from __future__ import annotations

import base64
import hashlib
import os
import re
import struct
import subprocess
import sys
import time

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


def decrypt(enc_b64: str, key: bytes) -> str:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    pad = "=" * ((4 - len(enc_b64) % 4) % 4)
    raw = base64.b64decode(enc_b64 + pad)
    nonce, ct = raw[:12], raw[12:]
    return AESGCM(key).decrypt(nonce, ct, None).decode().strip()


def load_token() -> str:
    key_b64 = os.environ.get("GROK_CREDENTIAL_KEY", "")
    if not key_b64:
        raise SystemExit("GROK_CREDENTIAL_KEY required")
    key = base64.b64decode(key_b64)
    sql = (
        "SELECT gc.encrypted_primary FROM grok_accounts ga "
        "JOIN grok_credentials gc ON gc.account_id = ga.id "
        "WHERE ga.enabled = true ORDER BY ga.id LIMIT 1"
    )
    enc = (
        subprocess.check_output(
            [
                "docker",
                "exec",
                "panda-postgres-1",
                "psql",
                "-U",
                "tnexus",
                "-d",
                "tnexus",
                "-t",
                "-A",
                "-c",
                sql,
            ]
        )
        .decode()
        .strip()
    )
    return decrypt(enc, key)


def main() -> int:
    proxy = os.environ.get("GROK_EGRESS_PROXY", "") or os.environ.get("GROK_LOCAL_PROXY", "")
    if not proxy:
        raise SystemExit("GROK_EGRESS_PROXY or GROK_LOCAL_PROXY required")
    token = load_token()
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        ctx = browser.new_context(proxy={"server": proxy}, user_agent=UA)
        ctx.add_init_script(DIGEST_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": token, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": token, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = ctx.new_page()
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=120000)
        page.wait_for_timeout(8000)
        page.evaluate(
            "async () => { try { await fetch('/rest/modes', {credentials:'include'}); } catch(e){} }"
        )
        page.wait_for_timeout(3000)
        digests = page.evaluate("() => (globalThis.__grokDigestInputs || []).slice(-80)") or []
        browser.close()

    best = max(digests, key=len) if digests else ""
    m = re.match(r"^([A-Z]+)!([^!]+)!(\d+)obfiowerehiring(.*)$", best)
    if not m:
        print(f"extract failed: digests={len(digests)} sample={best[:120]!r}", file=sys.stderr)
        return 1
    fp = m.group(4)
    print(fp)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
