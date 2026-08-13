#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

from curl_cffi import requests as cr

auth = json.loads(
    Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao\web_auths\kevinthomas8oqg@yumail.co.json").read_text(
        encoding="utf-8"
    )
)
bundle = Path(r"D:\SelfMadeTool\TNexus\crates\grok-signer\assets\grok_sign_standalone.js")
PROXY = "http://127.0.0.1:7897"

r = cr.get(
    "https://grok.com/",
    headers={"User-Agent": "Mozilla/5.0"},
    proxies={"http": PROXY, "https": PROXY},
    impersonate="chrome131",
    timeout=60,
)
m = re.search(r'name=["\']grok-site[^"\']*["\'][^>]+content=["\']([^"\']+)', r.text, re.I)
meta = m.group(1) if m else ""
print("meta", meta[:50])

js = (
    bundle.read_text(encoding="utf-8")
    .replace("__GROK_META__", meta)
    .replace("__SIGN_PATH__", "/rest/app-chat/conversations/new")
    .replace("__SIGN_METHOD__", "POST")
)
with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False, encoding="utf-8") as tmp:
    tmp.write(js)
    path = tmp.name
proc = subprocess.run(
    ["node", path],
    capture_output=True,
    text=True,
    timeout=60,
    env={**os.environ, "GROK_SIGN_MODULE": str(bundle.parent / "grok_sign_module_1645e3.js")},
)
print("node rc", proc.returncode)
print("stdout:", proc.stdout[-500:] if proc.stdout else "")
print("stderr:", proc.stderr[-800:] if proc.stderr else "")
sig = None
for line in proc.stdout.splitlines():
    if line.startswith("FULLSIG "):
        sig = line.split(" ", 2)[2].strip()
print("sig_len", len(sig or ""))
if sig:
    cookie = f"sso={auth['sso']}; sso-rw={auth.get('sso_rw', auth['sso'])}"
    body = {
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
        "enableImageGeneration": False,
        "enableImageStreaming": False,
        "enableSideBySide": True,
        "fileAttachments": [],
        "forceConcise": False,
        "forceSideBySide": False,
        "imageAttachments": [],
        "imageGenerationCount": 0,
        "isAsyncChat": False,
        "message": "Reply with exactly: PONG",
        "modeId": "fast",
        "responseMetadata": {},
        "returnImageBytes": False,
        "returnRawGrokInXaiRequest": False,
        "sendFinalMetadata": True,
        "temporary": True,
    }
    pr = cr.post(
        "https://grok.com/rest/app-chat/conversations/new",
        headers={
            "User-Agent": "Mozilla/5.0",
            "Cookie": cookie,
            "Content-Type": "application/json",
            "Origin": "https://grok.com",
            "Referer": "https://grok.com/",
            "x-statsig-id": sig,
        },
        data=json.dumps(body),
        proxies={"http": PROXY, "https": PROXY},
        impersonate="chrome131",
        timeout=90,
    )
    print("POST", pr.status_code, pr.text[:240])
os.unlink(path)
