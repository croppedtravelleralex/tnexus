"""Protocol bridge for gptimage-gateway-rs MVP.

PROTO_BRIDGE: executes pinned-account text/image via gptimage OpenAIBackendAPI.
Rust gateway orchestrates OpenAI HTTP shape; this process owns curl_cffi + PoW/SSE.

Env:
  GPTIMAGE_ROOT   path to gptimage checkout (default: sibling ../gptimage)
  HELPER_LISTEN   default 127.0.0.1:19001
  CHATGPT2API_AUTH_KEY or gptimage config.json auth-key (required by services.config)
  HELPER_INTERNAL_TOKEN  shared secret for every /v1/internal/* route. Unset =>
      those routes answer 503 and serve nothing (fail closed, not fail open).
  HELPER_ALLOW_CALLER_PROXY  opt-in (default off) to let HTTP callers pick the
      egress proxy; leaving it off keeps the helper from acting as an open proxy.

Callers of /v1/internal/* (crates/helper_client) must send the shared secret in
the ``X-Helper-Token`` request header, and must no longer expect ``access_token``
or a full ``proxy`` URL back from /v1/internal/accounts/candidates.
"""
from __future__ import annotations

import hmac
import logging
import os
import sys
import threading
import time
import uuid
from pathlib import Path
from typing import Any

from fastapi import Depends, FastAPI, Header, HTTPException
from fastapi.responses import StreamingResponse
from pydantic import BaseModel

# --- path bootstrap ---------------------------------------------------------
ROOT = Path(__file__).resolve().parents[1]
GPTIMAGE_ROOT = Path(os.environ.get("GPTIMAGE_ROOT") or (ROOT.parent / "gptimage")).resolve()
if str(GPTIMAGE_ROOT) not in sys.path:
    sys.path.insert(0, str(GPTIMAGE_ROOT))
os.chdir(GPTIMAGE_ROOT)

_cfg = GPTIMAGE_ROOT / "config.json"
if not os.environ.get("CHATGPT2API_AUTH_KEY") and _cfg.is_file():
    import json as _json

    try:
        _ak = str(_json.loads(_cfg.read_text(encoding="utf-8")).get("auth-key") or "").strip()
        if _ak:
            os.environ["CHATGPT2API_AUTH_KEY"] = _ak
    except Exception:
        pass

from curl_cffi import requests as curl_requests  # noqa: E402
from services.account_fingerprint import ensure_complete_fp  # noqa: E402
from services.openai_backend_api import OpenAIBackendAPI  # noqa: E402
from services.protocol.conversation import (  # noqa: E402
    ConversationRequest,
    collect_image_outputs,
    conversation_events,
    stream_image_outputs,
)
from services.proxy_service import proxy_settings  # noqa: E402

app = FastAPI(title="gptimage-gateway-rs-helper", version="0.1.0")

log = logging.getLogger("protocol_bridge")


def require_internal_token(x_helper_token: str | None = Header(default=None)) -> None:
    """Gate for /v1/internal/*: these routes hand out account-scoped execution.

    HELPER_INTERNAL_TOKEN unset is treated as misconfiguration, not as "auth
    disabled" — an unconfigured deploy must serve nothing rather than everything.
    """
    expected = os.environ.get("HELPER_INTERNAL_TOKEN") or ""
    if not expected.strip():
        raise HTTPException(
            status_code=503,
            detail={
                "error": {
                    "message": (
                        "helper internal API disabled: HELPER_INTERNAL_TOKEN is not set. "
                        "Set it on the helper process and send it as X-Helper-Token."
                    ),
                    "type": "helper_error",
                    "code": "internal_token_unconfigured",
                    "fault": "self",
                }
            },
        )
    # compare_digest keeps the reject path constant-time so a wrong token cannot
    # be recovered byte-by-byte from response latency.
    if not hmac.compare_digest(str(x_helper_token or ""), expected):
        raise HTTPException(
            status_code=401,
            detail={
                "error": {
                    "message": "missing or invalid X-Helper-Token",
                    "type": "helper_error",
                    "code": "internal_token_invalid",
                    "fault": "client",
                }
            },
        )


