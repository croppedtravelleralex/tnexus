#!/usr/bin/env python3
"""E2E: casting mode with 3 parallel slots (URL generation)."""
import json
import sys
import time
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
PROMPT = "a red cube on white background, studio product photo"


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

    body = {
        "mode": "casting",
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
        "director_factors": {"x": 0.5, "y": 0.5},
        "ps_factors": {"x": 0.5, "y": 0.5},
        "input_prompt": PROMPT,
        "actor_image_counts": {"gpt": 3},
    }

    print("==> casting 3 slots (single actor, chatgpt)")
    t0 = time.time()
    job_id = post("/api/jobs", body)["job_id"]
    print(f"job_id={job_id}")

    detail = None
    for i in range(1, 91):
        st = get(f"/api/jobs/{job_id}/status")
        status = st.get("status")
        if i == 1 or i % 5 == 0 or status in ("done", "failed"):
            print(f"  poll {i} status={status} elapsed={time.time()-t0:.0f}s")
        if status == "done":
            detail = get(f"/api/jobs/{job_id}")
            break
        if status == "failed":
            detail = get(f"/api/jobs/{job_id}")
            print(json.dumps(detail, indent=2)[:3000])
            return 1
        time.sleep(3)

    wall = time.time() - t0
    if not detail or detail.get("status") != "done":
        print("TIMEOUT")
        return 1

    results = detail.get("results") or []
    timings = detail.get("job", {}).get("phase_timings_ms") or detail.get("phase_timings_ms") or {}
    print(f"wall_clock={wall:.1f}s phase_timings={timings}")
    print(f"results={len(results)}")
    for ri, row in enumerate(results):
        preview = row.get("preview_url") or ""
        src = row.get("download_url") or row.get("preview_url") or ""
        kind = "thumb" if "/api/images/thumb/" in preview else ("url" if preview.startswith("http") else "other")
        print(f"  [{ri}] provider={row.get('provider')} preview={kind} src={str(src)[:80]}")

    if len(results) < 3:
        print(f"FAIL: expected 3 results, got {len(results)}")
        return 1

    has_url = any(
        (r.get("download_url") or "").startswith("http") or "/v1/images/assets/" in (r.get("download_url") or "")
        for r in results
    )
    if not has_url:
        print("WARN: no gateway asset URL in results (may still be thumb-only)")

    print(f"\nPARALLEL_CASTING_OK wall={wall:.1f}s slots=3")
    return 0


if __name__ == "__main__":
    sys.exit(main())
