#!/usr/bin/env python3
"""在 Panda 上对 pure_http keys 做 HTTP E2E（避免 Windows TLS 问题）。"""
from __future__ import annotations

import os

# Panda 直连 grok.com，不走本机 7897 代理
os.environ["GROK_LOCAL_PROXY"] = ""
os.environ["GROK_UPSTREAM_PROXY"] = ""

import argparse
import json
import os
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from grok_pure_http_client import GrokPureHttpClient  # noqa: E402

KEYS_DIR = Path(os.environ.get("PANDA_KEYS", "/opt/tnexus/pure_http_keys"))


def probe(account_id: int) -> dict:
    row: dict = {"account_id": account_id}
    path = KEYS_DIR / f"account_{account_id}.json"
    t0 = time.time()
    if not path.exists():
        row.update({"ok": False, "error": "missing_keys_file"})
        return row
    try:
        keys = json.loads(path.read_text(encoding="utf-8"))
        fp = (keys.get("fingerprint") or "").strip()
        row["fingerprint_len"] = len(fp)
        if not keys.get("meta_b64") or len(fp) < 8:
            row.update({"ok": False, "error": "weak_key_no_fingerprint"})
            return row
        client = GrokPureHttpClient(keys, signer="auto", upstream_proxy="")
        r1 = client.request("GET", "/rest/app-chat/conversations")
        row["conversations_http"] = r1.status_code
        chat = client.chat_new("Reply with exactly: PONG")
        row["chat_ok"] = bool(chat.get("conversation_id"))
        row["ok"] = row["conversations_http"] == 200 and row["chat_ok"]
    except Exception as exc:
        row.update({"ok": False, "error": f"{type(exc).__name__}:{exc}"})
    row["elapsed_s"] = round(time.time() - t0, 1)
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("account_ids", nargs="+", type=int)
    args = ap.parse_args()
    results = [probe(aid) for aid in args.account_ids]
    for row in results:
        mark = "OK" if row.get("ok") else "FAIL"
        detail = row.get("error") or f"conv={row.get('conversations_http')} chat={row.get('chat_ok')} fp={row.get('fingerprint_len')}"
        print(f"{mark} id={row['account_id']} {detail}", flush=True)
    ok = sum(1 for r in results if r.get("ok"))
    print(json.dumps({"n": len(results), "ok": ok, "fail": len(results) - ok}, ensure_ascii=False))
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