def _error_ref(exc: BaseException, event: str) -> str:
    """Log the traceback server-side and return only a correlation id.

    Tracebacks carry absolute paths and upstream internals, so they stay in the
    process log; the HTTP body gets nothing but this id.
    """
    ref = uuid.uuid4().hex[:12]
    log.exception("%s error_ref=%s: %s: %s", event, ref, type(exc).__name__, exc)
    return ref


# gptimage's `config` is a process-wide singleton whose mutations are persisted by
# ConfigStore._save(). Any per-request override must therefore be serialized and
# rolled back, or concurrent image requests overwrite each other and the last
# writer can leak into config.json.
_CONFIG_OVERRIDE_LOCK = threading.Lock()

# Per-email lock for direct-token image path (Rust face). Avoids double-hit on
# the same account without touching prod get_available_access_token slots.
_email_locks_guard = threading.Lock()
_email_locks: dict[str, threading.Lock] = {}


def _lock_for_email(email: str) -> threading.Lock:
    key = (email or "").strip().lower() or "_"
    with _email_locks_guard:
        lock = _email_locks.get(key)
        if lock is None:
            lock = threading.Lock()
            _email_locks[key] = lock
        return lock


class AccountIn(BaseModel):
    email: str = ""
    # When set (Rust candidates), execute_image uses direct_token path.
    access_token: str = ""
    device_id: str | None = None
    proxy: str | None = None
    user_agent: str | None = None


class TextIn(BaseModel):
    account: AccountIn
    prompt: str
    model: str = "gpt-4o-mini"


class ImageIn(BaseModel):
    account: AccountIn
    prompt: str
    model: str = "gpt-image-2"
    size: str = "1024x1024"


class QuotaIn(BaseModel):
    account: AccountIn
    # minimum remaining required for imageable=true
    min_remaining: int = 1


def _min_image_quota() -> int:
    try:
        return max(0, int(os.environ.get("MVP_MIN_IMAGE_QUOTA", "1")))
    except Exception:
        return 1


def _classify_fault(exc: BaseException) -> str:
    """Map an exception onto the contract taxonomy (client / self / upstream).

    `client` must be reachable: request-shaped failures (bad params, unparseable
    or incomplete caller JSON) are not helper defects and must not be charged to
    the self-fault SLO. Order matters — the client probe runs before the generic
    TypeError/KeyError arm, which would otherwise swallow validation errors.
    """
    name = type(exc).__name__.lower()
    msg = str(exc).lower()
    if any(
        x in name
        for x in ("validation", "valueerror", "unicodedecode", "jsondecode", "badrequest")
    ):
        return "client"
    if any(
        x in msg
        for x in (
            "requires account email",
            "invalid_request",
            "must include",
            "unsupported",
            "is required",
            "missing required",
            "field required",
        )
    ):
        return "client"
    if any(x in name for x in ("attribute", "type", "key", "assertion")):
        return "self"
    if "helper" in msg or "pin_" in msg:
        return "self"
    return "upstream"


def _allow_caller_proxy() -> bool:
    """Whether an HTTP caller may choose the egress proxy for its own request.

    Off by default: honouring caller-supplied proxies turns the helper into an
    open forward proxy for anyone who can reach it. Server-side per-account
    binding stays authoritative unless an operator explicitly opts in.
    """
    return str(os.environ.get("HELPER_ALLOW_CALLER_PROXY", "")).strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


