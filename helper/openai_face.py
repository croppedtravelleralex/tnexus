"""OpenAI-compatible face for panda :8013 when Rust binary is not yet published.

Interim PROTO_BRIDGE_FACE — same semantics as crates/gateway, implemented in Python
so MVP matrix can run without linux/amd64 rust artifacts.

Account binding:
  - default: PIN_ACCOUNT_FILE (unchanged)
  - optional header X-Preferred-Account-Email: resolve from gptimage account pool
    (unique proxy per email via pool identity); unknown email → 400

Env:
  GATEWAY_LISTEN  default 127.0.0.1:8013 — loopback unless an operator opts into
      a routable bind. Exposing this face publicly also exposes the account pool.
  HELPER_INTERNAL_TOKEN  shared secret required on every route except /health.
      Unset => those routes answer 503 (fail closed, not fail open).

Callers must send the shared secret in the ``X-Helper-Token`` request header;
bringup scripts and the Rust face need to be updated to supply it.
"""
from __future__ import annotations

import hmac
import json
import logging
import os
import threading
import time
import uuid
from pathlib import Path
from typing import Any

from fastapi import Depends, FastAPI, Header, HTTPException
from fastapi.responses import JSONResponse
from pydantic import BaseModel

from protocol_bridge import AccountIn, ImageIn, QuotaIn, TextIn, execute_image, execute_quota, execute_text

PIN_PATH = Path(os.environ.get("PIN_ACCOUNT_FILE", "secrets/pin_account.json"))
LISTEN = os.environ.get("GATEWAY_LISTEN", "127.0.0.1:8013")

log = logging.getLogger("openai_face")

face = FastAPI(title="gptimage-gateway-rs-face", version="0.1.0")


def require_face_token(x_helper_token: str | None = Header(default=None)) -> None:
    """Gate for every account-touching route on this face.

    Mirrors protocol_bridge.require_internal_token; kept as a separate function
    because this module must stay importable on its own. Unset secret means the
    deploy is unconfigured, so it serves nothing rather than everything.
    """
    expected = os.environ.get("HELPER_INTERNAL_TOKEN") or ""
    if not expected.strip():
        raise HTTPException(
            status_code=503,
            detail={
                "error": {
                    "message": (
                        "gateway face disabled: HELPER_INTERNAL_TOKEN is not set. "
                        "Set it on the face process and send it as X-Helper-Token."
                    ),
                    "type": "gateway_error",
                    "code": "internal_token_unconfigured",
                    "fault": "self",
                }
            },
        )
    # Constant-time compare: a byte-wise early exit leaks the secret via timing.
    if not hmac.compare_digest(str(x_helper_token or ""), expected):
        raise HTTPException(
            status_code=401,
            detail={
                "error": {
                    "message": "missing or invalid X-Helper-Token",
                    "type": "gateway_error",
                    "code": "internal_token_invalid",
                    "fault": "client",
                }
            },
        )


def _error_ref(exc: BaseException, event: str) -> str:
    """Log the traceback server-side and return only a correlation id."""
    ref = uuid.uuid4().hex[:12]
    log.exception("%s error_ref=%s: %s: %s", event, ref, type(exc).__name__, exc)
    return ref


def load_pin() -> AccountIn:
    if not PIN_PATH.is_file():
        raise RuntimeError(f"missing PIN_ACCOUNT_FILE={PIN_PATH}")
    raw = json.loads(PIN_PATH.read_text(encoding="utf-8"))
    return AccountIn(
        email=str(raw.get("email") or ""),
        access_token=str(raw.get("access_token") or ""),
        device_id=raw.get("device_id") or None,
        proxy=raw.get("proxy") or None,
        user_agent=raw.get("user_agent") or None,
    )


