#!/usr/bin/env python3
"""End-to-end URL chain test against production tnexus.relai.asia."""
import json
import sys
import time
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
EXPECTED_PREFIX = "https://imagemanager.relai.asia/"


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
    post("/api/auth/login", {"email": "demo", "password": "demo1234"})
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
            print(f"preview_url={preview}")
            if not preview or not preview.startswith(EXPECTED_PREFIX):
                print(json.dumps(detail, indent=2)[:3000])
                return 1
            with urllib.request.urlopen(preview, timeout=120) as resp:
                nbytes = len(resp.read())
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
