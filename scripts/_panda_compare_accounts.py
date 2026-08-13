#!/usr/bin/env python3
"""Compare gptimage sqlite vs TNexus PG accounts (run on Panda)."""
import json
import sqlite3
import subprocess

def main():
    gpt = {}
    con = sqlite3.connect("/root/gptimage/data/accounts.db")
    for token, data_s in con.execute("SELECT access_token, data FROM accounts"):
        d = json.loads(data_s)
        email = (d.get("email") or "").lower()
        gpt[email] = {
            "quota": d.get("quota"),
            "status": d.get("status"),
            "image_quota_unknown": d.get("image_quota_unknown"),
            "panda_receive_state": d.get("panda_receive_state"),
        }
    con.close()

    out = subprocess.check_output(
        [
            "docker", "exec", "panda-postgres-1", "psql", "-U", "tnexus", "-d", "tnexus",
            "-tAc", "SELECT email, data::text FROM tnexus_accounts;",
        ],
        text=True,
    )
    tn = {}
    for line in out.strip().split("\n"):
        if "|" not in line:
            continue
        email, data_s = line.split("|", 1)
        d = json.loads(data_s)
        tn[email.lower()] = {
            "quota": d.get("quota"),
            "status": d.get("status"),
            "image_quota_unknown": d.get("image_quota_unknown"),
            "panda_receive_state": d.get("panda_receive_state"),
        }

    print(f"gptimage={len(gpt)} tnexus_pg={len(tn)}")
    only_gpt = set(gpt) - set(tn)
    only_tn = set(tn) - set(gpt)
    both = set(gpt) & set(tn)
    print(f"only_gptimage={len(only_gpt)} only_tnexus={len(only_tn)} overlap={len(both)}")

    unk_tn = [e for e, v in tn.items() if v.get("image_quota_unknown")]
    unk_gpt = [e for e, v in gpt.items() if v.get("image_quota_unknown")]
    print(f"image_quota_unknown: tnexus={len(unk_tn)} gptimage={len(unk_gpt)}")

    mismatch = []
    for e in sorted(both):
        g, t = gpt[e], tn[e]
        if g.get("quota") != t.get("quota") or g.get("status") != t.get("status"):
            mismatch.append((e, g, t))
    print(f"quota/status mismatch in overlap: {len(mismatch)}")
    for e, g, t in mismatch[:8]:
        print(f"  {e[:40]}: gpt q={g['quota']} st={g['status']} | tn q={t['quota']} st={t['status']}")

    if only_tn:
        print("sample only_tnexus (first 5):")
        for e in list(only_tn)[:5]:
            print(f"  {e}: {tn[e]}")

if __name__ == "__main__":
    main()