_PIN: AccountIn | None = None
# Both caches hold account state that rotates upstream (tokens, quota), so every
# entry is timestamped and re-read past its TTL. An unbounded cache would keep
# serving a revoked access_token for the lifetime of the process.
_ACCOUNT_CACHE_TTL_SECS = 60.0
_QUOTA_CACHE_TTL_SECS = 60.0
_QUOTA_CACHE: dict[str, dict[str, Any]] = {}
_QUOTA_CACHE_GUARD = threading.Lock()
_QUOTA_KEY_LOCKS: dict[str, threading.Lock] = {}
_ACCOUNT_CACHE: dict[str, dict[str, Any]] = {}
_ACCOUNT_CACHE_GUARD = threading.Lock()
_IMAGE_LOCKS: dict[str, threading.Lock] = {}
_IMAGE_LOCKS_GUARD = threading.Lock()


def _quota_lock_for(key: str) -> threading.Lock:
    """Per-account refresh lock, same shape as _image_lock_for.

    _QUOTA_CACHE_GUARD only makes dict access atomic; it does not close the
    check-then-act window between "cache miss" and "store result". This lock
    does, so a burst of readers costs one upstream refresh, not one each.
    """
    with _QUOTA_CACHE_GUARD:
        lock = _QUOTA_KEY_LOCKS.get(key)
        if lock is None:
            lock = threading.Lock()
            _QUOTA_KEY_LOCKS[key] = lock
        return lock


def _cache_get_account(key: str) -> AccountIn | None:
    now = time.time()
    with _ACCOUNT_CACHE_GUARD:
        slot = _ACCOUNT_CACHE.get(key)
        if slot is None:
            return None
        if now - float(slot.get("ts") or 0) >= _ACCOUNT_CACHE_TTL_SECS:
            _ACCOUNT_CACHE.pop(key, None)
            return None
        return slot.get("account")


def _cache_put_account(key: str, account: AccountIn) -> None:
    now = time.time()
    with _ACCOUNT_CACHE_GUARD:
        # Sweep aged-out siblings first so the map cannot grow unbounded, then
        # insert — sweeping afterwards would drop the entry just written.
        for stale in [
            k
            for k, v in _ACCOUNT_CACHE.items()
            if now - float(v.get("ts") or 0) >= _ACCOUNT_CACHE_TTL_SECS
        ]:
            _ACCOUNT_CACHE.pop(stale, None)
        _ACCOUNT_CACHE[key] = {"ts": now, "account": account}


def _image_lock_for(email: str) -> threading.Lock:
    key = (email or "").strip().lower() or "_default"
    with _IMAGE_LOCKS_GUARD:
        lock = _IMAGE_LOCKS.get(key)
        if lock is None:
            lock = threading.Lock()
            _IMAGE_LOCKS[key] = lock
        return lock


def pin() -> AccountIn:
    global _PIN
    if _PIN is None:
        _PIN = load_pin()
    return _PIN


def _proxy_host(proxy: str) -> str:
    p = (proxy or "").strip()
    if not p:
        return ""
    return p.split("@")[-1].split(":")[0].lower()


def resolve_account(preferred: str | None) -> AccountIn:
    """Empty preferred → pin. Else resolve pool row by email (token+proxy required)."""
    pref = (preferred or "").strip()
    if not pref:
        return pin()
    key = pref.lower()
    pin_acc = pin()
    if key == (pin_acc.email or "").strip().lower():
        return pin_acc
    cached = _cache_get_account(key)
    if cached is not None:
        return cached
    try:
        from services.account_service import account_service

        for row in account_service.list_accounts():
            email = str(row.get("email") or "").strip()
            if email.lower() != key:
                continue
            token = str(row.get("access_token") or "").strip()
            proxy = str(row.get("proxy") or "").strip()
            if not token or not proxy:
                raise HTTPExceptionish(
                    400,
                    f"pool account {email} missing token or proxy",
                    "account_incomplete",
                    "self",
                )
            fp = row.get("fp") if isinstance(row.get("fp"), dict) else {}
            acc = AccountIn(
                email=email,
                access_token=token,
                device_id=str(
                    row.get("oai-device-id") or fp.get("oai-device-id") or ""
                )
                or None,
                proxy=proxy,
                user_agent=str(row.get("user-agent") or fp.get("user-agent") or "") or None,
            )
            _cache_put_account(key, acc)
            return acc
    except HTTPExceptionish:
        raise
    except Exception as exc:
        raise HTTPExceptionish(
            500,
            "pool resolve failed",
            "pool_resolve_failed",
            "self",
            error_ref=_error_ref(exc, "pool_resolve_failed"),
        ) from exc
    raise HTTPExceptionish(
        400,
        f"unknown preferred account email={pref}",
        "account_not_found",
        "self",
    )


