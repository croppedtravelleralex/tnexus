#!/usr/bin/env python3
"""E2E: casting mode with N parallel slots (URL generation)."""
import argparse
import json
import sys
import time
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
PROMPT = "a red cube on white background, studio product photo"


def run_case(slots: int, opener) -> int:
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
        "actor_image_counts": {"gpt": slots},
    }

    print(f"\n==> casting {slots} slots (single actor, chatgpt)")
    t0 = time.time()
    job_id = post("/api/jobs", body)["job_id"]
    print(f"job_id={job_id}")

    # ~60s per slot if serial; allow 3x headroom for parallel batches
    max_polls = max(90, slots * 12)
    poll_interval = 3
    detail = None
    for i in range(1, max_polls + 1):
        st = get(f"/api/jobs/{job_id}/status")
        status = st.get("status")
        elapsed = time.time() - t0
        if i == 1 or i % 5 == 0 or status in ("done", "failed"):
            print(f"  poll {i} status={status} elapsed={elapsed:.0f}s")
        if status == "done":
            detail = get(f"/api/jobs/{job_id}")
            break
        if status == "failed":
            detail = get(f"/api/jobs/{job_id}")
            print(json.dumps(detail, indent=2)[:4000])
            return 1
        time.sleep(poll_interval)

    wall = time.time() - t0
    if not detail or detail.get("status") != "done":
        print("TIMEOUT")
        return 1

    results = detail.get("results") or []
    timings = detail.get("job", {}).get("phase_timings_ms") or detail.get("phase_timings_ms") or {}
    if timings:
        print(f"phase_timings keys={list(timings.keys())}")
        bw = timings.get("bandwidth")
        lat = timings.get("latency_percentiles_ms")
        if bw:
            print(f"  bandwidth={bw}")
        if lat:
            print(f"  latency_percentiles_ms={lat}")
    per_slot = wall / max(len(results), 1)
    print(f"wall_clock={wall:.1f}s per_slot_avg={per_slot:.1f}s phase_timings={timings}")
    print(f"results={len(results)}")
    for ri, row in enumerate(results[:5]):
        preview = row.get("preview_url") or ""
        src = row.get("download_url") or row.get("preview_url") or ""
        kind = "thumb" if "/api/images/thumb/" in preview else ("url" if preview.startswith("http") else "other")
        print(f"  [{ri}] provider={row.get('provider')} preview={kind} src={str(src)[:80]}")
    if len(results) > 5:
        print(f"  ... +{len(results) - 5} more")

    if len(results) < slots:
        print(f"FAIL: expected {slots} results, got {len(results)}")
        return 1

    has_url = any(
        (r.get("download_url") or "").startswith("http")
        or "/v1/images/assets/" in (r.get("download_url") or "")
        for r in results
    )
    if not has_url:
        print("WARN: no gateway asset URL in results (may still be thumb-only)")

    print(f"PARALLEL_CASTING_OK wall={wall:.1f}s slots={slots}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Parallel casting E2E test")
    parser.add_argument(
        "slots",
        nargs="*",
        type=int,
        default=[3],
        help="slot counts to test (default: 3)",
    )
    args = parser.parse_args()

    cj = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))

    print("==> login")
    req = urllib.request.Request(
        f"{API}/api/auth/login",
        data=json.dumps({"email": "user", "password": "123456"}).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with opener.open(req, timeout=60):
        pass

    summary = []
    for slots in args.slots:
        rc = run_case(slots, opener)
        summary.append((slots, rc))
        if rc != 0:
            break

    print("\n==> summary")
    for slots, rc in summary:
        print(f"  slots={slots} rc={rc} {'OK' if rc == 0 else 'FAIL'}")

    return 0 if all(rc == 0 for _, rc in summary) else 1


if __name__ == "__main__":
    sys.exit(main())
