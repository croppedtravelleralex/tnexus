#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path

from grok_pure_http_client import KEYS_DIR, GrokPureHttpClient

email = "nancybaker2jyy@yumail.co"
keys = json.loads((KEYS_DIR / f"{email.replace('@', '_at_')}.json").read_text(encoding="utf-8"))
keys["email"] = email
c = GrokPureHttpClient(keys, signer="auto")

spec = importlib.util.spec_from_file_location(
    "canary", r"D:\SelfMadeTool\AutoRegister\grokImage\tools\web_http_chat_image_canary.v1.py"
)
canary = importlib.util.module_from_spec(spec)
assert spec and spec.loader
spec.loader.exec_module(canary)

r = c.request(
    "POST",
    "/rest/app-chat/conversations/new",
    json_body=canary.chat_payload("Reply with exactly: PONG"),
    stream=True,
    timeout=180,
)
chunks = []
for ch in r.iter_content(8192):
    if ch:
        chunks.append(ch)
body = b"".join(chunks).decode("utf-8", "replace")
print("status", r.status_code, "bytes", len(body))
Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao\reports\_sse_dump.txt").write_text(
    body[:8000], encoding="utf-8"
)
for i, line in enumerate(body.splitlines()[:25]):
    print(f"{i}: {line[:220]}")
