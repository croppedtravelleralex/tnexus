"""Sync TNexus account dict into gptimage sqlite pool for ops services."""
from __future__ import annotations

from typing import Any

from account_ops import ensure_gptimage


def ensure_account_in_pool(account: dict[str, Any]) -> dict[str, Any]:
    ensure_gptimage()
    from services.account_service import account_service

    token = str(account.get("access_token") or "").strip()
    if not token:
        raise ValueError("access_token is required")
    existing = account_service.get_account(token)
    payload = dict(account)
    if existing:
        account_service.update_account(token, payload, quiet=True)
    else:
        account_service.import_account_items([payload], include_items=False)
    return account_service.get_account(token) or payload


def ensure_accounts_in_pool(accounts: list[dict[str, Any]]) -> int:
    synced = 0
    for row in accounts:
        if not isinstance(row, dict):
            continue
        try:
            ensure_account_in_pool(row)
            synced += 1
        except Exception:
            continue
    return synced
