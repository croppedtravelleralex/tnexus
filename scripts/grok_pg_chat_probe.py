#!/usr/bin/env python3
"""Probe PG grok_accounts for chat POST viability (Panda)."""
from __future__ import annotations

import base64
import json
import os
import re
import subprocess
import tempfile
import urllib.error
import urllib.request

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
)
BASE = "https://grok.com"
CHAT_PATH = "/rest/app-chat/conversations/new"
BUNDLE = os.environ.get(
    "GROK_SIGN_BUNDLE", "/root/TNexus/crates/grok-signer/assets/grok_sign_standalone.js"
)
PROXY = os.environ.get("GROK_EGRESS_PROXY", "")
KEY_B64 = os.environ.get("GROK_CREDENTIAL_KEY", "")
LIMIT = int(os.environ.get("PROBE_LIMIT", "30"))


def decrypt(enc_b64: str, key: bytes) -> str:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    pad = "=" * ((4 - len(enc_b64) % 4) % 4)
    raw = base64.b64decode(enc_b64 + pad)
    nonce, ct = raw[:12], raw[12:]
    return AESGCM(key).decrypt(nonce, ct, None).decode().strip()


def http(
    url: str, method: str = "GET", headers: dict | None = None, body: bytes | None = None
) -> tuple[int, str]:
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({"http": PROXY, "https": PROXY})
    )
    req = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    try:
        with opener.open(req, timeout=60) as resp:
            return resp.status, resp.read(256 * 1024).decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read(256 * 1024).decode("utf-8", errors="replace")


def extract_meta(html: str) -> str | None:
    m = re.search(
        r'name=["\'](gr[^"\']*)["\'][^>]+content=["\']([^"\']+)["\']', html, re.I
    )
    if m:
        return m.group(2)
    m = re.search(
        r'content=["\']([^"\']+)["\'][^>]+name=["\'](gr[^"\']*)["\']', html, re.I
    )
    return m.group(1) if m else None


def sign(meta: str, path: str, method: str) -> str | None:
    js = (
        open(BUNDLE, encoding="utf-8")
        .read()
        .replace("__GROK_META__", meta)
        .replace("__SIGN_PATH__", path)
        .replace("__SIGN_METHOD__", method)
    )
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
        tmp.write(js)
        p = tmp.name
    proc = subprocess.run(["node", p], capture_output=True, text=True, timeout=30)
    os.unlink(p)
    for line in (proc.stdout + proc.stderr).splitlines():
        if line.startswith("FULLSIG "):
            return line.split(" ", 2)[2].strip()
    for line in reversed(proc.stdout.splitlines()):
        s = line.strip()
        if len(s) > 60:
            return s
    return None


def load_pg_rows(limit: int) -> list[tuple[str, str, str]]:
    sql = (
        "SELECT ga.id::text, ga.identity_key, gc.encrypted_primary "
        "FROM grok_accounts ga "
        "JOIN grok_credentials gc ON gc.account_id = ga.id "
        "WHERE ga.provider = 'grok_web' AND ga.enabled = true "
        "AND gc.encrypted_primary IS NOT NULL "
        f"ORDER BY ga.id LIMIT {limit}"
    )
    out = subprocess.check_output(
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
            "-F",
            "|",
            "-c",
            sql,
        ]
    ).decode()
    rows: list[tuple[str, str, str]] = []
    for line in out.strip().splitlines():
        if not line.strip():
            continue
        parts = line.split("|", 2)
        if len(parts) == 3:
            rows.append((parts[0], parts[1], parts[2]))
    return rows


def main() -> int:
    if not KEY_B64 or not PROXY:
        print("need GROK_CREDENTIAL_KEY and GROK_EGRESS_PROXY", flush=True)
        return 2
    key = base64.b64decode(KEY_B64)
    rows = load_pg_rows(LIMIT)
    print(f"[pg-probe] accounts={len(rows)} proxy={PROXY.split('@')[-1]}", flush=True)
    counts: dict[str, int] = {}
    for aid, ident, enc in rows:
        try:
            token = decrypt(enc, key)
        except Exception as e:
            counts["decrypt_fail"] = counts.get("decrypt_fail", 0) + 1
            print(f"id={aid} decrypt_fail {e}", flush=True)
            continue
        cookie = f"sso={token}; sso-rw={token}"
        st, html = http(BASE + "/", headers={"User-Agent": UA, "Cookie": cookie})
        if st != 200:
            counts["meta_fail"] = counts.get("meta_fail", 0) + 1
            print(f"id={aid} meta {st}", flush=True)
            continue
        meta = extract_meta(html)
        if not meta:
            counts["no_meta"] = counts.get("no_meta", 0) + 1
            print(f"id={aid} no_meta", flush=True)
            continue
        sig = sign(meta, CHAT_PATH, "POST")
        if not sig:
            counts["sign_fail"] = counts.get("sign_fail", 0) + 1
            print(f"id={aid} sign_fail", flush=True)
            continue
        payload = {
            "collectionIds": [],
            "disabledConnectorIds": [],
            "deviceEnvInfo": {
                "darkModeEnabled": False,
                "devicePixelRatio": 2,
                "screenHeight": 1328,
                "screenWidth": 2056,
                "viewportHeight": 1083,
                "viewportWidth": 2056,
            },
            "disableMemory": True,
            "disableSearch": False,
            "disableSelfHarmShortCircuit": False,
            "disableTextFollowUps": False,
            "enableImageGeneration": False,
            "enableImageStreaming": False,
            "enableSideBySide": True,
            "fileAttachments": [],
            "forceConcise": False,
            "forceSideBySide": False,
            "imageAttachments": [],
            "imageGenerationCount": 0,
            "isAsyncChat": False,
            "message": "Reply with exactly: OK",
            "modeId": "fast",
            "responseMetadata": {},
            "returnImageBytes": False,
            "returnRawGrokInXaiRequest": False,
            "sendFinalMetadata": True,
            "temporary": True,
        }
        body = json.dumps(payload).encode()
        headers = {
            "User-Agent": UA,
            "Content-Type": "application/json",
            "Origin": BASE,
            "Referer": BASE + "/",
            "Cookie": cookie,
            "x-statsig-id": sig,
        }
        pst, pbody = http(BASE + CHAT_PATH, "POST", headers, body)
        if pst == 200:
            label = "alive"
        elif pst == 401:
            label = "auth"
        elif pst == 403:
            label = "anti_bot"
        else:
            label = f"http_{pst}"
        counts[label] = counts.get(label, 0) + 1
        print(f"id={aid} POST {pst} {label} {pbody[:100]!r}", flush=True)
    print(f"[summary] {counts}", flush=True)
    return 0 if counts.get("alive", 0) > 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
