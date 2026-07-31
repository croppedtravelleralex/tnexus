"""TNexus ops bridge — nurture / outlook recovery / quota-window prime via gptimage libs."""
from __future__ import annotations

import logging
from typing import Any

from account_ops import ensure_gptimage
from account_pool_sync import ensure_account_in_pool, ensure_accounts_in_pool

log = logging.getLogger("ops_bridge")
_STARTED = False


def start_background_services() -> dict[str, Any]:
    global _STARTED
    ensure_gptimage()
    out: dict[str, Any] = {"ok": True, "services": []}
    try:
        from services.text_nurture_service import text_nurture_service

        text_nurture_service.start_background()
        out["services"].append("text_nurture")
    except Exception as exc:
        log.warning("text_nurture start failed: %s", exc)
        out.setdefault("errors", []).append(f"text_nurture: {exc}")
    try:
        from services.quota_window_prime_service import quota_window_prime_service

        quota_window_prime_service.start()
        out["services"].append("quota_window_prime")
    except Exception as exc:
        log.warning("quota_window_prime start failed: %s", exc)
        out.setdefault("errors", []).append(f"quota_window_prime: {exc}")
    try:
        from services.outlook_auto_recovery_loop_service import outlook_auto_recovery_loop_service

        outlook_auto_recovery_loop_service.start_background()
        out["services"].append("outlook_auto_recovery")
    except Exception as exc:
        log.warning("outlook_auto_recovery start failed: %s", exc)
        out.setdefault("errors", []).append(f"outlook_auto_recovery: {exc}")
    try:
        from services.webshare_cf_scan_service import webshare_cf_scan_service

        webshare_cf_scan_service.start_background()
        out["services"].append("webshare_cf_scan")
    except Exception as exc:
        log.warning("webshare_cf_scan start failed: %s", exc)
        out.setdefault("errors", []).append(f"webshare_cf_scan: {exc}")
    _STARTED = True
    out["started"] = _STARTED
    return out


def nurture_status() -> dict[str, Any]:
    ensure_gptimage()
    from services.text_nurture_service import text_nurture_service

    status = text_nurture_service.status()
    queue = status.get("queue") if isinstance(status.get("queue"), dict) else {}
    return {
        **status,
        "running": bool(status.get("running") or status.get("worker_alive")),
        "queue": {
            "depth": int(queue.get("depth") or queue.get("queued") or 0),
            "oldest_age_sec": int(queue.get("oldest_age_sec") or 0),
        },
        "completed_in_day": int(status.get("today_completed_total") or 0),
        "max_per_account_per_day": int(status.get("max_per_account_per_day") or 0),
        "last_error": status.get("last_error"),
        "source": "account-ops",
    }


def nurture_enable(enabled: bool) -> dict[str, Any]:
    ensure_gptimage()
    from services.text_nurture_service import text_nurture_service

    return text_nurture_service.set_enabled(bool(enabled))


