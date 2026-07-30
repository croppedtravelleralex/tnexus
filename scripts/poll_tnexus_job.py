#!/usr/bin/env python3
"""Poll TNexus job and verify preview_url (gateway asset) is fetchable."""
import json
import sys
import time
import urllib.request
import http.cookiejar

TN_API = "http://127.0.0.1:9000"


def main() -> int:
    cj = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))

    def post(path: str, body: dict) -> dict:
        req = urllib.request.Request(
            f"{TN_API}{path}",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with opener.open(req) as resp:
            return json.loads(resp.read())

    def get(path: str) -> dict:
        with opener.open(f"{TN_API}{path}") as resp:
            return json.loads(resp.read())

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
            "input_prompt": "green triangle on white background, minimal product photo",
        },
    )["job_id"]
    print(f"job_id={job_id}")

    for i in range(1, 121):
        detail = get(f"/api/jobs/{job_id}")
        status = detail.get("status")
        print(f"poll {i} status={status}")
        if status == "done":
            results = detail.get("results") or []
            preview = (results[0] or {}).get("preview_url") if results else None
            print(f"preview_url={preview}")
            if not preview:
                print(json.dumps(detail, indent=2))
                return 1
            with urllib.request.urlopen(preview) as resp:
                data = resp.read()
            print(f"preview_bytes={len(data)}")
            if len(data) < 1000:
                return 1
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
