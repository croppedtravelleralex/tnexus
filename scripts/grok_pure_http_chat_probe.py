#!/usr/bin/env python3
"""Panda 联调：grok2api SQLite 号池 + udeal 代理 + 无票纯 HTTP chat。

流程：解密 sso → udeal 出口抓 meta → node 本地签名 bundle → POST conversations/new。
不依赖 browser-bridge / chrome-ticket / 外部 wodf.de signer。

用法（Panda）：
  GROK_EGRESS_PROXY=http://user:pass@70.39.164.200:30000 \\
  python3 grok_pure_http_chat_probe.py \\
    --db /opt/grok2api/data/backend.db \\
    --bundle /tmp/grok_sign_standalone.js \\
    --provider grok_web --enabled-only --all \\
    --workers 4 --json-out /tmp/grok_screen.json
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Optional

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
)
CHAT_PATH = "/rest/app-chat/conversations/new"
BASE = "https://grok.com"


def decrypt_credential(enc_b64: str, key: bytes) -> Optional[str]:
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    except ImportError:
        print("[fatal] pip install cryptography", file=sys.stderr)
        sys.exit(2)
    if not enc_b64:
        return None
    try:
        # grok2api uses base64.RawStdEncoding (no padding)
        pad = "=" * ((4 - len(enc_b64) % 4) % 4)
        raw = base64.b64decode(enc_b64 + pad, validate=False)
        nonce, ct = raw[:12], raw[12:]
        pt = AESGCM(key).decrypt(nonce, ct, None)
        return pt.decode("utf-8", errors="replace").strip()
    except Exception:
        return None


def decrypt_token(enc_b64: str, key: bytes) -> Optional[str]:
    return decrypt_credential(enc_b64, key)


def load_proxy_from_egress(db_path: str, key: bytes, node_id: int = 110) -> Optional[str]:
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    row = con.execute(
        "SELECT encrypted_proxy_url FROM egress_nodes WHERE id = ?",
        (node_id,),
    ).fetchone()
    con.close()
    if not row or not row[0]:
        return None
    return decrypt_credential(row[0], key)


def extract_meta(html: str) -> Optional[str]:
    # meta name 以 gr 开头（grok-site-verification 等）
    m = re.search(
        r'<meta[^>]+name=["\'](gr[^"\']*)["\'][^>]+content=["\']([^"\']+)["\']',
        html,
        re.I,
    )
    if m:
        return m.group(2)
    m = re.search(
        r'<meta[^>]+content=["\']([^"\']+)["\'][^>]+name=["\'](gr[^"\']*)["\']',
        html,
        re.I,
    )
    if m:
        return m.group(1)
    return None


def http_request(
    url: str,
    method: str = "GET",
    headers: Optional[dict] = None,
    body: Optional[bytes] = None,
    proxy: Optional[str] = None,
    timeout: int = 60,
) -> tuple[int, str, dict]:
    headers = dict(headers or {})
    if proxy:
        opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({"http": proxy, "https": proxy})
        )
    else:
        opener = urllib.request.build_opener()
    req = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with opener.open(req, timeout=timeout) as resp:
            data = resp.read(256 * 1024).decode("utf-8", errors="replace")
            return resp.status, data, dict(resp.headers)
    except urllib.error.HTTPError as e:
        data = e.read(256 * 1024).decode("utf-8", errors="replace")
        return e.code, data, dict(e.headers)
    except OSError as e:
        return 0, str(e), {}


def sign_with_node(bundle_path: str, meta: str, path: str, method: str) -> Optional[str]:
    with open(bundle_path, encoding="utf-8") as f:
        js = f.read()
    js = js.replace("__GROK_META__", meta)
    js = js.replace("__SIGN_PATH__", path)
    js = js.replace("__SIGN_METHOD__", method)
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
        tmp.write(js)
        tmp_path = tmp.name
    try:
        proc = subprocess.run(
            ["node", tmp_path],
            capture_output=True,
            text=True,
            timeout=30,
        )
        out = proc.stdout + proc.stderr
        for line in out.splitlines():
            if line.startswith("FULLSIG "):
                parts = line.split(" ", 2)
                if len(parts) >= 3:
                    return parts[2].strip()
        # fallback: last non-empty stdout line
        for line in reversed(proc.stdout.splitlines()):
            s = line.strip()
            if len(s) > 60:
                return s
        print(f"  [sign] node exit={proc.returncode} out={out[:400]}", file=sys.stderr)
        return None
    finally:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass


def build_chat_body() -> bytes:
    payload = {
        "model": "grok-chat-fast",
        "messages": [{"role": "user", "content": "Reply with exactly: OK"}],
        "enableImageGeneration": False,
        "enableImageStreaming": False,
        "fileAttachments": [],
    }
    return json.dumps(payload).encode()


def sticky_proxy(base_proxy: str, account_id: int) -> str:
    # udeal relay: Proxy-Authorization username = sticky key（可选；默认 round-robin）
    return base_proxy


def load_accounts(
    db_path: str,
    *,
    limit: int,
    offset: int,
    provider: Optional[str],
    enabled_only: bool,
) -> list[tuple[int, str, str]]:
    clauses = [
        "ac.encrypted_primary IS NOT NULL",
        "ac.encrypted_primary != ''",
    ]
    params: list[object] = []
    if provider:
        clauses.append("pa.provider = ?")
        params.append(provider)
    if enabled_only:
        clauses.append("pa.enabled = 1")
    sql = f"""
        SELECT pa.id, pa.identity_key, ac.encrypted_primary
        FROM provider_accounts pa
        JOIN account_credentials ac ON ac.account_id = pa.id
        WHERE {' AND '.join(clauses)}
        ORDER BY pa.id
    """
    if limit > 0:
        sql += " LIMIT ? OFFSET ?"
        params.extend([limit, offset])
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    rows = con.execute(sql, params).fetchall()
    con.close()
    return rows


def classify_post(status: int, body: str) -> str:
    if status == 200:
        return "alive"
    if status == 401:
        return "auth_failed"
    if status == 403:
        low = body.lower()
        if "anti-bot" in low or "anti_bot" in low or '"code":7' in body:
            return "anti_bot"
        return "forbidden"
    if status == 429:
        return "rate_limited"
    if status == 0:
        return "network_error"
    return f"http_{status}"


def probe_one(
    account_id: int,
    identity: str,
    enc: str,
    *,
    key: bytes,
    bundle_path: str,
    proxy: str,
    skip_post: bool,
) -> dict:
    out: dict = {"id": account_id, "identity": identity, "status": "unknown"}
    token = decrypt_token(enc, key)
    if not token:
        out["status"] = "decrypt_fail"
        return out
    cookie = f"sso={token}; sso-rw={token}"
    proxy_url = sticky_proxy(proxy, account_id)

    status, html, _ = http_request(
        BASE + "/",
        headers={"User-Agent": UA, "Accept": "text/html,*/*", "Cookie": cookie},
        proxy=proxy_url,
    )
    if status != 200:
        out["status"] = "meta_http_fail"
        out["meta_http"] = status
        return out
    meta = extract_meta(html)
    if not meta:
        cf = "Just a moment" in html or "_cf_chl" in html
        out["status"] = "cf_challenge" if cf else "meta_parse_fail"
        return out

    sig = sign_with_node(bundle_path, meta, CHAT_PATH, "POST")
    if not sig:
        out["status"] = "sign_fail"
        return out

    headers = {
        "User-Agent": UA,
        "Accept": "*/*",
        "Content-Type": "application/json",
        "Origin": BASE,
        "Referer": BASE + "/",
        "Cookie": cookie,
        "x-statsig-id": sig,
    }

    get_path = "/rest/app-chat/conversations"
    get_sig = sign_with_node(bundle_path, meta, get_path, "GET")
    if get_sig:
        g_status, _, _ = http_request(
            BASE + get_path,
            headers={**headers, "x-statsig-id": get_sig},
            proxy=proxy_url,
        )
        out["get_http"] = g_status

    if skip_post:
        out["status"] = "meta_ok" if out.get("get_http") == 200 else "signed_only"
        return out

    p_status, p_body, _ = http_request(
        BASE + CHAT_PATH,
        method="POST",
        headers=headers,
        body=build_chat_body(),
        proxy=proxy_url,
        timeout=90,
    )
    out["post_http"] = p_status
    out["status"] = classify_post(p_status, p_body)
    if out["status"] != "alive":
        out["snippet"] = p_body.replace("\n", " ")[:180]
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="Pure HTTP grok chat probe (Panda)")
    ap.add_argument("--db", default="/opt/grok2api/data/backend.db")
    ap.add_argument("--bundle", default="/tmp/grok_sign_standalone.js")
    ap.add_argument(
        "--proxy",
        default=os.environ.get("GROK_EGRESS_PROXY", ""),
        help="HTTP proxy URL (or set GROK_EGRESS_PROXY)",
    )
    ap.add_argument("--provider", default="grok_web", help="filter provider_accounts.provider")
    ap.add_argument("--enabled-only", action="store_true", help="only enabled accounts")
    ap.add_argument("--all", action="store_true", help="scan entire filtered pool")
    ap.add_argument("--limit", type=int, default=15, help="accounts to try (ignored with --all)")
    ap.add_argument("--offset", type=int, default=0, help="skip first N accounts")
    ap.add_argument("--workers", type=int, default=1, help="concurrent probes")
    ap.add_argument("--json-out", default="", help="write full results JSON")
    ap.add_argument("--skip-post", action="store_true", help="only test GET+sign")
    args = ap.parse_args()

    key_b64 = os.environ.get("GROK_CREDENTIAL_KEY", "")
    if not key_b64:
        # fallback: read from grok2api config.yaml on Panda
        cfg = "/opt/grok2api/config.yaml"
        if os.path.isfile(cfg):
            with open(cfg, encoding="utf-8") as f:
                for line in f:
                    if "credentialEncryptionKey:" in line:
                        key_b64 = line.split(":", 1)[1].strip().strip('"')
                        break
    if not key_b64:
        print("Set GROK_CREDENTIAL_KEY or ensure config.yaml credentialEncryptionKey", file=sys.stderr)
        return 2
    key = base64.b64decode(key_b64)
    if len(key) != 32:
        print("GROK_CREDENTIAL_KEY must be 32 bytes", file=sys.stderr)
        return 2
    if not os.path.isfile(args.bundle):
        print(f"Missing bundle: {args.bundle}", file=sys.stderr)
        return 2

    if not args.proxy:
        args.proxy = load_proxy_from_egress(args.db, key) or ""
    if not args.proxy:
        print("Set --proxy, GROK_EGRESS_PROXY, or ensure egress node 110 has encrypted_proxy_url", file=sys.stderr)
        return 2

    limit = 0 if args.all else args.limit
    rows = load_accounts(
        args.db,
        limit=limit,
        offset=args.offset,
        provider=args.provider or None,
        enabled_only=args.enabled_only,
    )
    print(
        f"[probe] accounts={len(rows)} workers={args.workers} "
        f"provider={args.provider!r} enabled_only={args.enabled_only} "
        f"proxy={args.proxy.split('@')[-1] if '@' in args.proxy else args.proxy}",
        flush=True,
    )

    results: list[dict] = []
    lock = threading.Lock()
    done = 0
    t0 = time.time()

    def run_row(row: tuple[int, str, str]) -> dict:
        account_id, identity, enc = row
        return probe_one(
            account_id,
            identity,
            enc,
            key=key,
            bundle_path=args.bundle,
            proxy=args.proxy,
            skip_post=args.skip_post,
        )

    workers = max(1, args.workers)
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {pool.submit(run_row, row): row for row in rows}
        for fut in as_completed(futures):
            res = fut.result()
            with lock:
                results.append(res)
                done += 1
                st = res["status"]
                mark = "✅" if st == "alive" else "·"
                if st == "alive" or done % 25 == 0 or done == len(rows):
                    print(
                        f"  [{done}/{len(rows)}] id={res['id']} {st} {mark}",
                        flush=True,
                    )

    results.sort(key=lambda r: r["id"])
    counts: dict[str, int] = {}
    alive_ids: list[int] = []
    for r in results:
        counts[r["status"]] = counts.get(r["status"], 0) + 1
        if r["status"] == "alive":
            alive_ids.append(r["id"])

    summary = {
        "tried": len(results),
        "elapsed_s": round(time.time() - t0, 1),
        "counts": counts,
        "alive_ids": alive_ids,
        "alive_count": len(alive_ids),
    }
    print(f"\n[summary] {json.dumps(summary, ensure_ascii=False)}", flush=True)
    if alive_ids:
        print(f"[alive] ids={alive_ids}", flush=True)

    if args.json_out:
        payload = {"summary": summary, "results": results}
        with open(args.json_out, "w", encoding="utf-8") as f:
            json.dump(payload, f, ensure_ascii=False, indent=2)
        print(f"[json] wrote {args.json_out}", flush=True)

    return 0 if alive_ids else 1


if __name__ == "__main__":
    sys.exit(main())
