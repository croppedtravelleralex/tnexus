#!/usr/bin/env python3
"""E2E: director vs casting image generation on tnexus.relai.asia."""
import base64
import json
import sys
import time
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
PROMPT = "a blue sphere on gray background, minimal product photo"


def preview_ok(preview: str | None) -> tuple[bool, int]:
    if not preview:
        return False, 0
    if preview.startswith("data:image/"):
        comma = preview.find(",")
        if comma < 0:
            return False, 0
        payload = preview[comma + 1 :]
        if ";base64" in preview[:comma]:
            return True, len(base64.b64decode(payload))
        return True, len(payload.encode())
    if preview.startswith("https://"):
        with urllib.request.urlopen(preview, timeout=120) as resp:
            data = resp.read()
        return len(data) > 0, len(data)
    return False, 0


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

    print("==> login")
    post("/api/auth/login", {"email": "user", "password": "123456"})

    cases = [
        (
            "director",
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
                "input_prompt": PROMPT,
                "actor_image_counts": {"gpt": 1},
            },
            1,
        ),
        (
            "casting",
            {
                "mode": "casting",
                "workflow_path": "full_agent",
                "ps_enabled": False,
                "provider": "chatgpt",
                "director_models": ["gpt", "grok"],
                "gen_config": {
                    "quality": "auto",
                    "width": 1024,
                    "height": 1024,
                    "count": 1,
                    "transparent_bg": False,
                },
                "director_factors": {"x": 0.3, "y": 0.7},
                "ps_factors": {"x": 0.2, "y": 0.8},
                "input_prompt": PROMPT,
                "actor_image_counts": {"gpt": 1, "grok": 1},
            },
            2,
        ),
    ]

    ok_all = True
    for label, body, expect_results in cases:
        print(f"\n==> {label} mode")
        job_id = post("/api/jobs", body)["job_id"]
        print(f"job_id={job_id} expect_results>={expect_results}")
        detail = None
        for i in range(1, 121):
            detail = get(f"/api/jobs/{job_id}")
            status = detail.get("status")
            print(f"  poll {i} status={status}")
            if status == "done":
                break
            if status == "failed":
                print(json.dumps(detail, indent=2)[:2000])
                ok_all = False
                break
            time.sleep(5)
        else:
            print("  TIMEOUT")
            ok_all = False
            continue

        if detail.get("status") != "done":
            continue

        results = detail.get("results") or []
        print(f"  results={len(results)} providers={[r.get('provider') for r in results]}")
        if len(results) < expect_results:
            print(f"  FAIL expected>={expect_results} results")
            ok_all = False
            continue

        for ri, row in enumerate(results):
            preview = row.get("preview_url")
            good, nbytes = preview_ok(preview)
            kind = "b64" if preview and preview.startswith("data:") else "url"
            print(f"  result[{ri}] provider={row.get('provider')} preview={kind} bytes={nbytes}")
            if not good or nbytes < 10_000:
                ok_all = False

        if ok_all:
            print(f"  {label.upper()}_MODE_OK")

    if ok_all:
        print("\nSTUDIO_MODES_OK")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
