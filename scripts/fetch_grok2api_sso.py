#!/usr/bin/env python3
"""Decrypt grok2api SQLite account SSO for local probes (run on Panda or with local db copy)."""
from __future__ import annotations

import argparse
import base64
import json
import sqlite3
import sys
from pathlib import Path


def load_key_from_config(config_path: Path) -> bytes:
    text = config_path.read_text(encoding="utf-8")
    for line in text.splitlines():
        if "credentialEncryptionKey" in line:
            val = line.split(":", 1)[1].strip().strip('"')
            return base64.b64decode(val)
    raise RuntimeError(f"credentialEncryptionKey not found in {config_path}")


def decrypt_token(enc_b64: str, key: bytes) -> str:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    raw = base64.b64decode(enc_b64)
    return AESGCM(key).decrypt(raw[:12], raw[12:], None).decode("utf-8")


def fetch_accounts(db_path: Path, key: bytes, account_ids: list[int]) -> list[dict]:
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    rows = []
    for aid in account_ids:
        row = con.execute(
            """
            SELECT pa.id, pa.identity_key, ac.encrypted_primary
            FROM provider_accounts pa
            JOIN account_credentials ac ON ac.account_id = pa.id
            WHERE pa.id = ? AND pa.provider = 'grok_web'
            """,
            (aid,),
        ).fetchone()
        if not row:
            continue
        aid, identity, enc = row
        if not enc:
            continue
        try:
            sso = decrypt_token(enc, key)
        except Exception as exc:
            rows.append({"account_id": aid, "identity_key": identity, "error": str(exc)})
            continue
        rows.append({"account_id": aid, "identity_key": identity, "sso": sso})
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default="/opt/grok2api/data/backend.db")
    ap.add_argument("--config", default="/opt/grok2api/config.yaml")
    ap.add_argument("--account-ids", default="86,304,403", help="comma-separated ids")
    ap.add_argument("--out-dir", default="reports/old_pool_auths")
    args = ap.parse_args()

    key = load_key_from_config(Path(args.config))
    ids = [int(x.strip()) for x in args.account_ids.split(",") if x.strip()]
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    accounts = fetch_accounts(Path(args.db), key, ids)
    for acc in accounts:
        aid = acc["account_id"]
        path = out_dir / f"account_{aid}.json"
        path.write_text(json.dumps(acc, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"wrote {path} identity={acc.get('identity_key','?')[:40]}")
    print(json.dumps({"count": len(accounts), "out_dir": str(out_dir)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