def nurture_enqueue(
    *,
    prompt: str = "",
    source: str = "tnexus_ui",
    access_tokens: list[str] | None = None,
    accounts: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    ensure_gptimage()
    from services.account_service import account_service
    from services.text_nurture_service import text_nurture_service

    tokens = [str(t).strip() for t in (access_tokens or []) if str(t).strip()]
    if accounts:
        ensure_accounts_in_pool(accounts)
    results = []
    if tokens:
        for token in tokens:
            account = account_service.get_account(token) or {}
            results.append(
                text_nurture_service.enqueue(
                    prompt=prompt,
                    access_token=token,
                    email=str(account.get("email") or ""),
                    source=source or "tnexus_ui",
                )
            )
    else:
        results.append(
            text_nurture_service.enqueue(
                prompt=prompt,
                source=source or "tnexus_ui",
            )
        )
    return {"enqueued": len(results), "items": results, "source": "account-ops"}


def nurture_process_one(payload: dict[str, Any] | None = None) -> dict[str, Any]:
    ensure_gptimage()
    from services.text_nurture_service import text_nurture_service

    data = dict(payload or {})
    token = str(data.get("access_token") or "").strip()
    if token and data.get("account"):
        ensure_account_in_pool(dict(data["account"]))
    elif token:
        from services.account_service import account_service

        if not account_service.get_account(token):
            raise ValueError("account not found in pool; sync account first")
    if not data.get("source"):
        data["source"] = "tnexus_accounts_ui"
    return text_nurture_service.process_one(data)


def outlook_auto_recovery_status() -> dict[str, Any]:
    ensure_gptimage()
    from services.outlook_auto_recovery_loop_service import outlook_auto_recovery_loop_service

    status = outlook_auto_recovery_loop_service.get_status()
    status["source"] = "account-ops"
    status["available"] = True
    return status


def outlook_auto_recovery_update(settings: dict[str, Any]) -> dict[str, Any]:
    ensure_gptimage()
    from services.outlook_auto_recovery_loop_service import outlook_auto_recovery_loop_service

    updates = {k: v for k, v in settings.items() if v is not None}
    return outlook_auto_recovery_loop_service.update_settings(updates)


def outlook_recover_one(access_token: str, account: dict[str, Any] | None = None) -> dict[str, Any]:
    ensure_gptimage()
    from services.outlook_account_recovery_service import outlook_account_recovery_service

    token = str(access_token or "").strip()
    if not token:
        raise ValueError("access_token is required")
    if account:
        ensure_account_in_pool(account)
    progress_id = outlook_account_recovery_service.start(token)
    return {"progress_id": progress_id, "source": "account-ops"}


def outlook_recover_progress(progress_id: str) -> dict[str, Any] | None:
    ensure_gptimage()
    from services.outlook_account_recovery_service import outlook_account_recovery_service

    return outlook_account_recovery_service.get_progress(progress_id)


def quota_prime_enqueue(
    access_tokens: list[str],
    *,
    mode: str = "manual",
    accounts: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    ensure_gptimage()
    from services.quota_window_prime_service import quota_window_prime_service

    tokens = [str(t).strip() for t in access_tokens if str(t).strip()]
    if accounts:
        ensure_accounts_in_pool(accounts)
    if len(tokens) == 1:
        return quota_window_prime_service.enqueue(tokens[0], mode=mode)
    return quota_window_prime_service.enqueue_many(tokens, mode=mode)


def quota_prime_status() -> dict[str, Any]:
    ensure_gptimage()
    from services.quota_window_prime_service import quota_window_prime_service

    status = quota_window_prime_service.get_status()
    running = str(status.get("state") or "").lower() not in {"", "off", "idle"}
    queue = []
    pending = status.get("pending_tokens")
    if isinstance(pending, list):
        queue = pending
    return {
        "running": running,
        "state": status.get("state") or "idle",
        "queue": queue,
        "queue_depth": int(status.get("queue_depth") or len(queue)),
        "status": status,
        "source": "account-ops",
    }


def proxy_runtime_get() -> dict[str, Any]:
    ensure_gptimage()
    from services.config import config
    from services.proxy_service import proxy_settings

    return {
        "runtime": config.get_public_proxy_runtime_settings(),
        "status": proxy_settings.get_runtime_status(),
        "source": "account-ops",
    }


def proxy_runtime_save(settings: dict[str, Any]) -> dict[str, Any]:
    ensure_gptimage()
    from services.config import config

    config.update({"proxy_runtime": settings})
    return proxy_runtime_get()


def proxy_test(url: str) -> dict[str, Any]:
    ensure_gptimage()
    from services.proxy_service import test_proxy

    return {"result": test_proxy((url or "").strip()), "source": "account-ops"}


def webshare_cf_scan_status() -> dict[str, Any]:
    ensure_gptimage()
    from services.webshare_cf_scan_service import webshare_cf_scan_service

    data = webshare_cf_scan_service.status()
    data["source"] = "account-ops"
    return data


def webshare_cf_scan_inventory() -> dict[str, Any]:
    ensure_gptimage()
    from services.webshare_cf_scan_service import webshare_cf_scan_service

    data = webshare_cf_scan_service.inventory()
    data["source"] = "account-ops"
    return data


def webshare_cf_scan_run_once() -> dict[str, Any]:
    ensure_gptimage()
    from services.webshare_cf_scan_service import webshare_cf_scan_service

    return webshare_cf_scan_service.run_once(force=True)