def make_backend(acc: AccountIn) -> OpenAIBackendAPI:
    # Prefer in-pool account identity (complete fp/proxy binding). Pin-only inject
    # can leave SSE hanging after conversation_id with tool_invoked=null.
    try:
        from services.account_service import account_service

        email = (acc.email or "").strip().lower()
        if email:
            for row in account_service.list_accounts():
                if str(row.get("email") or "").strip().lower() != email:
                    continue
                token = str(row.get("access_token") or acc.access_token or "").strip()
                if token:
                    api = OpenAIBackendAPI(access_token=token)
                    try:
                        logger = __import__("logging").getLogger("protocol_bridge")
                        logger.info({
                            "event": "mvp_backend_identity",
                            "source": "pool",
                            "email": email,
                            "proxy_host": str((api.account or {}).get("proxy") or "").split("@")[-1][:80],
                        })
                    except Exception:
                        pass
                    return api
    except Exception as exc:
        try:
            __import__("logging").getLogger("protocol_bridge").warning({
                "event": "mvp_backend_pool_fallback",
                "error": f"{type(exc).__name__}: {exc}"[:240],
            })
        except Exception:
            pass

    api = OpenAIBackendAPI(access_token=acc.access_token)
    # Caller-supplied proxy is ignored unless explicitly opted in; the pool row
    # for this email (resolved above) is the intended egress binding.
    proxy = (acc.proxy or "").strip() if _allow_caller_proxy() else ""
    account: dict[str, Any] = {
        "email": acc.email or "",
        "proxy": proxy,
        "access_token": acc.access_token,
        "oai-device-id": acc.device_id or "",
        "user-agent": acc.user_agent or "",
        "fp": {},
    }
    if acc.device_id:
        account["fp"]["oai-device-id"] = acc.device_id
    if acc.user_agent:
        account["fp"]["user-agent"] = acc.user_agent
    api.account = account
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
    api.session.headers.update(
        {
            "User-Agent": api.user_agent,
            "Accept-Language": api._accept_language(),
        }
    )
    try:
        __import__("logging").getLogger("protocol_bridge").info({
            "event": "mvp_backend_identity",
            "source": "pin",
            "email": (acc.email or "").strip().lower(),
            "proxy_host": proxy.split("@")[-1][:80],
        })
    except Exception:
        pass
    return api


@app.get("/health")
def health() -> dict[str, Any]:
    return {
        "ok": True,
        "service": "protocol-bridge",
        "proto_bridge": True,
        "gptimage_root": str(GPTIMAGE_ROOT),
    }


def execute_quota(body: QuotaIn) -> dict[str, Any]:
    """Live refresh via OpenAIBackendAPI.get_user_info (conversation/init limits_progress)."""
    t0 = time.time()
    min_remaining = max(0, int(body.min_remaining if body.min_remaining is not None else _min_image_quota()))
    api = make_backend(body.account)
    try:
        info = api.get_user_info()
        remaining = int(info.get("quota") or 0)
        unknown = bool(info.get("image_quota_unknown"))
        status = str(info.get("status") or "")
        plan = str(info.get("type") or "")
        image_gen = [
            x
            for x in (info.get("limits_progress") or [])
            if isinstance(x, dict) and str(x.get("feature_name") or "") == "image_gen"
        ]
        # Sufficient only when known remaining meets threshold (unknown ≠ free pass for MVP).
        imageable = (not unknown) and remaining >= min_remaining and status != "限流"
        return {
            "ok": True,
            "email": info.get("email") or body.account.email,
            "plan": plan,
            "status": status,
            "remaining": remaining,
            "restore_at": info.get("restore_at"),
            "image_quota_unknown": unknown,
            "min_remaining": min_remaining,
            "imageable": imageable,
            "image_gen": image_gen,
            "fault": None,
            "error": None,
            "elapsed_ms": int((time.time() - t0) * 1000),
        }
    except Exception as e:
        return {
            "ok": False,
            "email": body.account.email,
            "plan": None,
            "status": None,
            "remaining": None,
            "restore_at": None,
            "image_quota_unknown": None,
            "min_remaining": min_remaining,
            "imageable": False,
            "image_gen": [],
            "fault": _classify_fault(e),
            "error": f"{type(e).__name__}: {e}"[:800],
            "elapsed_ms": int((time.time() - t0) * 1000),
            "error_ref": _error_ref(e, "quota_refresh_failed"),
        }
    finally:
        try:
            api.close()
        except Exception:
            pass


