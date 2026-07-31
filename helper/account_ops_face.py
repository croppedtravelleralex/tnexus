"""TNexus account ops HTTP face — OAuth / refresh / relogin (port 9011)."""
from __future__ import annotations

import hmac
import logging
import os
from typing import Any

from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

from account_ops import refresh_account, relogin_account
from oauth_login import OAuthLoginError, oauth_login_service

LISTEN = os.environ.get("ACCOUNT_OPS_LISTEN", "127.0.0.1:9011")
log = logging.getLogger("account_ops_face")
app = FastAPI(title="tnexus-account-ops", version="0.1.0")


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


@app.get("/health")
def health() -> dict[str, Any]:
    return {"ok": True, "service": "tnexus-account-ops"}


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


if __name__ == "__main__":
    import uvicorn

    logging.basicConfig(level=logging.INFO)
    uvicorn.run(app, host=LISTEN.split(":")[0], port=int(LISTEN.split(":")[-1]))