def _cached_quota(account: AccountIn, *, force: bool = False) -> dict[str, Any]:
    key = (account.email or "").strip().lower() or "_pin"

    def _fresh() -> dict[str, Any] | None:
        with _QUOTA_CACHE_GUARD:
            slot = _QUOTA_CACHE.get(key) or {}
            if slot.get("body") and time.time() - float(slot.get("ts") or 0) < _QUOTA_CACHE_TTL_SECS:
                return dict(slot["body"])
        return None

    if not force:
        hit = _fresh()
        if hit is not None:
            return hit
    with _quota_lock_for(key):
        # `force` must still reach upstream — callers use it to retry past a
        # cached failure body — so only the unforced path re-checks here, where
        # a concurrent refresh may have landed while this thread waited.
        if not force:
            hit = _fresh()
            if hit is not None:
                return hit
        body = execute_quota(QuotaIn(account=account))
        with _QUOTA_CACHE_GUARD:
            _QUOTA_CACHE[key] = {"ts": time.time(), "body": body}
        return body


class ChatReq(BaseModel):
    model: str = "gpt-4o-mini"
    messages: list[dict[str, Any]]
    stream: bool = False


class ImageReq(BaseModel):
    model: str = "gpt-image-2"
    prompt: str
    n: int = 1
    size: str = "1024x1024"
    response_format: str = "b64_json"


@face.get("/health")
def health() -> dict[str, Any]:
    # Left unauthenticated for container/bringup liveness probes, so it must not
    # disclose account identity — pin_email moved behind the authenticated routes.
    return {
        "ok": True,
        "service": "gptimage-gateway-rs",
        "wave": "mvp",
        "proto_bridge": True,
        "proto_bridge_face": True,
        "helper_ok": True,
        "listen": LISTEN,
        "pin_loaded": PIN_PATH.is_file(),
        "multi_account": True,
        "min_image_quota": int(os.environ.get("MVP_MIN_IMAGE_QUOTA", "1")),
    }


@face.get("/v1/models", dependencies=[Depends(require_face_token)])
def models() -> dict[str, Any]:
    return {
        "object": "list",
        "data": [
            {"id": "gpt-4o-mini", "object": "model", "owned_by": "gptimage-gateway-rs"},
            {"id": "gpt-image-2", "object": "model", "owned_by": "gptimage-gateway-rs"},
        ],
    }


@face.get("/v1/accounts/candidates", dependencies=[Depends(require_face_token)])
def account_candidates(limit: int = 20):
    """List pool accounts with token+proxy; unique proxy_host preferred for multi-conc.

    Emits no token and no proxy URL — only the fields needed to pick an account.
    """
    limit = max(1, min(100, int(limit or 20)))
    try:
        from services.account_service import account_service

        rows = account_service.list_accounts()
    except Exception as exc:
        return JSONResponse(
            status_code=500,
            content={
                "error": {
                    "message": "list_accounts failed",
                    "type": "gateway_error",
                    "code": "pool_list_failed",
                    "fault": "self",
                    "error_ref": _error_ref(exc, "pool_list_failed"),
                }
            },
        )
    out: list[dict[str, Any]] = []
    seen_proxy: set[str] = set()
    for row in rows:
        email = str(row.get("email") or "").strip()
        token = str(row.get("access_token") or "").strip()
        proxy = str(row.get("proxy") or "").strip()
        status = str(row.get("status") or "")
        if not email or not token or not proxy:
            continue
        if status in {"禁用", "异常", "限流"}:
            continue
        host = _proxy_host(proxy)
        if not host or host in seen_proxy:
            continue
        seen_proxy.add(host)
        out.append(
            {
                "email": email,
                "proxy_host": host,
                "status": status,
                "quota": row.get("quota"),
                "panda_receive_state": row.get("panda_receive_state"),
            }
        )
        if len(out) >= limit:
            break
    return {"ok": True, "count": len(out), "accounts": out}