# Upstream text slugs gptimage will forward as-is (services/protocol/openai_v1_models.py).
# Anything else — including the OpenAI-shaped aliases callers send, e.g.
# "gpt-4o-mini" — has no upstream equivalent and must degrade to "auto".
_UPSTREAM_TEXT_MODELS = frozenset(
    {"auto", "gpt-5", "gpt-5-1", "gpt-5-2", "gpt-5-3", "gpt-5-3-mini", "gpt-5-mini"}
)


def _resolve_text_model(requested: str) -> str:
    """Model actually sent upstream. Responses must echo this, not the request."""
    name = str(requested or "").strip().lower()
    return name if name in _UPSTREAM_TEXT_MODELS else "auto"


def execute_text(body: TextIn) -> dict[str, Any]:
    t0 = time.time()
    api = make_backend(body.account)
    model = _resolve_text_model(body.model)
    try:
        cid = ""
        text = ""
        messages = [{"role": "user", "content": body.prompt}]
        for ev in conversation_events(api, messages=messages, model=model):
            cid = ev.get("conversation_id") or cid
            if ev.get("type") == "conversation.delta":
                text += ev.get("delta") or ""
            if ev.get("type") == "conversation.done":
                text = ev.get("text") or text
        ok = bool(str(text).strip())
        return {
            "ok": ok,
            "content": text,
            "conversation_id": cid,
            "model": model,
            "fault": None if ok else "upstream",
            "error": None if ok else "empty assistant content",
            "elapsed_ms": int((time.time() - t0) * 1000),
        }
    except Exception as e:
        return {
            "ok": False,
            "content": None,
            "conversation_id": None,
            "model": model,
            "fault": _classify_fault(e),
            "error": f"{type(e).__name__}: {e}"[:800],
            "elapsed_ms": int((time.time() - t0) * 1000),
            "error_ref": _error_ref(e, "text_failed"),
        }
    finally:
        try:
            api.close()
        except Exception:
            pass


def _mvp_post_ready_secs() -> float | None:
    """Resolve the MVP post_ready value from env. Constant for the process."""
    raw = os.environ.get("MVP_IMAGE_SSE_POST_READY_SECS", "50").strip()
    if raw.lower() in {"", "none", "null", "off", "0"}:
        return None
    try:
        return max(30.0, float(raw))
    except ValueError:
        return 50.0


_post_ready_applied = False


def _apply_mvp_sse_post_ready() -> float | None:
    """MVP soft post_ready after conversation_id.

    Prefer complete_predicate (file_id) early-exit; soft post_ready is a safety
    valve to leave SSE for poll without hard-waiting EOF (~90s) or hanging past
    the client wall. Default 50s aligns with ~40-60s healthy e2e + poll residual.

    Two constraints drive the shape of this override:

    1. ``config.data`` is what ``ConfigStore._save()`` serializes into the live
       config.json, and any ``config.update()`` from an unrelated code path
       copies ``data`` wholesale before saving. Injecting there means an MVP-only
       tuning value can be written into production config permanently. So the
       override is installed on the class descriptor instead — ``data`` is never
       touched and there is nothing for ``_save()`` to pick up.
    2. ``openai_backend_api`` reads this only through the module-level ``config``
       singleton; there is no per-call parameter to thread it through. Since the
       value is derived from an env var it is identical for every request, so it
       is applied exactly once under a lock rather than rewritten per request —
       which is what previously let 40 pooled threads clobber each other.
    """
    global _post_ready_applied
    secs = _mvp_post_ready_secs()
    if _post_ready_applied:
        return secs
    with _CONFIG_OVERRIDE_LOCK:
        if _post_ready_applied:
            return secs
        try:
            from services.config import config as cfg

            type(cfg).image_sse_post_ready_timeout_secs = property(lambda _self: secs)
        except Exception:
            log.warning("mvp post_ready override not installed; using gptimage default")
        _post_ready_applied = True
    return secs


