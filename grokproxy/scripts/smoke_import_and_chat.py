#!/usr/bin/env python3
"""End-to-end smoke: import a real Build credential, then chat through grokProxy.

Reads one `xai-*.json` produced by the register pipeline, posts it to the
running grokProxy, and issues a chat request so the whole path (ingest →
refresh → schedule → upstream) is exercised against the live upstream.
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path


def call(url: str, payload: dict | None, token: str, timeout: float = 120.0):
    data = None if payload is None else json.dumps(payload).encode()
    headers = {"Accept": "application/json"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=data, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
            return response.status, json.loads(body or b"{}")
    except urllib.error.HTTPError as exc:
        raw = exc.read()
        try:
            return exc.code, json.loads(raw or b"{}")
        except Exception:
            return exc.code, {"raw": raw[:400].decode("utf-8", "replace")}
    except Exception as exc:
        return 0, {"error": str(exc)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="http://127.0.0.1:8110")
    ap.add_argument("--admin-key", default="")
    ap.add_argument("--api-key", default="")
    ap.add_argument("--auth-file", required=True, help="path to xai-<email>.json")
    args = ap.parse_args()

    auth_path = Path(args.auth_file)
    auth = json.loads(auth_path.read_text(encoding="utf-8"))
    account = {
        "email": auth.get("email") or auth_path.stem.replace("xai-", ""),
        "access_token": auth.get("access_token", ""),
        "refresh_token": auth.get("refresh_token", ""),
        "expires_at": auth.get("expires_at") or auth.get("expired") or 0,
        "headers": auth.get("headers") or {},
    }
    # proxy_url intentionally omitted: the smoke host reaches upstream directly.

    status, body = call(
        f"{args.base}/api/v1/accounts",
        {"provider": "build", "accounts": [account]},
        args.admin_key,
    )
    print(f"IMPORT  {status} {json.dumps(body, ensure_ascii=False)[:200]}", flush=True)
    if status != 200:
        return 2

    status, body = call(f"{args.base}/readyz", None, "")
    print(f"READYZ  {status} {json.dumps(body, ensure_ascii=False)[:120]}", flush=True)

    status, body = call(f"{args.base}/v1/models", None, args.api_key)
    print(f"MODELS  {status} {json.dumps(body, ensure_ascii=False)[:200]}", flush=True)

    status, body = call(
        f"{args.base}/v1/chat/completions",
        {
            "model": "grok-4.5",  # deliberately stale: the proxy must correct it
            "messages": [{"role": "user", "content": "Reply with exactly PROXY_OK"}],
            "stream": False,
            "max_tokens": 8,
        },
        args.api_key,
    )
    text = json.dumps(body, ensure_ascii=False)
    print(f"CHAT    {status} {text[:400]}", flush=True)

    status, listed = call(f"{args.base}/api/v1/accounts", None, args.admin_key)
    for row in listed.get("accounts", []):
        print(
            f"ACCOUNT {row['email']} health={row['health']} model={row['last_model']} "
            f"ok={row['success_count']} fail={row['failure_count']} err={row['last_error'][:80]}",
            flush=True,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