@face.post("/v1/quota/refresh", dependencies=[Depends(require_face_token)])
def quota_refresh(x_preferred_account_email: str | None = Header(default=None)):
    account = resolve_account(x_preferred_account_email)
    r = _cached_quota(account, force=True)
    if not r.get("ok"):
        fault = r.get("fault") or "upstream"
        code = 500 if fault == "self" else 502
        return JSONResponse(
            status_code=code,
            content={
                "error": {
                    "message": r.get("error") or "quota refresh failed",
                    "type": "gateway_error",
                    "code": "quota_refresh_failed",
                    "fault": fault,
                }
            },
        )
    return {
        "ok": True,
        "email": r.get("email"),
        "plan": r.get("plan"),
        "status": r.get("status"),
        "remaining": r.get("remaining"),
        "restore_at": r.get("restore_at"),
        "image_quota_unknown": r.get("image_quota_unknown"),
        "min_remaining": r.get("min_remaining"),
        "imageable": r.get("imageable"),
        "image_gen": r.get("image_gen"),
        "elapsed_ms": r.get("elapsed_ms"),
        "proxy_host": _proxy_host(account.proxy or ""),
    }


@face.get("/v1/quota", dependencies=[Depends(require_face_token)])
def quota_get(x_preferred_account_email: str | None = Header(default=None)):
    # Direct call, not a request dispatch: the decorator's dependency already ran.
    return quota_refresh(x_preferred_account_email)


class HTTPExceptionish(Exception):
    def __init__(
        self,
        status: int,
        message: str,
        code: str,
        fault: str,
        *,
        error_ref: str | None = None,
    ):
        self.status = status
        error: dict[str, Any] = {
            "message": message,
            "type": "gateway_error",
            "code": code,
            "fault": fault,
        }
        # Correlation id only; the traceback stays in the process log.
        if error_ref:
            error["error_ref"] = error_ref
        self.payload = {"error": error}


@face.exception_handler(HTTPExceptionish)
async def _http_exc_handler(_req, exc: HTTPExceptionish):
    return JSONResponse(status_code=exc.status, content=exc.payload)


@face.post("/v1/chat/completions", dependencies=[Depends(require_face_token)])
def chat(
    body: ChatReq,
    x_preferred_account_email: str | None = Header(default=None),
):
    if body.stream:
        return JSONResponse(
            status_code=400,
            content={
                "error": {
                    "message": "stream not supported in MVP",
                    "type": "gateway_error",
                    "code": "stream_unsupported",
                    # Caller asked for an unsupported mode — not a helper defect,
                    # so it must not be charged to the self-fault SLO.
                    "fault": "client",
                }
            },
        )
    account = resolve_account(x_preferred_account_email)
    prompt = ""
    for m in reversed(body.messages or []):
        if m.get("role") == "user":
            c = m.get("content")
            prompt = c if isinstance(c, str) else str(c)
            break
    if not str(prompt).strip():
        return JSONResponse(
            status_code=400,
            content={
                "error": {
                    "message": "messages must include a user text",
                    "type": "gateway_error",
                    "code": "invalid_request",
                    # Malformed request body: client fault, not self.
                    "fault": "client",
                }
            },
        )
    r = execute_text(TextIn(account=account, prompt=prompt, model=body.model))
    if not r.get("ok"):
        fault = r.get("fault") or "upstream"
        code = 400 if fault == "client" else (500 if fault == "self" else 502)
        error: dict[str, Any] = {
            "message": r.get("error") or "text failed",
            "type": "gateway_error",
            "code": "text_failed",
            "fault": fault,
        }
        if r.get("error_ref"):
            error["error_ref"] = r["error_ref"]
        return JSONResponse(status_code=code, content={"error": error})
    return {
        "id": f"chatcmpl-{uuid.uuid4()}",
        "object": "chat.completion",
        "created": int(time.time()),
        # Echo the model actually sent upstream, which the bridge resolved and
        # returned; body.model is only a request hint and may not be honoured.
        "model": r.get("model") or body.model,
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": r.get("content") or ""},
                "finish_reason": "stop",
            }
        ],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
    }