def execute_image(body: ImageIn, *, skip_quota_gate: bool = False) -> dict[str, Any]:
    t0 = time.time()
    q: dict[str, Any] | None = None
    # Hard gate: refresh live quota; refuse before prepare/SSE when not imageable.
    if not skip_quota_gate:
        q = execute_quota(QuotaIn(account=body.account, min_remaining=_min_image_quota()))
        if not q.get("ok"):
            return {
                "ok": False,
                "b64_json": None,
                "conversation_id": None,
                "fault": q.get("fault") or "upstream",
                "error": f"quota_refresh_failed: {q.get('error')}",
                "elapsed_ms": int((time.time() - t0) * 1000),
                "quota": q,
            }
        if not q.get("imageable"):
            return {
                "ok": False,
                "b64_json": None,
                "conversation_id": None,
                "fault": "quota",
                "error": (
                    f"image_quota_insufficient: remaining={q.get('remaining')} "
                    f"status={q.get('status')} min={q.get('min_remaining')} restore_at={q.get('restore_at')}"
                ),
                "elapsed_ms": int((time.time() - t0) * 1000),
                "quota": q,
            }

    post_ready = _apply_mvp_sse_post_ready()
    poll_timeout = float(os.environ.get("MVP_IMAGE_POLL_TIMEOUT_SECS", "90"))
    cancel_event = threading.Event()
    wall = float(os.environ.get("MVP_IMAGE_WALL_SECS", "120"))
    prefer = (body.account.email or "").strip()
    token_in = (body.account.access_token or "").strip()
    force_sticky = str(os.environ.get("MVP_FORCE_POOL_STICKY", "")).strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }
    token = ""
    api: OpenAIBackendAPI | None = None
    slot_released = False
    # Default: direct make_backend (pool identity by email). Avoid helper-local
    # get_available_access_token which silently falls through sticky → empty ready.
    use_direct = (not force_sticky) or bool(token_in)
    path = "direct_token" if use_direct else "pool_sticky"
    email_lock: threading.Lock | None = None

    def _arm_wall() -> None:
        if cancel_event.wait(timeout=max(5.0, wall)):
            return
        cancel_event.set()

    wall_thread = threading.Thread(target=_arm_wall, name="mvp-image-wall", daemon=True)
    wall_thread.start()
    try:
        from services.account_service import account_service

        if not prefer:
            raise RuntimeError("image requires account email")

        if use_direct:
            email_lock = _lock_for_email(prefer)
            if not email_lock.acquire(
                blocking=True,
                timeout=float(os.environ.get("MVP_EMAIL_LOCK_SECS", "90")),
            ):
                raise RuntimeError(f"email lock timeout: {prefer}")
            api = make_backend(body.account)
        else:
            token = account_service.get_available_access_token(preferred_email=prefer)
            got = account_service.get_account(token) or {}
            got_email = str(got.get("email") or "").strip().lower()
            if got_email != prefer.lower():
                try:
                    account_service.release_image_slot(token)
                except Exception:
                    pass
                token = ""
                raise RuntimeError(
                    f"preferred sticky miss: wanted={prefer} got={got.get('email')}"
                )
            api = OpenAIBackendAPI(access_token=token)

        api.cancel_event = cancel_event
        outs = list(
            stream_image_outputs(
                api,
                ConversationRequest(
                    prompt=body.prompt,
                    model=body.model or "gpt-image-2",
                    n=1,
                    size=body.size or "1024x1024",
                    response_format="b64_json",
                    poll_timeout_secs=poll_timeout,
                    cancel_event=cancel_event,
                ),
            )
        )
        result = collect_image_outputs(outs)
        cid = next(
            (
                getattr(o, "conversation_id", None)
                for o in outs
                if getattr(o, "conversation_id", None)
            ),
            "",
        ) or ""
        data = result.get("data") or [{}]
        b64 = ""
        if data and isinstance(data[0], dict):
            b64 = str(data[0].get("b64_json") or "")
        ok = len(b64) > 1000
        if token:
            try:
                account_service.mark_image_result(token, bool(ok))
                slot_released = True
            except Exception:
                pass
        return {
            "ok": ok,
            "b64_json": b64 if ok else None,
            "conversation_id": cid,
            "fault": None if ok else "upstream",
            "error": None if ok else (result.get("message") or "empty/short b64"),
            "elapsed_ms": int((time.time() - t0) * 1000),
            "quota": q,
            "timing": {
                "post_ready_secs": post_ready,
                "poll_timeout_secs": poll_timeout,
                "wall_secs": wall,
                "path": path,
                "preferred_email": prefer,
            },
        }
    except Exception as e:
        name = type(e).__name__
        msg = str(e)
        timed = cancel_event.is_set() and ("cancel" in msg.lower() or "Cancel" in name)
        fault = _classify_fault(e)
        if (
            "concurrency limit" in msg.lower()
            or "sticky miss" in msg.lower()
            or "email lock" in msg.lower()
        ):
            fault = "self"
        if token and not slot_released:
            try:
                from services.account_service import account_service

                account_service.mark_image_result(token, False)
                slot_released = True
            except Exception:
                pass
        return {
            "ok": False,
            "b64_json": None,
            "conversation_id": getattr(e, "conversation_id", None),
            "fault": fault,
            "error": (
                f"image_wall_timeout_{wall:.0f}s"
                if timed
                else f"{name}: {msg}"[:800]
            ),
            "elapsed_ms": int((time.time() - t0) * 1000),
            "error_ref": _error_ref(e, "image_failed"),
            "quota": q,
            "timing": {
                "post_ready_secs": post_ready,
                "poll_timeout_secs": poll_timeout,
                "wall_secs": wall,
                "path": path,
                "preferred_email": prefer,
            },
        }
    finally:
        cancel_event.set()
        if email_lock is not None:
            try:
                email_lock.release()
            except Exception:
                pass
        if token and not slot_released:
            try:
                from services.account_service import account_service

                account_service.release_image_slot(token)
            except Exception:
                pass
        if api is not None:
            try:
                api.close()
            except Exception:
                pass

