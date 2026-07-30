#!/usr/bin/env python3
"""Panda loopback URL chain: TNexus :9000 -> ImageManager :8014 -> signed asset URL."""
import json
import os
import sys
import time
import urllib.request
import http.cookiejar

TN_API = os.environ.get("TN_API", "http://127.0.0.1:9000")
GW_BASE = os.environ.get("GW_BASE", "http://127.0.0.1:8014")
PREVIEW_PREFIX = os.environ.get(
    "PREVIEW_PREFIX", "https://tnexus.relai.asia/"
)


def main() -> int:
    print(f"==> loopback upstream check GPTIMAGE_BASE expected {GW_BASE}")
    with urllib.request.urlopen(f"{GW_BASE}/health", timeout=15) as resp:
        print("gateway", resp.read().decode())

    print(f"==> tnexus api {TN_API}")
    with urllib.request.urlopen(f"{TN_API}/health", timeout=15) as resp:
        print("tnexus", resp.read().decode())

    cj = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))

    print("==> login demo")
    login_req = urllib.request.Request(
        f"{TN_API}/api/auth/login",
        data=json.dumps({"email": "demo", "password": "demo1234"}).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with opener.open(login_req, timeout=60) as resp:
        token = None
        for header in resp.headers.get_all("Set-Cookie") or []:
            if header.startswith("tnexus_session="):
                token = header.split(";", 1)[0].split("=", 1)[1]
                break
        if not token:
            raise RuntimeError("missing tnexus_session cookie from login")
    auth_headers = {"Authorization": f"Bearer {token}"}

    def post(path: str, body: dict) -> dict:
        req = urllib.request.Request(
            f"{TN_API}{path}",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json", **auth_headers},
            method="POST",
        )
        with opener.open(req, timeout=60) as resp:
            return json.loads(resp.read())

    def get(path: str) -> dict:
        req = urllib.request.Request(
            f"{TN_API}{path}",
            headers=auth_headers,
            method="GET",
        )
        with opener.open(req, timeout=60) as resp:
            return json.loads(resp.read())

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
            source = (detail.get("results") or [{}])[0].get("source_url")
            print(f"preview_url={preview}")
            print(f"source_url={source}")
            if not preview or not preview.startswith(PREVIEW_PREFIX):
                print(json.dumps(detail, indent=2)[:3000])
                return 1
            with urllib.request.urlopen(preview, timeout=120) as resp:
                nbytes = len(resp.read())
            print(f"preview_bytes={nbytes}")
            print("PANDA_LOOPBACK_URL_CHAIN_OK")
            return 0
        if status == "failed":
            print(json.dumps(detail, indent=2))
            return 1
        time.sleep(5)
    print("timeout")
    return 1


if __name__ == "__main__":
    sys.exit(main())
