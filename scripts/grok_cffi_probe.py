#!/usr/bin/env python3
"""curl_cffi + node sign probe (TLS impersonation)."""
import base64, json, os, re, subprocess, tempfile
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from curl_cffi import requests as creq

UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
PROXY = os.environ["GROK_EGRESS_PROXY"]
KEY = base64.b64decode(os.environ["GROK_CREDENTIAL_KEY"])
BUNDLE = "/root/TNexus/crates/grok-signer/assets/grok_sign_standalone.js"
PATH = "/rest/app-chat/conversations/new"

def decrypt(enc):
    pad = "=" * ((4 - len(enc) % 4) % 4)
    raw = base64.b64decode(enc + pad)
    return AESGCM(KEY).decrypt(raw[:12], raw[12:], None).decode().strip()

def extract_meta(html):
    m = re.search(r'name=["\'](gr[^"\']*)["\'][^>]+content=["\']([^"\']+)["\']', html, re.I)
    return m.group(2) if m else None

def sign(meta, path, method):
    js = open(BUNDLE).read().replace("__GROK_META__", meta).replace("__SIGN_PATH__", path).replace("__SIGN_METHOD__", method)
    with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
        f.write(js); p = f.name
    out = subprocess.check_output(["node", p], text=True)
    os.unlink(p)
    for line in out.splitlines():
        if line.startswith("FULLSIG "):
            return line.split(" ", 2)[2].strip()
    return out.strip().splitlines()[-1].strip()

sql = "SELECT ga.id, gc.encrypted_primary FROM grok_accounts ga JOIN grok_credentials gc ON gc.account_id=ga.id WHERE ga.enabled=true ORDER BY ga.id DESC LIMIT 5"
rows = subprocess.check_output([
    "docker","exec","panda-postgres-1","psql","-U","tnexus","-d","tnexus","-t","-A","-F","|","-c",sql
]).decode().strip().splitlines()

payload = {
    "collectionIds": [], "disabledConnectorIds": [],
    "deviceEnvInfo": {"darkModeEnabled": False, "devicePixelRatio": 2, "screenHeight": 1328, "screenWidth": 2056, "viewportHeight": 1083, "viewportWidth": 2056},
    "disableMemory": True, "disableSearch": False, "disableSelfHarmShortCircuit": False, "disableTextFollowUps": False,
    "enableImageGeneration": False, "enableImageStreaming": False, "enableSideBySide": True,
    "fileAttachments": [], "forceConcise": False, "forceSideBySide": False, "imageAttachments": [],
    "imageGenerationCount": 0, "isAsyncChat": False, "message": "Reply OK", "modeId": "fast",
    "responseMetadata": {}, "returnImageBytes": False, "returnRawGrokInXaiRequest": False,
    "sendFinalMetadata": True, "temporary": True,
}

proxies = {"http": PROXY, "https": PROXY}
for row in rows:
    aid, enc = row.split("|", 1)
    token = decrypt(enc)
    cookie = f"sso={token}; sso-rw={token}"
    r = creq.get("https://grok.com/", headers={"User-Agent": UA, "Cookie": cookie}, impersonate="chrome131", proxies=proxies, timeout=60)
    print(f"id={aid} GET home {r.status_code}")
    meta = extract_meta(r.text)
    if not meta:
        print("  no meta"); continue
    sig = sign(meta, PATH, "POST")
    headers = {"User-Agent": UA, "Content-Type": "application/json", "Origin": "https://grok.com", "Referer": "https://grok.com/", "Cookie": cookie, "x-statsig-id": sig}
    pr = creq.post("https://grok.com"+PATH, headers=headers, json=payload, impersonate="chrome131", proxies=proxies, timeout=60)
    print(f"  POST {pr.status_code} {pr.text[:160]!r}")