def execute_text_stream(body: TextIn):
    """Yield OpenAI-compatible SSE chunks from conversation_events.

    ``conversation.done`` carries the accumulated full text, not a tail delta —
    the same field the non-streaming path assigns (replace semantics). Emitting
    it as a delta would hand the client every chunk twice, so the done event only
    terminates the stream here. Both paths therefore end with identical text.
    """
    import json as _json

    api = make_backend(body.account)
    model = _resolve_text_model(body.model)
    try:
        cid = ""
        chunk_id = f"chatcmpl-{int(time.time())}"
        messages = [{"role": "user", "content": body.prompt}]
        for ev in conversation_events(api, messages=messages, model=model):
            cid = ev.get("conversation_id") or cid
            if ev.get("type") != "conversation.delta":
                continue
            delta = ev.get("delta") or ""
            if not delta:
                continue
            payload = {
                "id": chunk_id,
                "object": "chat.completion.chunk",
                "created": int(time.time()),
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {"content": delta},
                    "finish_reason": None,
                }],
            }
            yield f"data: {_json.dumps(payload, ensure_ascii=False)}\n\n"
        done = {
            "id": chunk_id,
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        }
        yield f"data: {_json.dumps(done)}\n\n"
        yield "data: [DONE]\n\n"
    except Exception as e:
        err = {
            "error": {
                "message": f"{type(e).__name__}: {e}"[:800],
                "type": "bridge_error",
                "code": "text_stream_failed",
                "fault": _classify_fault(e),
                "error_ref": _error_ref(e, "text_stream_failed"),
            }
        }
        yield f"data: {_json.dumps(err)}\n\n"
        yield "data: [DONE]\n\n"
    finally:
        try:
            api.close()
        except Exception:
            pass


