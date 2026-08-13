#!/usr/bin/env python3
"""Compare dead vs healthy gptimage accounts — run on Panda."""
from __future__ import annotations

import json
import sqlite3
import statistics
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

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

DB = Path("/root/gptimage/data/accounts.db")
FIELDS = [
    "status",
    "type",
    "quota",
    "fail",
    "invalid_count",
    "image_fail_streak",
    "identity_conflict_count",
    "success",
    "proxy",
    "password",
    "refresh_token",
    "last_token_refresh_error",
    "last_refresh_error",
    "last_invalid_at",
    "last_token_refresh_error_at",
    "last_refresh_error_at",
    "last_used_at",
    "created_at",
    "last_quota_refresh_error",
    "image_fail_cooldown_until",
    "chatgpt_session_expires",
    "default_model_slug",
    "is_deactivated",
    "has_active_subscription",
    "identity_last_conflict_fields",
    "identity_update_reason",
    "last_login_error",
    "outlook_recovery_status",
    "outlook_last_check_at",
    "outlook_mail_error",
    "nurture_state",
    "cf_probe_streak",
    "soft_band_percent",
]


def load_accounts() -> list[dict]:
    conn = sqlite3.connect(DB)
    rows = conn.execute("SELECT access_token, data FROM accounts").fetchall()
    conn.close()
    out = []
    for token, raw in rows:
        try:
            data = json.loads(raw or "{}")
        except json.JSONDecodeError:
            continue
        if not isinstance(data, dict):
            continue
        data = dict(data)
        data["access_token"] = token
        email = str(data.get("email") or "").strip().lower()
        if not email:
            continue
        data["email"] = email
        out.append(data)
    return out


def domain(email: str) -> str:
    return email.split("@")[-1] if "@" in email else "?"


def parse_ts(v) -> float | None:
    if v is None:
        return None
    if isinstance(v, (int, float)):
        return float(v)
    s = str(v).strip()
    if not s:
        return None
    try:
        return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def summarize_group(name: str, accounts: list[dict]) -> None:
    print(f"\n{'='*60}\n{name} (n={len(accounts)})\n{'='*60}")
    print("domains:", dict(Counter(domain(a["email"]) for a in accounts)))
    print("status:", dict(Counter(str(a.get("status") or "?") for a in accounts)))
    print("type:", dict(Counter(str(a.get("type") or "?") for a in accounts)))

  # refresh errors
    err_c = Counter()
    for a in accounts:
        e = str(a.get("last_token_refresh_error") or a.get("last_refresh_error") or "")
        if "session has ended" in e.lower():
            err_c["session_ended"] += 1
        elif "already been used" in e.lower():
            err_c["refresh_reused"] += 1
        elif e.strip():
            err_c[e[:60]] += 1
        else:
            err_c["none"] += 1
    print("refresh_errors:", dict(err_c))

    nums = ["quota", "fail", "invalid_count", "image_fail_streak", "identity_conflict_count", "success"]
    for k in nums:
        vals = [a.get(k) for a in accounts if isinstance(a.get(k), (int, float))]
        if vals:
            print(f"  {k}: min={min(vals)} max={max(vals)} median={statistics.median(vals):.1f}")

    has_pw = sum(1 for a in accounts if str(a.get("password") or "").strip())
    has_rt = sum(1 for a in accounts if str(a.get("refresh_token") or "").strip())
    has_proxy = sum(1 for a in accounts if str(a.get("proxy") or "").strip())
    print(f"  has_password={has_pw}/{len(accounts)} has_refresh_token={has_rt}/{len(accounts)} has_proxy={has_proxy}/{len(accounts)}")

    deactivated = sum(1 for a in accounts if a.get("is_deactivated") is True)
    if deactivated:
        print(f"  is_deactivated_true={deactivated}")

    # extra keys present only in some accounts
    extra_keys = Counter()
    for a in accounts:
        for k in a:
            if k.startswith("outlook") or k.startswith("identity") or "cooldown" in k or "nurture" in k:
                if a.get(k) not in (None, "", 0, False, []):
                    extra_keys[k] += 1
    if extra_keys:
        print("  notable_keys:", dict(extra_keys.most_common(15)))


def print_dead_detail(accounts: list[dict]) -> None:
    print(f"\n{'='*60}\nDEAD ACCOUNT DETAIL\n{'='*60}")
    for a in sorted(accounts, key=lambda x: x["email"]):
        err = str(a.get("last_token_refresh_error") or a.get("last_refresh_error") or "")[:100]
        print(
            f"\n{a['email']}\n"
            f"  status={a.get('status')} type={a.get('type')} quota={a.get('quota')} fail={a.get('fail')} "
            f"invalid={a.get('invalid_count')} img_streak={a.get('image_fail_streak')}\n"
            f"  created={a.get('created_at')} last_used={a.get('last_used_at')} last_invalid={a.get('last_invalid_at')}\n"
            f"  proxy={str(a.get('proxy') or '')[:60]}\n"
            f"  refresh_err={err}\n"
            f"  identity_conflicts={a.get('identity_conflict_count')} reason={a.get('identity_update_reason')}\n"
            f"  outlook={a.get('outlook_recovery_status')} mail_err={str(a.get('outlook_mail_error') or '')[:80]}"
        )


def compare_numeric(dead: list[dict], healthy: list[dict], key: str) -> None:
    d = [a.get(key) for a in dead if isinstance(a.get(key), (int, float))]
    h = [a.get(key) for a in healthy if isinstance(a.get(key), (int, float))]
    if not d and not h:
        return
    print(f"  {key}: dead_median={statistics.median(d) if d else 'n/a'} healthy_median={statistics.median(h) if h else 'n/a'}")


def main() -> None:
    accounts = load_accounts()
    dead = [a for a in accounts if a["email"] in DEAD]
    healthy = [
        a
        for a in accounts
        if a["email"] not in DEAD
        and str(a.get("status")) == "正常"
        and int(a.get("quota") or 0) > 0
    ]
    print(f"total={len(accounts)} dead={len(dead)} healthy_ref={len(healthy)}")
    summarize_group("DEAD", dead)
    summarize_group("HEALTHY (正常+quota>0)", healthy)
    print("\n--- numeric contrast ---")
    for k in ["fail", "invalid_count", "image_fail_streak", "identity_conflict_count", "quota"]:
        compare_numeric(dead, healthy, k)
    print_dead_detail(dead)

    # check outlook credentials store if exists
    cred_paths = [
        "/root/gptimage/data/outlook_credentials.json",
        "/root/gptimage/data/outlook_accounts.json",
        "/opt/tnexus/data/outlook",
    ]
    print("\n--- outlook/mail artifacts ---")
    for p in cred_paths:
        path = Path(p)
        print(f"  {p}: exists={path.exists()}")


if __name__ == "__main__":
    main()
