"""TNexus account ops HTTP face — OAuth / refresh / relogin / nurture / outlook / quota prime (port 9011)."""
from __future__ import annotations

import hmac
import logging
import os
from contextlib import asynccontextmanager
from typing import Any

from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

from account_ops import refresh_account, relogin_account
from oauth_login import OAuthLoginError, oauth_login_service

LISTEN = os.environ.get("ACCOUNT_OPS_LISTEN", "127.0.0.1:9011")
log = logging.getLogger("account_ops_face")


def require_token(x_account_ops_token: str | None = Header(default=None)) -> None:
    expected = os.environ.get("ACCOUNT_OPS_TOKEN") or os.environ.get("HELPER_INTERNAL_TOKEN") or ""
    if not expected.strip():
        raise HTTPException(status_code=503, detail={"error": "ACCOUNT_OPS_TOKEN not configured"})
    if not hmac.compare_digest(str(x_account_ops_token or ""), expected):
        raise HTTPException(status_code=401, detail={"error": "invalid X-Account-Ops-Token"})


class OAuthStartIn(BaseModel):
    email_hint: str = ""


class OAuthFinishIn(BaseModel):
    session_id: str
    callback: str


class AccountIn(BaseModel):
    account: dict[str, Any] = Field(default_factory=dict)


class NurtureEnqueueIn(BaseModel):
    prompt: str = ""
    source: str = "tnexus_ui"
    access_tokens: list[str] = Field(default_factory=list)
    accounts: list[dict[str, Any]] = Field(default_factory=list)


class NurtureEnableIn(BaseModel):
    enabled: bool


class NurtureProcessOneIn(BaseModel):
    prompt: str = ""
    access_token: str = ""
    email: str = ""
    source: str = ""
    account: dict[str, Any] = Field(default_factory=dict)


class OutlookRecoverIn(BaseModel):
    access_token: str
    account: dict[str, Any] = Field(default_factory=dict)


class OutlookAutoRecoveryIn(BaseModel):
    enabled: bool | None = None
    interval_sec: int | None = None
    max_per_cycle: int | None = None


class QuotaPrimeIn(BaseModel):
    access_tokens: list[str] = Field(default_factory=list)
    mode: str = "manual"
    accounts: list[dict[str, Any]] = Field(default_factory=list)


@asynccontextmanager
async def lifespan(_app: FastAPI):
    try:
        from ops_bridge import start_background_services

        result = start_background_services()
        log.info("background ops started: %s", result)
    except Exception as exc:
        log.warning("background ops startup skipped: %s", exc)
    yield


app = FastAPI(title="tnexus-account-ops", version="0.2.0", lifespan=lifespan)


@app.get("/health")
def health() -> dict[str, Any]:
    return {"ok": True, "service": "tnexus-account-ops", "ops_bridge": True}


@app.post("/v1/oauth/start", dependencies=[Depends(require_token)])
def oauth_start(body: OAuthStartIn) -> dict[str, Any]:
    return oauth_login_service.start(body.email_hint)


@app.post("/v1/oauth/finish", dependencies=[Depends(require_token)])
def oauth_finish(body: OAuthFinishIn) -> dict[str, Any]:
    try:
        tokens = oauth_login_service.finish(body.session_id, body.callback)
    except OAuthLoginError as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)}) from exc
    return {
        "access_token": tokens["access_token"],
        "refresh_token": tokens["refresh_token"],
        "id_token": tokens.get("id_token") or "",
        "source_type": "oauth_login",
    }


@app.post("/v1/accounts/refresh-one", dependencies=[Depends(require_token)])
def refresh_one(body: AccountIn) -> dict[str, Any]:
    try:
        updated = refresh_account(body.account)
    except Exception as exc:
        log.exception("refresh_one failed")
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc
    return {"ok": True, "account": updated}


@app.post("/v1/accounts/relogin-one", dependencies=[Depends(require_token)])
def relogin_one(body: AccountIn) -> dict[str, Any]:
    try:
        updated = relogin_account(body.account)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)}) from exc
    except Exception as exc:
        log.exception("relogin_one failed")
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc
    return {"ok": True, "account": updated}


@app.get("/v1/nurture/status", dependencies=[Depends(require_token)])
def nurture_status() -> dict[str, Any]:
    try:
        from ops_bridge import nurture_status as _status

        return _status()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/nurture/enable", dependencies=[Depends(require_token)])
def nurture_enable(body: NurtureEnableIn) -> dict[str, Any]:
    try:
        from ops_bridge import nurture_enable as _enable

        return _enable(body.enabled)
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/nurture/enqueue", dependencies=[Depends(require_token)])
def nurture_enqueue(body: NurtureEnqueueIn) -> dict[str, Any]:
    try:
        from ops_bridge import nurture_enqueue as _enqueue

        return _enqueue(
            prompt=body.prompt,
            source=body.source,
            access_tokens=body.access_tokens,
            accounts=body.accounts,
        )
    except Exception as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/nurture/process-one", dependencies=[Depends(require_token)])
