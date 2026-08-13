#!/usr/bin/env python3
import json
import re
from collections import Counter
from pathlib import Path

p = Path(r"D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao\reports\grok_ws_capture.json")
d = json.loads(p.read_text(encoding="utf-8"))
frames = d.get("ws_frames", [])
types = Counter()
sends: list[dict] = []
sessions: set[str] = set()

for f in frames:
    prev = f.get("preview", "")
    for m in re.finditer(r'"type":"([^"]+)"', prev):
        types[m.group(1)] += 1
    sid = re.search(r'"session_id":"([^"]+)"', prev)
    if sid:
        sessions.add(sid.group(1))
    if f.get("event") == "framesent" and "conversation.item.create" in prev:
        text_m = re.search(r'"input_chunks":\[\{"text":\{"text":"([^"]*)"', prev)
        sends.append(
            {
                "text": text_m.group(1) if text_m else "?",
                "t": f.get("t"),
                "has_castle": "castle_request_token" in prev,
            }
        )

print(json.dumps(
    {
        "email": d.get("email"),
        "http_req": len(d.get("requests", [])),
        "ws_frames": len(frames),
        "ws_connections": len(d.get("ws_connections", [])),
        "ws_url": (d.get("ws_connections") or [{}])[0].get("url", "")[:140],
        "session_ids": sorted(sessions),
        "chat_posts": len(d.get("chat_posts", [])),
        "app_chat_posts": len(d.get("app_chat_posts", [])),
        "event_types": types.most_common(15),
        "user_sends": sends,
        "has_response_create": types.get("response.create", 0),
        "has_castle_in_send": sum(1 for x in sends if x.get("has_castle")),
    },
    ensure_ascii=False,
    indent=2,
))