@face.post("/v1/images/generations", dependencies=[Depends(require_face_token)])
def images(
    body: ImageReq,
    x_preferred_account_email: str | None = Header(default=None),
):
    if body.n != 1:
        return JSONResponse(
            status_code=400,
            content={
                "error": {
                    "message": "MVP only supports n=1",
                    "type": "gateway_error",
                    "code": "n_unsupported",
                    # Unsupported parameter value from the caller: client fault.
                    "fault": "client",
                }
            },
        )
    account = resolve_account(x_preferred_account_email)
    with _image_lock_for(account.email or ""):
        # Use cached quota only — do not open a second cold Session just to
        # re-probe before pool_sticky image (prod does not do that on sync face).
        q = _cached_quota(account, force=False)
        if not q.get("ok"):
            # One forced refresh on cache miss/failure, still before image.
            q = _cached_quota(account, force=True)
        if not q.get("ok"):
            fault = q.get("fault") or "upstream"
            code = 500 if fault == "self" else 502
            return JSONResponse(
                status_code=code,
                content={
                    "error": {
                        "message": q.get("error") or "quota refresh failed",
                        "type": "gateway_error",
                        "code": "quota_refresh_failed",
                        "fault": fault,
                    }
                },
            )
        if not q.get("imageable"):
            return JSONResponse(
                status_code=429,
                content={
                    "error": {
                        "message": (
                            f"image_quota_insufficient: remaining={q.get('remaining')} "
                            f"status={q.get('status')} min={q.get('min_remaining')}"
                        ),
                        "type": "gateway_error",
                        "code": "image_quota_insufficient",
                        "fault": "quota",
                        "quota": q,
                    }
                },
            )
        r = execute_image(
            ImageIn(
                account=account,
                prompt=body.prompt,
                model=body.model,
                size=body.size,
            ),
            skip_quota_gate=True,
        )
    if not r.get("ok"):
        fault = r.get("fault") or "upstream"
        if fault == "quota":
            code = 429
            err_code = "image_quota_insufficient"
        elif fault == "client":
            code = 400
            err_code = "invalid_request"
        elif fault == "self":
            code = 500
            err_code = "image_failed"
        else:
            code = 502
            err_code = "image_failed"
        error: dict[str, Any] = {
            "message": r.get("error") or "image failed",
            "type": "gateway_error",
            "code": err_code,
            "fault": fault,
            "quota": r.get("quota"),
        }
        if r.get("error_ref"):
            error["error_ref"] = r["error_ref"]
        return JSONResponse(status_code=code, content={"error": error})
    return {"created": int(time.time()), "data": [{"b64_json": r.get("b64_json")}]}


def main() -> None:
    import uvicorn

    pin()
    # Fail fast rather than starting an unauthenticated listener.
    if not (os.environ.get("HELPER_INTERNAL_TOKEN") or "").strip():
        raise RuntimeError(
            "HELPER_INTERNAL_TOKEN must be set; refusing to start with unauthenticated routes"
        )
    host, _, port_s = LISTEN.partition(":")
    uvicorn.run(face, host=host or "127.0.0.1", port=int(port_s or "8013"), log_level="info")


if __name__ == "__main__":
    main()
