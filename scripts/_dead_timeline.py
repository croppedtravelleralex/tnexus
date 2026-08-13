#!/usr/bin/env python3
import hashlib
import json
import sqlite3

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

DB = "/app/data/accounts.db"
LOG = "/app/data/logs.jsonl"


def token_hash(t: str) -> str:
    return "token:" + hashlib.md5(t.encode()).hexdigest()[:10]


conn = sqlite3.connect(DB)
rows = conn.execute("SELECT access_token, data FROM accounts").fetchall()
email_by_th = {}
by_email = {}
for t, raw in rows:
    d = json.loads(raw or "{}")
    em = str(d.get("email") or "").lower()
    if not em:
        continue
    email_by_th[token_hash(t)] = em
    by_email.setdefault(em, []).append(d)

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
print("=== batch refresh 16:53 token -> email ===")
for t in batch:
    print(t, "->", email_by_th.get(t, "?"))

first, last, cnt = {}, {}, {}
sources = {}
with open(LOG, encoding="utf-8") as f:
    for line in f:
        if "session has ended" not in line and "already been used" not in line:
            continue
        j = json.loads(line)
        tok = j.get("detail", {}).get("token")
        em = email_by_th.get(tok)
        if em not in DEAD:
            continue
        tm = j["time"]
        src = j.get("detail", {}).get("source", "?")
        cnt[em] = cnt.get(em, 0) + 1
        sources.setdefault(em, {})[src] = sources[em].get(src, 0) + 1
        if em not in first or tm < first[em]:
            first[em] = tm
        if em not in last or tm > last[em]:
            last[em] = tm

print("\n=== fail timeline ===")
for em in sorted(DEAD):
    print(
        em,
        "fails",
        cnt.get(em, 0),
        "first",
        first.get(em, "-"),
        "last",
        last.get(em, "-"),
    )
    if em in sources:
        top = sorted(sources[em].items(), key=lambda x: -x[1])[:4]
        print("  sources:", top)

print("\n=== duplicate rows / same refresh_token? ===")
for em in sorted(DEAD):
    lst = by_email.get(em, [])
    rts = {str(a.get("refresh_token") or "")[:40] for a in lst}
    print(em, "rows", len(lst), "unique_rt_prefix", len(rts))