@app.post("/v1/internal/text", dependencies=[Depends(require_internal_token)])
def run_text(body: TextIn) -> dict[str, Any]:
    return execute_text(body)


@app.post("/v1/internal/text/stream", dependencies=[Depends(require_internal_token)])
def run_text_stream(body: TextIn):
    return StreamingResponse(
        execute_text_stream(body),
        media_type="text/event-stream",
        headers={"Cache-Control": "no-cache", "Connection": "keep-alive"},
    )


@app.post("/v1/internal/image", dependencies=[Depends(require_internal_token)])
def run_image(body: ImageIn) -> dict[str, Any]:
    return execute_image(body)


@app.post("/v1/internal/quota/refresh", dependencies=[Depends(require_internal_token)])
def run_quota(body: QuotaIn) -> dict[str, Any]:
    return execute_quota(body)


@app.get(
    "/v1/internal/accounts/candidates",
    dependencies=[Depends(require_internal_token)],
)
def list_candidates(limit: int = 20) -> dict[str, Any]:
    """Unique-proxy pool accounts for Rust multi-account concurrent tests.

    Deliberately credential-free: no ``access_token`` and no proxy URL (which
    embeds ``user:pass@``). Callers address an account by ``email`` and the
    helper resolves the pool row itself, so nothing here needs the secrets.
    Presence is reported via ``has_token`` for selection purposes only.
    """
    limit = max(1, min(100, int(limit or 20)))
    try:
        from services.account_service import account_service

        rows = account_service.list_accounts()
    except Exception as exc:
        return {
            "ok": False,
            "count": 0,
            "accounts": [],
            "fault": "self",
            "error": "list_accounts failed",
            "error_ref": _error_ref(exc, "candidates_list_failed"),
        }
    out: list[dict[str, Any]] = []
    seen: set[str] = set()
    for row in rows:
        email = str(row.get("email") or "").strip()
        token = str(row.get("access_token") or "").strip()
        proxy = str(row.get("proxy") or "").strip()
        status = str(row.get("status") or "")
        if not email or not token or not proxy:
            continue
        if status in {"禁用", "异常", "限流"}:
            continue
        host = proxy.split("@")[-1].split(":")[0].lower()
        if not host or host in seen:
            continue
        seen.add(host)
        out.append(
            {
                "email": email,
                "proxy_host": host,
                "status": status,
                "quota": row.get("quota"),
                "has_token": True,
                "device_id": str(
                    row.get("oai-device-id")
                    or ((row.get("fp") or {}) if isinstance(row.get("fp"), dict) else {}).get("oai-device-id")
                    or ""
                )
                or None,
                "user_agent": str(
                    row.get("user-agent")
                    or ((row.get("fp") or {}) if isinstance(row.get("fp"), dict) else {}).get("user-agent")
                    or ""
                )
                or None,
            }
        )
        if len(out) >= limit:
            break
    return {"ok": True, "count": len(out), "accounts": out}


def main() -> None:
    import uvicorn

    # Fail fast rather than starting a listener whose /v1/internal/* routes would
    # all answer 503; a silent 503-only service is harder to diagnose than a
    # refusal at boot.
    if not (os.environ.get("HELPER_INTERNAL_TOKEN") or "").strip():
        raise RuntimeError(
            "HELPER_INTERNAL_TOKEN must be set; refusing to start with unauthenticated routes"
        )
    listen = os.environ.get("HELPER_LISTEN", "127.0.0.1:19001")
    host, _, port_s = listen.partition(":")
    uvicorn.run(app, host=host or "127.0.0.1", port=int(port_s or "19001"), log_level="info")


if __name__ == "__main__":
    main()
