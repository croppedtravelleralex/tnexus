#!/usr/bin/env python3
"""Grok 纯 HTTP 客户端：session 提取 → Python/Node 签名 → upload-file → conversations/new|responses。

不依赖浏览器发请求（session 提取需 Playwright 一次）。
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any, Literal

from curl_cffi import requests as crequests

def _detect_root() -> Path:
    if raw := os.environ.get("GROK_TNEXUS_ROOT", "").strip():
        return Path(raw)
    here = Path(__file__).resolve().parent
    for candidate in (here.parent, Path("/root/TNexus")):
        if (candidate / "crates" / "grok-signer").is_dir():
            return candidate
    return here.parent


ROOT = _detect_root()
GROK_ROOT = Path(os.environ.get("GROK_WEB_ROOT", ROOT))
CANARY = Path(
    os.environ.get(
        "GROK_CANARY",
        r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py",
    )
)
BUNDLE = ROOT / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js"
MODULE = BUNDLE.parent / "grok_sign_module_1645e3.js"
KEYS_DIR = Path(os.environ.get("GROK_KEYS_DIR", str(ROOT / "reports" / "pure_http_keys")))
DEFAULT_OCR_IMAGE = Path(
    os.environ.get(
        "GROK_OCR_PROBE_IMAGE",
        r"C:\Users\Lenovo\Downloads\image-1785287126849-88e3a45901dc98-1785287699703-649ee24e9542d8.png",
    )
)
DEFAULT_OCR_PROMPT = "提取图中全部可见文字，若无文字则描述画面。"
EPOCH = 1682924400
UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
)
PROXY = os.environ.get("GROK_LOCAL_PROXY", "http://127.0.0.1:7897")
UPSTREAM_PROXY = os.environ.get("GROK_UPSTREAM_PROXY", "")
PROXIES = {"http": PROXY, "https": PROXY}
UPSTREAM_PROXIES = (
    {"http": UPSTREAM_PROXY, "https": UPSTREAM_PROXY} if UPSTREAM_PROXY.strip() else PROXIES
)
IMPERSONATE = "chrome131"


def chat_payload(message: str, *, enable_image: bool = False) -> dict:
    """对齐 web_http_chat_image_canary.v1.py::chat_payload（内联，免外部依赖）。"""
    return {
        "collectionIds": [],
        "disabledConnectorIds": [],
        "deviceEnvInfo": {
            "darkModeEnabled": False,
            "devicePixelRatio": 2,
            "screenHeight": 1328,
            "screenWidth": 2056,
            "viewportHeight": 1083,
            "viewportWidth": 2056,
        },
        "disableMemory": True,
        "disableSearch": False,
        "disableSelfHarmShortCircuit": False,
        "disableTextFollowUps": False,
        "enableImageGeneration": enable_image,
        "enableImageStreaming": enable_image,
        "enableSideBySide": True,
        "fileAttachments": [],
        "forceConcise": False,
        "forceSideBySide": False,
        "imageAttachments": [],
        "imageGenerationCount": 2 if enable_image else 0,
        "isAsyncChat": False,
        "message": message,
        "modeId": "fast",
        "responseMetadata": {},
        "returnImageBytes": False,
        "returnRawGrokInXaiRequest": False,
        "sendFinalMetadata": True,
        "temporary": True,
    }


def classify_body(status: int, body: str, cf: str | None) -> str:
    lower = body.lower()
    if cf or "just a moment" in lower:
        return "cf_challenge"
    if status == 403 and "anti-bot" in lower:
        return "anti_bot_403"
    if status == 400:
        return "http_400"
    if status == 200 and '"modelResponse"' in body:
        return "chat_ok"
    return f"http_{status}"


def load_canary():
    if CANARY.exists():
        spec = importlib.util.spec_from_file_location("canary", CANARY)
        mod = importlib.util.module_from_spec(spec)
        assert spec and spec.loader
        spec.loader.exec_module(mod)
        return mod
    # Panda / 无 AutoRegister：使用内联 fallback
    class _Fallback:
        chat_payload = staticmethod(chat_payload)
        classify_body = staticmethod(classify_body)

    return _Fallback()


def load_auth(email: str) -> dict:
    path = GROK_ROOT / "web_auths" / f"{email}.json"
    if path.exists():
        return json.loads(path.read_text(encoding="utf-8"))
    return {"email": email}


def cookie_header(auth: dict) -> str:
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    return f"sso={sso}; sso-rw={sso_rw}"


def b64decode(s: str) -> bytes:
    pad = "=" * ((4 - len(s) % 4) % 4)
    return base64.b64decode(s + pad)


def generate_statsig(
    method: str,
    path: str,
    meta48: bytes,
    fingerprint: str,
    *,
    n: int | None = None,
    key: int | None = None,
    trailer: bytes = b"\x03",
) -> str:
    if len(meta48) != 48:
        raise ValueError(f"meta48 len={len(meta48)}")
    n = int(time.time() - EPOCH) if n is None else n
    dig = f"{method}!{path}!{n}obfiowerehiring{fingerprint}"
    sha = hashlib.sha256(dig.encode()).digest()[:16]
    key = (hashlib.sha256(dig.encode()).digest()[0]) if key is None else key
    block = meta48 + struct.pack("<I", n) + sha + trailer
    enc = bytearray([key]) + bytearray(b ^ key for b in block)
    return base64.b64encode(bytes(enc)).decode().rstrip("=")


def node_sign(meta: str, path: str, method: str) -> str | None:
    js = (
        BUNDLE.read_text(encoding="utf-8")
        .replace("__GROK_META__", meta)
        .replace("__SIGN_PATH__", path)
        .replace("__SIGN_METHOD__", method)
    )
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
        tmp.write(js)
        p = tmp.name
    try:
        env = {**os.environ, "GROK_SIGN_MODULE": str(MODULE)}
        proc = subprocess.run(["node", p], capture_output=True, text=True, timeout=60, env=env)
        for line in proc.stdout.splitlines():
            if line.startswith("FULLSIG "):
                return line.split(" ", 2)[2].strip()
    finally:
        os.unlink(p)
    return None


def extract_meta_from_html(html: str) -> str | None:
    m = re.search(r'name=["\'](gr[^"\']*)["\'][^>]+content=["\']([^"\']+)["\']', html, re.I)
    if m:
        return m.group(2)
    m = re.search(r'content=["\']([^"\']+)["\'][^>]+name=["\'](gr[^"\']*)["\']', html, re.I)
    return m.group(1) if m else None


DIGEST_HOOK = """
(() => {
  globalThis.__grokDigestInputs = [];
  const capture = (d) => {
    try {
      const u8 = d instanceof ArrayBuffer ? new Uint8Array(d)
        : (d instanceof Uint8Array ? d : new Uint8Array(d));
      const t = new TextDecoder().decode(u8);
      if (t.includes('obfiowerehiring')) globalThis.__grokDigestInputs.push(t);
    } catch (e) {}
  };
  const proto = globalThis.SubtleCrypto && SubtleCrypto.prototype;
  if (proto && typeof proto.digest === 'function') {
    const original = proto.digest;
    Object.defineProperty(proto, 'digest', {
      configurable: true,
      value: function(a, d) { capture(d); return Reflect.apply(original, this, [a, d]); },
    });
  } else if (crypto?.subtle?.digest) {
    const original = crypto.subtle.digest.bind(crypto.subtle);
    crypto.subtle.digest = (a, d) => { capture(d); return original(a, d); };
  }
})();
"""


def extract_session_keys(email: str, *, headed: bool = False, proxy: str = PROXY) -> dict:
    """Playwright 一次：meta48 + fingerprint + cookie（纯 HTTP 后续签名用）。"""
    canary = load_canary()
    auth = load_auth(email)
    sso = str(auth.get("sso") or "").strip()
    sso_rw = str(auth.get("sso_rw") or sso)
    from playwright.sync_api import sync_playwright

    digests: list[str] = []
    sigs: list[dict] = []
    with sync_playwright() as p:
        browser = p.chromium.launch(
            headless=not headed,
            args=["--disable-blink-features=AutomationControlled"],
        )
        ctx = browser.new_context(proxy={"server": proxy}, user_agent=UA)
        ctx.add_init_script(canary.TURBOPACK_HOOK)
        ctx.add_init_script(canary.CAPTURE_HOOK)
        ctx.add_init_script(DIGEST_HOOK)
        ctx.add_cookies(
            [
                {"name": "sso", "value": sso, "domain": ".grok.com", "path": "/"},
                {"name": "sso-rw", "value": sso_rw, "domain": ".grok.com", "path": "/"},
            ]
        )
        page = ctx.new_page()

        def on_req(req):
            if "/rest/" not in req.url:
                return
            sig = req.headers.get("x-statsig-id") or ""
            if not sig or sig.startswith("eDA6") or sig.startswith("x0:"):
                return
            from urllib.parse import urlparse

            sigs.append({"path": urlparse(req.url).path, "method": req.method, "sig": sig})

        page.on("request", on_req)
        page.goto("https://grok.com/", wait_until="domcontentloaded", timeout=90000)
        page.wait_for_timeout(6000)
        page.evaluate("async () => { try { await fetch('/rest/modes', {credentials:'include'}); } catch(e){} }")
        page.wait_for_timeout(2500)
        digests = page.evaluate("() => (globalThis.__grokDigestInputs || []).slice(-80)") or []
        html = page.content()
        cookie = "; ".join(f"{c['name']}={c['value']}" for c in ctx.cookies("https://grok.com"))
        browser.close()

    meta_html = extract_meta_from_html(html)
    best = max(digests, key=len) if digests else ""
    fp = ""
    n = None
    trailer = b"\x03"
    meta48: bytes | None = None

    m = re.match(r"^([A-Z]+)!([^!]+)!(\d+)obfiowerehiring(.*)$", best)
    if m:
        fp = m.group(4)
        n = int(m.group(3))
        sha = hashlib.sha256(best.encode()).digest()[:16]
        for s in reversed(sigs):
            raw = bytearray(b64decode(s["sig"]))
            key = raw[0]
            plain = bytes(b ^ key for b in raw[1:])
            nb = struct.pack("<I", n)
            if len(plain) >= 69 and plain[49:53] == nb and plain[53:69] == sha:
                meta48 = plain[1:49]
                trailer = plain[69:70] or b"\x03"
                break

    if meta48 is None and meta_html:
        meta48 = b64decode(meta_html) if len(meta_html) > 60 else meta_html.encode()[:48]
        if len(meta48) != 48:
            meta48 = (meta48 + b"\x00" * 48)[:48]

    if meta48 is None or len(meta48) != 48:
        raise RuntimeError(f"session extract failed: digests={len(digests)} sigs={len(sigs)} meta_html={bool(meta_html)}")

    keys = {
        "email": email,
        "sso": sso,
        "meta_b64": base64.b64encode(meta48).decode(),
        "meta_html": meta_html or "",
        "fingerprint": fp,
        "trailer_hex": trailer.hex(),
        "cookie": cookie,
        "has_cf": "cf_clearance=" in cookie,
        "extracted_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "digest_sample": best[:120],
    }
    KEYS_DIR.mkdir(parents=True, exist_ok=True)
    out = KEYS_DIR / f"{email.replace('@', '_at_')}.json"
    out.write_text(json.dumps(keys, ensure_ascii=False, indent=2), encoding="utf-8")
    return keys


class GrokPureHttpClient:
    def __init__(
        self,
        keys: dict,
        *,
        signer: Literal["python", "node", "auto"] = "auto",
        upstream_proxy: str | None = None,
    ):
        self.keys = keys
        self.signer_mode = signer
        self.auth = load_auth(keys["email"])
        if keys.get("sso"):
            self.auth["sso"] = keys["sso"]
            self.auth.setdefault("sso_rw", keys["sso"])
        self.cookie = cookie_header(self.auth)
        if keys.get("cookie") and "cf_clearance=" in keys["cookie"]:
            self.cookie = keys["cookie"]
        up = (upstream_proxy if upstream_proxy is not None else UPSTREAM_PROXY).strip()
        self.proxies = {"http": up, "https": up} if up else PROXIES

    def _sign(self, method: str, path: str) -> str:
        meta48 = b64decode(self.keys["meta_b64"])
        fp = self.keys.get("fingerprint") or ""
        trailer = bytes.fromhex(self.keys.get("trailer_hex") or "03")
        if self.signer_mode == "python" or (self.signer_mode == "auto" and fp):
            return generate_statsig(method, path, meta48, fp, trailer=trailer)
        meta = self.keys.get("meta_html") or base64.b64encode(meta48).decode()
        sig = node_sign(meta, path, method)
        if not sig:
            raise RuntimeError("node sign failed")
        return sig

    def request(
        self,
        method: str,
        path: str,
        *,
        json_body: dict | None = None,
        stream: bool = False,
        timeout: int = 120,
    ) -> crequests.Response:
        sig = self._sign(method, path)
        headers = {
            "Accept": "*/*",
            "Content-Type": "application/json",
            "Origin": "https://grok.com",
            "Referer": "https://grok.com/",
            "User-Agent": UA,
            "Cookie": self.cookie,
            "x-statsig-id": sig,
            "x-xai-request-id": str(uuid.uuid4()),
        }
        url = "https://grok.com" + path
        if method.upper() == "GET":
            return crequests.get(
                url, headers=headers, impersonate=IMPERSONATE, proxies=self.proxies, timeout=timeout
            )
        return crequests.post(
            url,
            headers=headers,
            json=json_body or {},
            impersonate=IMPERSONATE,
            proxies=self.proxies,
            timeout=timeout,
            stream=stream,
        )

    def upload_file(self, file_path: Path, *, mime: str | None = None) -> dict:
        data = file_path.read_bytes()
        mime = mime or ("image/png" if file_path.suffix.lower() == ".png" else "application/octet-stream")
        path = "/rest/app-chat/upload-file"
        body = {
            "fileName": file_path.name,
            "fileMimeType": mime,
            "content": base64.b64encode(data).decode(),
        }
        r = self.request("POST", path, json_body=body, timeout=180)
        text = r.text[:2000]
        if r.status_code != 200:
            return {"ok": False, "http": r.status_code, "body": text}
        val = json.loads(r.text)
        fid = val.get("fileMetadataId") or val.get("fileId")
        return {"ok": bool(fid), "http": 200, "fileMetadataId": fid, "raw": val}

    def chat_new(self, message: str, *, file_ids: list[str] | None = None, mode: str = "fast") -> dict:
        canary = load_canary()
        payload = canary.chat_payload(message)
        payload["modeId"] = mode
        if file_ids:
            payload["fileAttachments"] = file_ids
        path = "/rest/app-chat/conversations/new"
        r = self.request("POST", path, json_body=payload, stream=True, timeout=180)
        return self._parse_chat_response(r, path)

    def chat_followup(
        self, conversation_id: str, parent_response_id: str, message: str, *, file_ids: list[str] | None = None
    ) -> dict:
        canary = load_canary()
        payload = canary.chat_payload(message)
        payload["responseId"] = parent_response_id
        if file_ids:
            payload["fileAttachments"] = file_ids
        path = f"/rest/app-chat/conversations/{conversation_id}/responses"
        r = self.request("POST", path, json_body=payload, stream=True, timeout=180)
        return self._parse_chat_response(r, path)

    def _parse_chat_response(self, r: crequests.Response, path: str) -> dict:
        chunks: list[bytes] = []
        total = 0
        for chunk in r.iter_content(8192):
            if not chunk:
                continue
            chunks.append(chunk)
            total += len(chunk)
            if total > 2_000_000:
                break
        body = b"".join(chunks).decode("utf-8", errors="replace")
        text_parts: list[str] = []
        conv_id = None
        resp_id = None
        parent_id = None
        for line in body.splitlines():
            line = line.strip()
            if not line or line == "[DONE]":
                continue
            if line.startswith("data:"):
                line = line[5:].strip()
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            res = obj.get("result") or obj
            if not isinstance(res, dict):
                continue
            conv = res.get("conversation") or {}
            if isinstance(conv, dict) and conv.get("conversationId"):
                conv_id = conv["conversationId"]
            response = res.get("response")
            targets = [response] if isinstance(response, dict) else [res]
            for block in targets:
                if not isinstance(block, dict):
                    continue
                if block.get("responseId"):
                    resp_id = block["responseId"]
                mr = block.get("modelResponse")
                if isinstance(mr, dict):
                    if mr.get("responseId"):
                        resp_id = mr["responseId"]
                    if mr.get("message"):
                        text_parts = [str(mr["message"])]
                    if mr.get("parentResponseId"):
                        parent_id = mr["parentResponseId"]
                ur = block.get("userResponse")
                if isinstance(ur, dict) and ur.get("responseId"):
                    parent_id = ur["responseId"]
                tok = block.get("token")
                tag = block.get("messageTag") or ""
                thinking = block.get("isThinking", False)
                if tok and not thinking and tag in ("final", "response_start", ""):
                    if tag == "final" or (tag == "response_start" and tok):
                        if not text_parts or text_parts[-1] != str(tok):
                            text_parts.append(str(tok))
        reply = "".join(text_parts)
        if not reply:
            # fallback: last modelResponse.message in body
            m = re.search(r'"modelResponse":\{[^}]*"message":"([^"]*)"', body)
            if m:
                reply = m.group(1)
        canary = load_canary()
        kind = canary.classify_body(r.status_code, body, r.headers.get("cf-mitigated"))
        return {
            "ok": r.status_code == 200 and bool(reply.strip()),
            "http": r.status_code,
            "kind": kind,
            "path": path,
            "conversation_id": conv_id,
            "response_id": resp_id,
            "parent_response_id": parent_id,
            "reply": reply,
            "body_prefix": body[:300].replace("\n", " "),
        }


def run_gate(
    email: str,
    *,
    extract: bool,
    headed: bool,
    signer: str,
    image_path: Path | None = None,
    ocr_prompt: str = DEFAULT_OCR_PROMPT,
    keys_path: Path | None = None,
) -> dict:
    keys_path = keys_path or (KEYS_DIR / f"{email.replace('@', '_at_')}.json")
    if extract or not keys_path.exists():
        keys = extract_session_keys(email, headed=headed)
    else:
        keys = json.loads(keys_path.read_text(encoding="utf-8"))
        keys["email"] = email

    client = GrokPureHttpClient(
        keys,
        signer=signer,  # type: ignore[arg-type]
        upstream_proxy=os.environ.get("GROK_UPSTREAM_PROXY", ""),
    )
    report: dict[str, Any] = {"email": email, "signer": signer, "steps": []}

    def step(name: str, fn):
        try:
            row = fn()
            row["name"] = name
            report["steps"].append(row)
            return row
        except Exception as exc:
            row = {"name": name, "ok": False, "error": f"{type(exc).__name__}: {exc}"}
            report["steps"].append(row)
            return row

    step("get_conversations", lambda: {"ok": client.request("GET", "/rest/app-chat/conversations").status_code == 200, "http": client.request("GET", "/rest/app-chat/conversations").status_code})

    probe_image = image_path or DEFAULT_OCR_IMAGE
    if probe_image.exists():
        up = step("upload_file", lambda: client.upload_file(probe_image, mime="image/png"))
    else:
        up = {"ok": False, "skipped": f"image missing: {probe_image}"}
        report["steps"].append({"name": "upload_file", **up})

    file_ids = [up["fileMetadataId"]] if up.get("fileMetadataId") else None
    chat1 = step("chat_new_text", lambda: client.chat_new("Reply with exactly: PONG"))
    chat2 = None
    chat3 = None
    if chat1.get("conversation_id") and chat1.get("response_id"):
        chat2 = step(
            "chat_followup",
            lambda: client.chat_followup(
                chat1["conversation_id"], chat1["response_id"], "Reply with exactly: PONG2"
            ),
        )
        if chat2.get("response_id") and chat1.get("conversation_id"):
            chat3 = step(
                "chat_followup_2",
                lambda: client.chat_followup(
                    chat1["conversation_id"],
                    chat2["response_id"],
                    "What were my previous two replies? One short sentence.",
                ),
            )
    if file_ids:
        step(
            "chat_ocr_with_file",
            lambda: client.chat_new(ocr_prompt, file_ids=file_ids, mode="fast"),
        )

    report["ok"] = any(s.get("ok") for s in report["steps"] if s["name"] == "chat_new_text")
    report["followup_ok"] = any(s.get("ok") for s in report["steps"] if s["name"].startswith("chat_followup"))
    report["ocr_ok"] = any(s.get("ok") for s in report["steps"] if s["name"] == "chat_ocr_with_file")
    report["upload_ok"] = any(s.get("ok") for s in report["steps"] if s["name"] == "upload_file")
    report["probe_image"] = str(probe_image)
    out = (keys_path.parent if keys_path else KEYS_DIR) / f"gate_{email.replace('@', '_at_')}.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return report


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--email", default="nancybaker2jyy@yumail.co")
    ap.add_argument("--extract", action="store_true")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--signer", choices=("auto", "python", "node"), default="auto")
    ap.add_argument("--keys", type=Path, help="pure_http_keys JSON（老池 account_{id}.json）")
    ap.add_argument("--gate", action="store_true", help="run full upload+chat gate")
    ap.add_argument("--image", type=Path, default=DEFAULT_OCR_IMAGE, help="OCR probe image (default bundled)")
    ap.add_argument("--ocr-prompt", default=DEFAULT_OCR_PROMPT)
    ap.add_argument("--message", default="Reply with exactly: PONG")
    args = ap.parse_args()

    if args.gate:
        keys_path = args.keys
        email = args.email
        if keys_path:
            keys_data = json.loads(keys_path.read_text(encoding="utf-8"))
            email = str(keys_data.get("email") or f"account_{keys_data.get('account_id', 'local')}")
        report = run_gate(
            email,
            extract=args.extract,
            headed=args.headed,
            signer=args.signer,
            image_path=args.image,
            ocr_prompt=args.ocr_prompt,
            keys_path=keys_path,
        )
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return 0 if report.get("ok") else 1

    keys_path = KEYS_DIR / f"{args.email.replace('@', '_at_')}.json"
    if args.extract or not keys_path.exists():
        keys = extract_session_keys(args.email, headed=args.headed)
    else:
        keys = json.loads(keys_path.read_text(encoding="utf-8"))
        keys["email"] = args.email
    client = GrokPureHttpClient(
        keys,
        signer=args.signer,  # type: ignore[arg-type]
        upstream_proxy=os.environ.get("GROK_UPSTREAM_PROXY", ""),
    )
    result = client.chat_new(args.message)
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main())
