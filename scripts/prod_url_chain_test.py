#!/usr/bin/env python3
"""End-to-end URL chain test against production tnexus.relai.asia."""
import base64
import json
import sys
import time
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
HTTPS_PREFIX = "https://tnexus.relai.asia/"


def resolve_preview(preview: str) -> str:
    if preview.startswith("/"):
        return f"{API}{preview}"
    return preview


def preview_bytes(preview: str, opener: urllib.request.OpenerDirector | None = None) -> int:
    if preview.startswith("data:"):
        comma = preview.find(",")
        if comma < 0:
            raise ValueError("malformed data URL")
        payload = preview[comma + 1 :]
        if ";base64" in preview[:comma]:
            return len(base64.b64decode(payload))
        return len(payload.encode())
    fetch = opener.open if opener else urllib.request.urlopen
    with fetch(preview, timeout=120) as resp:
        return len(resp.read())


def main() -> int:
    cj = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))

    def post(path: str, body: dict) -> dict:
        req = urllib.request.Request(
            f"{API}{path}",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with opener.open(req, timeout=60) as resp:
            return json.loads(resp.read())

    def get(path: str) -> dict:
        with opener.open(f"{API}{path}", timeout=60) as resp:
            return json.loads(resp.read())

    print("==> health")
    with urllib.request.urlopen(f"{API}/health", timeout=30) as resp:
        print(resp.read().decode())

    print("==> login")
    post("/api/auth/login", {"email": "user", "password": "123456"})
    job_id = post(
        "/api/jobs",
        {
            "mode": "director",
            "workflow_path": "full_agent",
            "ps_enabled": False,
            "provider": "chatgpt",
            "director_models": ["gpt"],
            "gen_config": {
                "quality": "auto",
                "width": 1024,
                "height": 1024,
                "count": 1,
                "transparent_bg": False,
            },
            "director_factors": {"x": 0, "y": 0},
            "ps_factors": {"x": 0, "y": 0},
            "input_prompt": "a red cube on a white background, product photo, studio lighting",
        },
    )["job_id"]
    print(f"job_id={job_id}")

    for i in range(1, 121):
        detail = get(f"/api/jobs/{job_id}")
        status = detail.get("status")
        print(f"poll {i} status={status}")
        if status == "done":
            preview = (detail.get("results") or [{}])[0].get("preview_url")
            kind = (
                "inline_b64"
                if preview and preview.startswith("data:")
                else "thumb"
                if preview and preview.startswith("/api/images/thumb/")
                else "https"
            )
            print(f"preview_kind={kind}")
            if preview and len(preview) > 120:
                print(f"preview_url={preview[:80]}...({len(preview)} chars)")
            else:
                print(f"preview_url={preview}")
            if not preview or not (
                preview.startswith(HTTPS_PREFIX)
                or preview.startswith("data:image/")
                or preview.startswith("/api/images/thumb/")
            ):
                print(json.dumps(detail, indent=2)[:3000])
                return 1
            nbytes = preview_bytes(resolve_preview(preview), opener)
            print(f"preview_bytes={nbytes}")
            print("TNEXUS_URL_CHAIN_OK")
            return 0
        if status == "failed":
            print(json.dumps(detail, indent=2))
            return 1
        time.sleep(5)
    print("timeout")
    return 1


if __name__ == "__main__":
    sys.exit(main())
