"""TNexus standalone OAuth PKCE bridge (no gptimage HTTP dependency)."""
from __future__ import annotations

import os
import secrets
import threading
import time
import uuid
from typing import Any
from urllib.parse import parse_qs, urlencode, urlparse

from curl_cffi import requests

from pkce import generate_pkce

AUTH_BASE = "https://auth.openai.com"
PLATFORM_BASE = "https://platform.openai.com"
PLATFORM_OAUTH_CLIENT_ID = "app_2SKx67EdpoN0G6j64rFvigXD"
PLATFORM_OAUTH_REDIRECT_URI = f"{PLATFORM_BASE}/auth/callback"
PLATFORM_OAUTH_AUDIENCE = "https://api.openai.com/v1"
PLATFORM_AUTH0_CLIENT = "eyJuYW1lIjoiYXV0aDAtc3BhLWpzIiwidmVyc2lvbiI6IjEuMjEuMCJ9"
USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
    "AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/145.0.0.0 Safari/537.36"
)
SEC_CH_UA = '"Google Chrome";v="145", "Not?A_Brand";v="8", "Chromium";v="145"'


class OAuthLoginError(Exception):
    pass


def _proxy_session_kwargs(account: dict[str, Any] | None = None) -> dict[str, Any]:
    proxy = ""
    if account:
        proxy = str(account.get("proxy") or "").strip()
    if not proxy:
        proxy = str(os.environ.get("TNEXUS_UPSTREAM_PROXY") or os.environ.get("HTTPS_PROXY") or "").strip()
    kwargs: dict[str, Any] = {"impersonate": "chrome", "verify": False}
    if proxy:
        kwargs["proxies"] = {"http": proxy, "https": proxy}
    return kwargs


