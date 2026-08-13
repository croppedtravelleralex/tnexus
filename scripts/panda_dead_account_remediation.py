#!/usr/bin/env python3
"""Panda gptimage dead-account remediation: dedup, isolate, append recovery creds.

Run inside chatgpt2api-local:
  docker exec chatgpt2api-local /app/.venv/bin/python /app/data/panda_dead_account_remediation.py
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
DUP_EMAILS = {
    "alvinian4635@outlook.com",
    "conradflta5259@outlook.com",
    "hypatiajordan4883@outlook.com",
}
ZOMBIE_FORCE = {
    "gitanaamanda19706@outlook.com",
    "hypatiajordan4883@outlook.com",
}

DB = Path("/app/data/accounts.db")
CREDS = Path("/app/data/runlogs/panda-outlook-recovery.credentials.secret.txt")
DEFAULT_CLIENT_ID = "9e5f94bc-e8a4-4e73-b8be-63364c29d753"


def parse_time(v: object) -> float:
    if v is None:
        return 0.0
    s = str(v).strip()
    if not s:
        return 0.0
    try:
        if s.endswith("Z"):
            s = s[:-1] + "+00:00"
        return datetime.fromisoformat(s).timestamp()
    except ValueError:
        pass
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M:%S.%f"):
        try:
            return datetime.strptime(s, fmt).replace(tzinfo=timezone.utc).timestamp()
        except ValueError:
            continue
    return 0.0


def row_score(data: dict) -> tuple:
    err = str(data.get("last_token_refresh_error") or data.get("last_refresh_error") or "")
    has_terminal = "session has ended" in err.lower() or "already been used" in err.lower()
    return (
        0 if has_terminal else 1,
        parse_time(data.get("last_used_at")),
        parse_time(data.get("last_token_refresh_at")),
        -int(data.get("invalid_count") or 0),
        int(data.get("quota") or 0),
    )


def load_rows(conn: sqlite3.Connection) -> list[tuple[str, dict]]:
    out = []
    for token, raw in conn.execute("SELECT access_token, data FROM accounts"):
        try:
            data = json.loads(raw or "{}")
        except json.JSONDecodeError:
            continue
        if isinstance(data, dict):
            out.append((token, data))
    return out


def main() -> None:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    backup = DB.with_suffix(f".bak-dead-remediation-{stamp}")
    shutil.copy2(DB, backup)
    print(f"backup={backup}")

    conn = sqlite3.connect(DB)
    rows = load_rows(conn)
    by_email: dict[str, list[tuple[str, dict]]] = {}
    for token, data in rows:
        em = str(data.get("email") or "").lower().strip()
        if em:
            by_email.setdefault(em, []).append((token, data))

    delete_tokens: list[str] = []
    for em in DUP_EMAILS:
        items = by_email.get(em, [])
        if len(items) <= 1:
            print(f"dedup {em}: single row, skip")
            continue
        ranked = sorted(items, key=lambda x: row_score(x[1]), reverse=True)
        keep_token, keep_data = ranked[0]
        drop = [t for t, _ in ranked[1:]]
        delete_tokens.extend(drop)
        print(f"dedup {em}: keep token_len={len(keep_token)} drop={len(drop)}")

    now = datetime.now(timezone.utc).isoformat()
    updated = 0
    for token, data in rows:
        em = str(data.get("email") or "").lower().strip()
        if token in delete_tokens:
            continue
        if em not in DEAD_EMAILS:
            continue
        next_data = dict(data)
        next_data["status"] = "异常"
        next_data["panda_receive_state"] = "identity_isolated"
        next_data["panda_sync_state"] = "ready"
        next_data["quota"] = 0
        next_data["image_quota_unknown"] = False
        next_data["outlook_recovery_state"] = next_data.get("outlook_recovery_state") or "pending"
        if em in ZOMBIE_FORCE or em in DEAD_EMAILS:
            next_data["invalid_count"] = max(int(next_data.get("invalid_count") or 0), 1)
        err = str(next_data.get("last_token_refresh_error") or next_data.get("last_refresh_error") or "")
        if err and "invalid_count" not in next_data:
            next_data["invalid_count"] = 1
        next_data["last_invalid_at"] = next_data.get("last_invalid_at") or now
        next_data["updated_at"] = now
        conn.execute(
            "UPDATE accounts SET data=? WHERE access_token=?",
            (json.dumps(next_data, ensure_ascii=False), token),
        )
        updated += 1
        print(f"isolated {em} status=异常 receive=identity_isolated")

    for token in delete_tokens:
        conn.execute("DELETE FROM accounts WHERE access_token=?", (token,))
        print(f"deleted duplicate token len={len(token)}")

    conn.commit()
    conn.close()
    print(f"updated_rows={updated} deleted={len(delete_tokens)}")

    # Append recovery credentials (email----password----client_id----refresh_token)
    if not CREDS.parent.exists():
        CREDS.parent.mkdir(parents=True, exist_ok=True)
    existing = CREDS.read_text(encoding="utf-8", errors="ignore").lower() if CREDS.is_file() else ""
    conn = sqlite3.connect(DB)
    appended = 0
    lines: list[str] = []
    seen_em: set[str] = set()
    for token, raw in conn.execute("SELECT access_token, data FROM accounts"):
        data = json.loads(raw or "{}")
        em = str(data.get("email") or "").lower().strip()
        if em not in DEAD_EMAILS or em in seen_em:
            continue
        seen_em.add(em)
        if em in existing:
            print(f"creds skip existing {em}")
            continue
        pw = str(data.get("password") or "").strip()
        rt = str(data.get("refresh_token") or "").strip()
        if not pw:
            print(f"creds skip no password {em}")
            continue
        lines.append(f"{em}----{pw}----{DEFAULT_CLIENT_ID}----{rt}")
        appended += 1
    conn.close()
    if lines:
        with CREDS.open("a", encoding="utf-8") as f:
            if CREDS.stat().st_size > 0:
                f.write("\n")
            f.write("\n".join(lines))
            f.write("\n")
        print(f"creds_appended={appended} file={CREDS}")
    else:
        print("creds_appended=0")


if __name__ == "__main__":
    main()