def nurture_process_one(body: NurtureProcessOneIn) -> dict[str, Any]:
    try:
        from ops_bridge import nurture_process_one as _process

        payload = {
            "prompt": body.prompt,
            "access_token": body.access_token,
            "email": body.email,
            "source": body.source or "tnexus_accounts_ui",
        }
        if body.account:
            payload["account"] = body.account
        return _process(payload)
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail={"error": str(exc)[:500]}) from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.get("/v1/outlook/auto-recovery/status", dependencies=[Depends(require_token)])
def outlook_auto_recovery_status() -> dict[str, Any]:
    try:
        from ops_bridge import outlook_auto_recovery_status as _status

        return _status()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/outlook/auto-recovery/settings", dependencies=[Depends(require_token)])
def outlook_auto_recovery_settings(body: OutlookAutoRecoveryIn) -> dict[str, Any]:
    try:
        from ops_bridge import outlook_auto_recovery_update as _update

        return _update(body.model_dump(exclude_none=True))
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/outlook/recover-one", dependencies=[Depends(require_token)])
def outlook_recover_one(body: OutlookRecoverIn) -> dict[str, Any]:
    try:
        from ops_bridge import outlook_recover_one as _recover

        return _recover(body.access_token, body.account or None)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)}) from exc
    except RuntimeError as exc:
        raise HTTPException(status_code=409, detail={"error": str(exc)}) from exc
    except (FileNotFoundError, PermissionError) as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)}) from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.get("/v1/outlook/recover/progress/{progress_id}", dependencies=[Depends(require_token)])
def outlook_recover_progress(progress_id: str) -> dict[str, Any]:
    try:
        from ops_bridge import outlook_recover_progress as _progress

        row = _progress(progress_id)
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc
    if row is None:
        raise HTTPException(status_code=404, detail={"error": "progress not found"})
    return row


@app.post("/v1/quota-window/prime", dependencies=[Depends(require_token)])
def quota_window_prime(body: QuotaPrimeIn) -> dict[str, Any]:
    try:
        from ops_bridge import quota_prime_enqueue as _prime

        return _prime(body.access_tokens, mode=body.mode, accounts=body.accounts)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)}) from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.get("/v1/quota-window/prime/status", dependencies=[Depends(require_token)])
def quota_window_prime_status() -> dict[str, Any]:
    try:
        from ops_bridge import quota_prime_status as _status

        return _status()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


class ProxyRuntimeIn(BaseModel):
    model_config = {"extra": "allow"}


class ProxyTestIn(BaseModel):
    url: str = ""


@app.get("/v1/proxy/runtime", dependencies=[Depends(require_token)])
def proxy_runtime_get() -> dict[str, Any]:
    try:
        from ops_bridge import proxy_runtime_get as _get

        return _get()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/proxy/runtime", dependencies=[Depends(require_token)])
def proxy_runtime_save(body: ProxyRuntimeIn) -> dict[str, Any]:
    try:
        from ops_bridge import proxy_runtime_save as _save

        return _save(body.model_dump(mode="python"))
    except ValueError as exc:
        raise HTTPException(status_code=400, detail={"error": str(exc)[:500]}) from exc
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/proxy/test", dependencies=[Depends(require_token)])
def proxy_test_endpoint(body: ProxyTestIn) -> dict[str, Any]:
    try:
        from ops_bridge import proxy_test as _test

        return _test(body.url)
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


@app.get("/v1/webshare-cf-scan/status", dependencies=[Depends(require_token)])
def webshare_cf_scan_status() -> dict[str, Any]:
    try:
        from ops_bridge import webshare_cf_scan_status as _status

        return _status()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


@app.get("/v1/webshare-cf-scan/inventory", dependencies=[Depends(require_token)])
def webshare_cf_scan_inventory() -> dict[str, Any]:
    try:
        from ops_bridge import webshare_cf_scan_inventory as _inventory

        return _inventory()
    except Exception as exc:
        raise HTTPException(status_code=503, detail={"error": str(exc)[:500]}) from exc


@app.post("/v1/webshare-cf-scan/run-once", dependencies=[Depends(require_token)])
def webshare_cf_scan_run_once() -> dict[str, Any]:
    try:
        from ops_bridge import webshare_cf_scan_run_once as _run

        return _run()
    except Exception as exc:
        raise HTTPException(status_code=502, detail={"error": str(exc)[:500]}) from exc


if __name__ == "__main__":
    import uvicorn

    logging.basicConfig(level=logging.INFO)
    uvicorn.run(app, host=LISTEN.split(":")[0], port=int(LISTEN.split(":")[-1]))