class OAuthLoginService:
    _SESSION_TTL_SECONDS = 10 * 60
    _MAX_SESSIONS = 64

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._sessions: dict[str, dict[str, Any]] = {}

    def _purge_expired_locked(self) -> None:
        now = time.time()
        expired = [sid for sid, item in self._sessions.items() if now - item["created_at"] > self._SESSION_TTL_SECONDS]
        for sid in expired:
            self._sessions.pop(sid, None)
        if len(self._sessions) > self._MAX_SESSIONS:
            ordered = sorted(self._sessions.items(), key=lambda kv: kv[1]["created_at"])
            for sid, _ in ordered[: len(self._sessions) - self._MAX_SESSIONS]:
                self._sessions.pop(sid, None)

    def start(self, email_hint: str = "") -> dict[str, str]:
        verifier, challenge = generate_pkce()
        nonce = secrets.token_urlsafe(32)
        device_id = str(uuid.uuid4())
        session_id = uuid.uuid4().hex
        state = f"{session_id}.{secrets.token_urlsafe(16)}"
        params = {
            "issuer": AUTH_BASE,
            "client_id": PLATFORM_OAUTH_CLIENT_ID,
            "audience": PLATFORM_OAUTH_AUDIENCE,
            "redirect_uri": PLATFORM_OAUTH_REDIRECT_URI,
            "device_id": device_id,
            "screen_hint": "login_or_signup",
            "max_age": "0",
            "scope": "openid profile email offline_access",
            "response_type": "code",
            "response_mode": "query",
            "state": state,
            "nonce": nonce,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
            "auth0Client": PLATFORM_AUTH0_CLIENT,
        }
        email_hint = str(email_hint or "").strip()
        if email_hint:
            params["login_hint"] = email_hint
        authorize_url = f"{AUTH_BASE}/api/accounts/authorize?{urlencode(params)}"
        with self._lock:
            self._purge_expired_locked()
            self._sessions[session_id] = {
                "code_verifier": verifier,
                "state": state,
                "created_at": time.time(),
                "redirect_uri": PLATFORM_OAUTH_REDIRECT_URI,
            }
        return {
            "session_id": session_id,
            "authorize_url": authorize_url,
            "expires_in": str(self._SESSION_TTL_SECONDS),
            "redirect_uri_prefix": PLATFORM_OAUTH_REDIRECT_URI,
        }

    @staticmethod
    def _extract_code_from_callback(value: str) -> tuple[str, str]:
        raw = str(value or "").strip()
        if not raw:
            return "", ""
        if raw.startswith("http://") or raw.startswith("https://"):
            parsed = parse_qs(urlparse(raw).query)
            code = str((parsed.get("code") or [""])[0]).strip()
            state = str((parsed.get("state") or [""])[0]).strip()
            if not code:
                err = str((parsed.get("error_description") or parsed.get("error") or [""])[0]).strip()
                raise OAuthLoginError(err or "callback URL 中没有 code 参数")
            return code, state
        return raw, ""

    def finish(self, session_id: str, callback: str) -> dict[str, str]:
        body_sid = str(session_id or "").strip()
        code, state = self._extract_code_from_callback(callback)
        if not code:
            raise OAuthLoginError("缺少 code 或 callback URL")
        state_sid = state.split(".", 1)[0] if state else ""
        candidate_sids = [sid for sid in (state_sid, body_sid) if sid]
        if not candidate_sids:
            raise OAuthLoginError("既未提供 session_id，callback URL 中也未携带 state")
        with self._lock:
            self._purge_expired_locked()
            session = None
            picked_sid = ""
            for sid in candidate_sids:
                cur = self._sessions.get(sid)
                if cur is not None:
                    session = cur
                    picked_sid = sid
                    break
        if session is None:
            raise OAuthLoginError("OAuth 会话已过期或不存在，请重新生成授权链接")
        if state and session.get("state") and state != session["state"]:
            raise OAuthLoginError("state 不匹配，请点「重新生成」后再走一次授权")
        tokens = self._exchange_code(code, session["code_verifier"], session.get("redirect_uri") or PLATFORM_OAUTH_REDIRECT_URI)
        with self._lock:
            self._sessions.pop(picked_sid, None)
        return tokens

    @staticmethod
    def _exchange_code(code: str, code_verifier: str, redirect_uri: str) -> dict[str, str]:
        session = requests.Session(**_proxy_session_kwargs())
        try:
            response = session.post(
                f"{AUTH_BASE}/api/accounts/oauth/token",
                headers={
                    "accept": "application/json",
                    "content-type": "application/json",
                    "origin": PLATFORM_BASE,
                    "referer": f"{PLATFORM_BASE}/",
                    "auth0-client": PLATFORM_AUTH0_CLIENT,
                    "sec-ch-ua": SEC_CH_UA,
                    "user-agent": USER_AGENT,
                },
                json={
                    "client_id": PLATFORM_OAUTH_CLIENT_ID,
                    "code_verifier": code_verifier,
                    "grant_type": "authorization_code",
                    "code": code,
                    "redirect_uri": redirect_uri,
                },
                timeout=60,
            )
            try:
                data = response.json() if response.text else {}
            except Exception:
                data = {}
        finally:
            session.close()
        if response.status_code != 200 or not isinstance(data, dict) or not data.get("access_token"):
            detail = ""
            if isinstance(data, dict):
                detail = str(data.get("error_description") or data.get("error") or data.get("message") or "")
            raise OAuthLoginError(
                f"OpenAI 拒绝换 token (HTTP {response.status_code}){': ' + detail if detail else ''}"
            )
        access_token = str(data.get("access_token") or "").strip()
        refresh_token = str(data.get("refresh_token") or "").strip()
        id_token = str(data.get("id_token") or "").strip()
        if not access_token:
            raise OAuthLoginError("OpenAI 返回的 access_token 为空")
        if not refresh_token:
            raise OAuthLoginError("OpenAI 没有返回 refresh_token")
        return {
            "access_token": access_token,
            "refresh_token": refresh_token,
            "id_token": id_token,
        }


oauth_login_service = OAuthLoginService()
