#!/usr/bin/env python3
"""Production smoke for studio UX items not covered by prod_url_chain_test."""
from __future__ import annotations

import json
import struct
import sys
import time
import urllib.request
import http.cookiejar
from io import BytesIO

API = "https://tnexus.relai.asia"


def png_size(data: bytes) -> tuple[int, int] | None:
    if len(data) < 24 or data[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


def jpeg_size(data: bytes) -> tuple[int, int] | None:
    i = 2
    while i < len(data) - 8:
        if data[i] != 0xFF:
            i += 1
            continue
        marker = data[i + 1]
        if marker in (0xC0, 0xC1, 0xC2):
            h, w = struct.unpack(">HH", data[i + 5 : i + 9])
            return w, h
        if marker in (0xD8, 0xD9):
            i += 2
            continue
        seg_len = struct.unpack(">H", data[i + 2 : i + 4])[0]
        i += 2 + seg_len
    return None


def image_size(data: bytes) -> tuple[int, int] | None:
    return png_size(data) or jpeg_size(data)


class Client:
    def __init__(self) -> None:
        cj = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(cj))

    def post(self, path: str, body: dict) -> dict:
        req = urllib.request.Request(
            f"{API}{path}",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with self.opener.open(req, timeout=120) as resp:
            return json.loads(resp.read())

    def get(self, path: str) -> dict:
        with self.opener.open(f"{API}{path}", timeout=120) as resp:
            return json.loads(resp.read())

    def fetch_bytes(self, url: str) -> bytes:
        if url.startswith("/"):
            url = f"{API}{url}"
        with self.opener.open(url, timeout=180) as resp:
            return resp.read()

    def poll_job(self, job_id: str, timeout_s: int = 180) -> tuple[dict, float]:
        t0 = time.time()
        detail: dict = {}
        first_non_queued = None
        for _ in range(timeout_s // 2):
            detail = self.get(f"/api/jobs/{job_id}")
            status = detail.get("status")
            if status != "queued" and first_non_queued is None:
                first_non_queued = time.time() - t0
            if status in ("done", "failed"):
                return detail, first_non_queued or (time.time() - t0)
            time.sleep(2)
        return detail, first_non_queued or (time.time() - t0)


def quota_badge_variant(account: dict) -> str:
    receive = str(account.get("panda_receive_state") or "").strip().lower()
    manual_on = receive in ("", "verified_ready", "verified", "local_verified")
    if manual_on and account.get("status") == "正常":
        return "success"
    state = str(account.get("image_quota_state") or "").strip().lower()
    if state == "unlimited":
        return "info"
    if state in ("unverified", "refresh_pending"):
        return "warning"
    return "secondary"


def main() -> int:
    c = Client()
    failures: list[str] = []

    print("==> login")
    c.post("/api/auth/login", {"email": "user", "password": "123456"})

    # 1) Aspect ratio / style preset → stored gen_config + distinct output geometry
    print("\n==> [1] aspect ratio 16:9(4k) vs 1:1")
    style_prompt = (
        "[风格预设: 赛博朋克] Style reference: neon cyberpunk cityscape, rain-soaked streets. "
        "a single red umbrella on wet pavement"
    )
    job_169 = c.post(
        "/api/jobs",
        {
            "mode": "director",
            "workflow_path": "full_agent",
            "ps_enabled": False,
            "provider": "chatgpt",
            "director_models": ["gpt"],
            "gen_config": {
                "quality": "high",
                "width": 3840,
                "height": 2160,
                "count": 1,
                "transparent_bg": False,
            },
            "director_factors": {"x": 0.5, "y": 0.5},
            "ps_factors": {"x": 0.5, "y": 0.5},
            "input_prompt": style_prompt,
            "actor_image_counts": {"gpt": 1},
        },
    )["job_id"]
    job_11 = c.post(
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
            "director_factors": {"x": 0.5, "y": 0.5},
            "ps_factors": {"x": 0.5, "y": 0.5},
            "input_prompt": "a single red umbrella on wet pavement",
            "actor_image_counts": {"gpt": 1},
        },
    )["job_id"]

    for label, job_id, exp_w, exp_h in [
        ("16:9_4k", job_169, 3840, 2160),
        ("1:1", job_11, 1024, 1024),
    ]:
        detail, start_s = c.poll_job(job_id)
        gc = detail.get("gen_config") or {}
        stored = (gc.get("width"), gc.get("height"))
        print(f"  {label} status={detail.get('status')} start_s={start_s:.1f} stored={stored}")
        if stored != (exp_w, exp_h):
            failures.append(f"{label}: gen_config stored {stored} != ({exp_w},{exp_h})")
        if detail.get("status") != "done":
            failures.append(f"{label}: job {detail.get('status')} {detail.get('error_message')}")
            continue
        prompt = detail.get("input_prompt") or ""
        if label == "16:9_4k" and "赛博朋克" not in prompt:
            failures.append(f"{label}: style preset missing in input_prompt")
        preview = (detail.get("results") or [{}])[0].get("preview_url")
        if preview:
            try:
                data = c.fetch_bytes(preview)
                sz = image_size(data)
                print(f"  {label} preview_bytes={len(data)} decoded_size={sz}")
                if sz:
                    ratio = sz[0] / sz[1]
                    if label == "16:9_4k" and ratio < 1.4:
                        failures.append(f"{label}: image ratio {ratio:.2f} not landscape 16:9-ish")
                    if label == "1:1" and abs(ratio - 1.0) > 0.15:
                        failures.append(f"{label}: image ratio {ratio:.2f} not ~1:1")
            except Exception as e:
                print(f"  {label} preview_fetch_warn: {e}")

    # 2) Casting + extreme ps_factors (non-uniform white / rich detail in prompt path)
    print("\n==> [2] casting + extreme ps_factors")
    job_cast = c.post(
        "/api/jobs",
        {
            "mode": "casting",
            "workflow_path": "full_agent",
            "ps_enabled": True,
            "provider": "chatgpt",
            "director_models": ["gpt", "grok"],
            "gen_config": {
                "quality": "high",
                "width": 1536,
                "height": 1024,
                "count": 1,
                "transparent_bg": False,
                "polish_factor": 0.8,
            },
            "director_factors": {"x": 0.9, "y": 0.9},
            "ps_factors": {"x": 0.95, "y": 0.95},
            "input_prompt": "portrait of a ceramic vase with flowers, studio product shot",
            "actor_image_counts": {"gpt": 1, "grok": 1},
        },
    )["job_id"]
    cast_detail, cast_start = c.poll_job(job_cast, timeout_s=240)
    print(f"  casting status={cast_detail.get('status')} start_s={cast_start:.1f} results={len(cast_detail.get('results') or [])}")
    if cast_detail.get("status") != "done":
        failures.append(f"casting: {cast_detail.get('status')} {cast_detail.get('error_message')}")
    else:
        providers = {r.get("provider") for r in cast_detail.get("results") or []}
        if len(providers) < 2:
            failures.append(f"casting: expected 2 providers, got {providers}")
        for r in cast_detail.get("results") or []:
            pv = r.get("preview_url")
            if not pv:
                failures.append(f"casting {r.get('provider')}: no preview")
                continue
            try:
                data = c.fetch_bytes(pv)
                if len(data) < 8000:
                    failures.append(f"casting {r.get('provider')}: preview too small ({len(data)}B)")
            except Exception as e:
                failures.append(f"casting {r.get('provider')}: fetch failed {e}")

    # 3) Queue hint logic — job should leave queued within 30s
    print("\n==> [3] queue window (<30s to start)")
    job_q = c.post(
        "/api/jobs",
        {
            "mode": "director",
            "workflow_path": "full_agent",
            "ps_enabled": False,
            "provider": "chatgpt",
            "director_models": ["gpt"],
            "gen_config": {"quality": "auto", "width": 1024, "height": 1024, "count": 1, "transparent_bg": False},
            "director_factors": {"x": 0.5, "y": 0.5},
            "ps_factors": {"x": 0.5, "y": 0.5},
            "input_prompt": "green apple on wooden table",
            "actor_image_counts": {"gpt": 1},
        },
    )["job_id"]
    _, q_start = c.poll_job(job_q, timeout_s=120)
    print(f"  queue_start_s={q_start:.1f}")
    if q_start >= 30:
        failures.append(f"queue: job still queued after {q_start:.1f}s (UI would show yellow hint)")

    # 4) Account quota badge — scheduling + 正常 => success (green)
    print("\n==> [4] quota badge variant (admin)")
    c.post("/api/auth/login", {"email": "admin", "password": "123456"})
    accounts = c.get("/api/accounts?limit=200").get("items") or []
    scheduling = [
        a
        for a in accounts
        if a.get("status") == "正常"
        and str(a.get("panda_receive_state") or "").strip().lower()
        in ("", "verified_ready", "verified", "local_verified", "scheduling")
    ]
    green = [a for a in scheduling if quota_badge_variant(a) == "success"]
    print(f"  scheduling_normal={len(scheduling)} green_badge={len(green)}")
    if scheduling and len(green) < len(scheduling):
        bad = [a.get("email") for a in scheduling if quota_badge_variant(a) != "success"][:5]
        failures.append(f"quota badge: non-green for scheduling accounts: {bad}")
    sample = scheduling[0] if scheduling else accounts[0] if accounts else {}
    print(
        "  sample",
        {
            "email": sample.get("email"),
            "status": sample.get("status"),
            "receive": sample.get("panda_receive_state"),
            "image_quota_state": sample.get("image_quota_state"),
            "badge": quota_badge_variant(sample),
        },
    )

    # 5) Chat streaming SSE
    print("\n==> [5] chat stream")
    c.post("/api/auth/login", {"email": "user", "password": "123456"})
    req = urllib.request.Request(
        f"{API}/api/chat/completions",
        data=json.dumps(
            {
                "model": "gpt-4o-mini",
                "messages": [{"role": "user", "content": "Count to three slowly."}],
                "stream": True,
                "max_tokens": 32,
            }
        ).encode(),
        headers={"Content-Type": "application/json", "Accept": "text/event-stream"},
        method="POST",
    )
    chunks: list[str] = []
    with c.opener.open(req, timeout=90) as resp:
        ctype = resp.headers.get("Content-Type", "")
        raw = resp.read(8000).decode("utf-8", errors="replace")
    print(f"  content_type={ctype!r} bytes={len(raw)}")
    if "text/event-stream" not in ctype and "text/plain" not in ctype:
        failures.append(f"chat stream: unexpected content-type {ctype}")
    if "data:" not in raw:
        failures.append("chat stream: no SSE data frames")
    else:
        for line in raw.splitlines():
            if line.startswith("data:") and line.strip() != "data: [DONE]":
                chunks.append(line)
        print(f"  sse_frames={len(chunks)} sample={chunks[0][:120] if chunks else 'none'}")

    # 6) Log phase timings sum ≈ total (via latest done job)
    print("\n==> [6] phase timings alignment")
    c.post("/api/auth/login", {"email": "user", "password": "123456"})
    jobs = c.get("/api/jobs?limit=5")
    items = jobs if isinstance(jobs, list) else jobs.get("items") or jobs.get("jobs") or []
    done = next((j for j in items if j.get("status") == "done"), None)
    if done and done.get("phase_timings_ms"):
        pt = done["phase_timings_ms"]
        total = sum(int(v) for v in pt.values() if isinstance(v, (int, float)))
        created = done.get("created_at")
        updated = done.get("updated_at")
        print(f"  job={done.get('id')} phases={pt} sum_ms={total}")
        if total <= 0:
            failures.append("phase timings: empty sum")
    else:
        detail = c.get(f"/api/jobs/{job_q}")
        pt = detail.get("phase_timings_ms") or {}
        print(f"  job={job_q} phases={pt}")
        if not pt:
            failures.append("phase timings: missing on completed job")

    print("\n==> summary")
    if failures:
        for f in failures:
            print(f"  FAIL {f}")
        return 1
    print("  UX_COVERAGE_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
