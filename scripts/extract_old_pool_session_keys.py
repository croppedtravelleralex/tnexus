#!/usr/bin/env python3
"""从 grok2api 老池 SSO 提取 pure_http session keys（存 account_{id}.json）。"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import grok_pure_http_client as gph  # noqa: E402

AUTHS_DIR = ROOT / "reports" / "old_pool_auths"
OUT_DIR = ROOT / "reports" / "pure_http_keys"


def load_sso(account_id: int) -> str:
    path = AUTHS_DIR / f"account_{account_id}.json"
    data = json.loads(path.read_text(encoding="utf-8"))
    sso = str(data.get("sso") or "").strip()
    if not sso:
        raise RuntimeError(f"account {account_id}: missing sso in {path}")
    return sso


def extract_for_account(account_id: int, *, headed: bool = False) -> Path:
    sso = load_sso(account_id)
    email = f"account_{account_id}@oldpool.local"
    orig_load_auth = gph.load_auth

    def patched_auth(e: str, _sso=sso, _email=email):
        if e == _email:
            return {"email": _email, "sso": _sso, "sso_rw": _sso}
        return orig_load_auth(e)

    gph.load_auth = patched_auth
    gph.KEYS_DIR = OUT_DIR
    try:
        keys = gph.extract_session_keys(email, headed=headed)
    finally:
        gph.load_auth = orig_load_auth
    keys["account_id"] = account_id
    keys["sso"] = sso
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out = OUT_DIR / f"account_{account_id}.json"
    out.write_text(json.dumps(keys, ensure_ascii=False, indent=2), encoding="utf-8")
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--account-ids", default="86,304,92", help="comma-separated grok2api ids")
    ap.add_argument("--headed", action="store_true")
    args = ap.parse_args()
    ids = [int(x.strip()) for x in args.account_ids.split(",") if x.strip()]
    results = []
    for aid in ids:
        try:
            out = extract_for_account(aid, headed=args.headed)
            row = {"account_id": aid, "ok": True, "path": str(out)}
            keys = json.loads(out.read_text(encoding="utf-8"))
            row["has_cf"] = keys.get("has_cf")
            results.append(row)
            print(json.dumps(row, ensure_ascii=False))
        except Exception as exc:
            row = {"account_id": aid, "ok": False, "error": str(exc)}
            results.append(row)
            print(json.dumps(row, ensure_ascii=False), file=sys.stderr)
    summary = {
        "ok": sum(1 for r in results if r.get("ok")),
        "n": len(results),
        "out_dir": str(OUT_DIR),
    }
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if summary["ok"] == summary["n"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
