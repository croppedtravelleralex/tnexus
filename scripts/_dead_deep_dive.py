#!/usr/bin/env python3
"""Deep dive dead account analysis — run inside chatgpt2api-local."""
from __future__ import annotations

import hashlib
import json
import sqlite3
from collections import defaultdict

DEAD = {
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


def token_fp(t: str) -> str:
    return "token:" + hashlib.sha256(t.encode()).hexdigest()[:10]


def main() -> None:
    conn = sqlite3.connect("/app/data/accounts.db")
    rows = conn.execute("SELECT access_token, data FROM accounts").fetchall()
    email_by_fp: dict[str, str] = {}
    by_email: dict[str, list[tuple[str, dict]]] = defaultdict(list)
    for t, raw in rows:
        d = json.loads(raw or "{}")
        em = str(d.get("email") or "").lower()
        if not em:
            continue
        email_by_fp[token_fp(t)] = em
        by_email[em].append((t, d))

    batch = [
        "token:8cb319978f",
        "token:e59e709e5e",
        "token:7cdd49d666",
        "token:fe938a16cb",
        "token:259a97a912",
        "token:c553c5fe7f",
        "token:3ab7fce8ed",
        "token:4843b5ea0c",
        "token:a5c26c30ea",
        "token:4d91091a8b",
        "token:a867dc6f16",
        "token:1f6ddf2b8d",
        "token:f3dad2c091",
    ]
    print("=== batch refresh 16:53 mapping ===")
    for tok in batch:
        print(tok, "->", email_by_fp.get(tok, "?"))

    first: dict[str, str] = {}
    last: dict[str, str] = {}
    cnt: dict[str, int] = defaultdict(int)
    sources: dict[str, dict[str, int]] = defaultdict(lambda: defaultdict(int))
    with open("/app/data/logs.jsonl", encoding="utf-8") as f:
        for line in f:
            if "session has ended" not in line and "already been used" not in line:
                continue
            j = json.loads(line)
            tok = j.get("detail", {}).get("token")
            em = email_by_fp.get(tok)
            if em not in DEAD:
                continue
            tm = j["time"]
            src = j.get("detail", {}).get("source", "?")
            cnt[em] += 1
            sources[em][src] += 1
            if em not in first or tm < first[em]:
                first[em] = tm
            if em not in last or tm > last[em]:
                last[em] = tm

    print("\n=== historical fail timeline ===")
    for em in sorted(DEAD):
        print(f"{em}: fails={cnt[em]} first={first.get(em, '-')} last={last.get(em, '-')}")
        if sources[em]:
            top = sorted(sources[em].items(), key=lambda x: -x[1])[:4]
            print("  sources:", top)

    print("\n=== duplicate rows ===")
    for em in sorted(DEAD):
        lst = by_email.get(em, [])
        rts = {str(d.get("refresh_token") or "")[:24] for _, d in lst}
        print(em, "rows", len(lst), "unique_rt", len(rts), "status", [d.get("status") for _, d in lst])

    print("\n=== yumail recent mail (security signals) ===")
    try:
        from services import yumail_otp

        print("yumail_ok", yumail_otp.is_configured(), yumail_otp.probe_reachable().get("ok"))
        for em in sorted(DEAD):
            try:
                if hasattr(yumail_otp, "list_recent_messages_by_email"):
                    msgs = yumail_otp.list_recent_messages_by_email(em, limit=5) or []
                else:
                    msgs = []
                hints = []
                for m in msgs[:5]:
                    subj = str(m.get("subject") or "")
                    sender = str(m.get("from") or m.get("sender") or "")
                    hints.append(f"{sender[:30]}|{subj[:50]}")
                print(em, "n=", len(msgs), hints[:3] if hints else "no_api")
            except Exception as exc:
                print(em, "mail_err", str(exc)[:100])
    except Exception as exc:
        print("yumail_import_fail", exc)

    print("\n=== outlook recovery creds presence ===")
    try:
        from services.outlook_account_recovery_service import outlook_account_recovery_service

        creds = outlook_account_recovery_service._load_credentials()  # noqa: SLF001
        cred_emails = {str(c.get("email") or "").lower() for c in (creds or [])}
        for em in sorted(DEAD):
            print(em, "has_cred", em in cred_emails)
    except Exception as exc:
        print("cred_check_fail", str(exc)[:120])


if __name__ == "__main__":
    main()
