#!/usr/bin/env python3
"""Delete dead Outlook accounts from gptimage accounts.db (Panda ops).

Run inside chatgpt2api-local:
  docker exec chatgpt2api-local /app/.venv/bin/python /app/data/panda_delete_dead_accounts.py
"""
from __future__ import annotations

import json
import shutil
import sqlite3
from datetime import datetime, timezone
from pathlib import Path

DEAD_EMAILS = {
    "alvinian4635@outlook.com",
    "aspenvincent99941@outlook.com",
    "barthcherry24674@outlook.com",
    "conradflta5259@outlook.com",
    "davidlynn8783@outlook.com",
    "dreamachristine11594@outlook.com",
    "ellencary92031@outlook.com",
    "everleighpearl98363@outlook.com",
    "freemansavannah5327@outlook.com",
    "garyelizabeth8128@outlook.com",
    "gitanaamanda19706@outlook.com",
    "hypatiajordan4883@outlook.com",
}

DB = Path("/app/data/accounts.db")
CREDS = Path("/app/data/runlogs/panda-outlook-recovery.credentials.secret.txt")


def main() -> None:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    backup = DB.with_suffix(f".bak-delete-dead-{stamp}")
    shutil.copy2(DB, backup)
    print(f"backup={backup}")

    conn = sqlite3.connect(DB)
    delete_tokens: list[str] = []
    deleted_emails: list[str] = []
    for token, raw in conn.execute("SELECT access_token, data FROM accounts"):
        try:
            data = json.loads(raw or "{}")
        except json.JSONDecodeError:
            continue
        em = str(data.get("email") or "").lower().strip()
        if em in DEAD_EMAILS:
            delete_tokens.append(token)
            deleted_emails.append(em)

    for token in delete_tokens:
        conn.execute("DELETE FROM accounts WHERE access_token=?", (token,))
    conn.commit()
    remaining = conn.execute("SELECT COUNT(*) FROM accounts").fetchone()[0]
    conn.close()

    print(f"deleted={len(delete_tokens)} emails={sorted(set(deleted_emails))}")
    print(f"remaining={remaining}")

    if CREDS.is_file():
        lines = CREDS.read_text(encoding="utf-8", errors="ignore").splitlines()
        kept = [ln for ln in lines if ln.strip() and ln.split("----")[0].lower().strip() not in DEAD_EMAILS]
        removed = len(lines) - len(kept)
        if removed:
            CREDS.write_text("\n".join(kept) + ("\n" if kept else ""), encoding="utf-8")
            print(f"creds_removed={removed}")


if __name__ == "__main__":
    main()
