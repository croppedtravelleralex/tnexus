"""Account refresh / relogin for TNexus (uses gptimage Python libs when GPTIMAGE_ROOT is set)."""
from __future__ import annotations

import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

log = logging.getLogger("account_ops")

ROOT = Path(__file__).resolve().parents[1]
GPTIMAGE_ROOT = Path(os.environ.get("GPTIMAGE_ROOT") or (ROOT.parent / "gptimage")).resolve()
_GPTIMAGE_READY = False


def ensure_gptimage() -> None:
    global _GPTIMAGE_READY
    if _GPTIMAGE_READY:
        return
    if not GPTIMAGE_ROOT.is_dir():
        raise RuntimeError(f"GPTIMAGE_ROOT not found: {GPTIMAGE_ROOT}")
    if str(GPTIMAGE_ROOT) not in sys.path:
        sys.path.insert(0, str(GPTIMAGE_ROOT))
    os.chdir(GPTIMAGE_ROOT)
    cfg = GPTIMAGE_ROOT / "config.json"
    if not os.environ.get("CHATGPT2API_AUTH_KEY") and cfg.is_file():
        import json

        try:
            ak = str(json.loads(cfg.read_text(encoding="utf-8")).get("auth-key") or "").strip()
            if ak:
                os.environ["CHATGPT2API_AUTH_KEY"] = ak
        except Exception:
            pass
    _GPTIMAGE_READY = True


def _account_dict_to_backend(account: dict[str, Any]):
    from curl_cffi import requests as curl_requests
    from services.account_fingerprint import ensure_complete_fp
    from services.openai_backend_api import OpenAIBackendAPI
    from services.proxy_service import proxy_settings

    token = str(account.get("access_token") or "").strip()
    if not token:
        raise ValueError("access_token is required")
    api = OpenAIBackendAPI(access_token=token)
    proxy = str(account.get("proxy") or "").strip()
    acc: dict[str, Any] = {
        "email": str(account.get("email") or ""),
        "proxy": proxy,
        "access_token": token,
        "refresh_token": str(account.get("refresh_token") or ""),
        "oai-device-id": str(account.get("device_id") or account.get("oai-device-id") or ""),
        "user-agent": str(account.get("user_agent") or account.get("user-agent") or ""),
        "fp": dict(account.get("fp") or {}),
    }
    if acc["oai-device-id"]:
        acc["fp"].setdefault("oai-device-id", acc["oai-device-id"])
    if acc["user-agent"]:
        acc["fp"].setdefault("user-agent", acc["user-agent"])
    api.account = acc
    api.fp, _ = ensure_complete_fp(api.account)
    api.user_agent = api.fp["user-agent"]
    api.device_id = api.fp["oai-device-id"]
    api.session_id = api.fp["oai-session-id"]
    try:
        api.close()
    except Exception:
        pass
    api._closed = False
    api.session = curl_requests.Session(
        **proxy_settings.build_session_kwargs(
            account=api.account,
            impersonate=api.fp["impersonate"],
            verify=True,
            upstream=True,
        )
    )
    api.session.headers.update({"User-Agent": api.user_agent, "Accept-Language": api._accept_language()})
    return api


def refresh_access_token(account: dict[str, Any], *, force: bool = False) -> dict[str, Any]:
    ensure_gptimage()
    from curl_cffi import requests
    from services.proxy_service import proxy_settings

    acc = dict(account)
    refresh_token = str(acc.get("refresh_token") or "").strip()
    if not refresh_token:
        return acc
    oauth_client_id = "app_2SKx67EdpoN0G6j64rFvigXD"
    session = requests.Session(
        **proxy_settings.build_session_kwargs(account=acc, impersonate="chrome110", verify=True, upstream=True)
    )
    try:
        response = session.post(
            "https://auth.openai.com/oauth/token",
            headers={
                "Accept": "application/json",
                "Content-Type": "application/x-www-form-urlencoded",
                "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            },
            data={
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": oauth_client_id,
            },
            timeout=60,
        )
        data = response.json() if response.text else {}
        if response.status_code == 200 and isinstance(data, dict) and data.get("access_token"):
            acc["access_token"] = str(data.get("access_token") or "").strip()
            if data.get("refresh_token"):
                acc["refresh_token"] = str(data.get("refresh_token") or "").strip()
            if data.get("id_token"):
                acc["id_token"] = str(data.get("id_token") or "").strip()
            acc["last_token_refresh_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
            acc["last_token_refresh_error"] = None
        elif force:
            acc["last_token_refresh_error"] = str(
                data.get("error_description") or data.get("error") or response.text
            )[:300]
    finally:
        session.close()
    return acc


def refresh_account(account: dict[str, Any]) -> dict[str, Any]:
    """Refresh token (if possible) then pull remote user/quota info."""
    ensure_gptimage()
    from services.openai_backend_api import InvalidAccessTokenError

    acc = refresh_access_token(dict(account))
    token = str(acc.get("access_token") or "").strip()
    api = _account_dict_to_backend(acc)
    try:
        info = api.get_user_info()
    except InvalidAccessTokenError:
        acc = refresh_access_token(acc, force=True)
        api = _account_dict_to_backend(acc)
        info = api.get_user_info()
    finally:
        try:
            api.close()
        except Exception:
            pass
    merged = dict(acc)
    for key in (
        "email",
        "status",
        "quota",
        "type",
        "restore_at",
        "image_quota_unknown",
        "limits_progress",
        "last_quota_refresh_at",
    ):
        if key in info and info[key] is not None:
            merged[key] = info[key]
    merged["source_type"] = merged.get("source_type") or "tnexus_refresh"
    return merged


def relogin_account(account: dict[str, Any]) -> dict[str, Any]:
    ensure_gptimage()
    from services.account_service import account_service

    email = str(account.get("email") or "").strip()
    password = str(account.get("password") or "").strip()
    if not email or not password:
        raise ValueError("账号缺少 email/password，无法密码重登")
    result = account_service._login_with_password(email, password, account=dict(account))
    if not result.get("ok"):
        raise RuntimeError(str(result.get("error") or "password relogin failed"))
    merged = dict(account)
    merged.update(
        {
            "access_token": str(result.get("access_token") or ""),
            "refresh_token": str(result.get("refresh_token") or merged.get("refresh_token") or ""),
            "id_token": str(result.get("id_token") or merged.get("id_token") or ""),
            "status": "正常",
            "source_type": str(result.get("source_type") or "password"),
        }
    )
    return refresh_account(merged)
