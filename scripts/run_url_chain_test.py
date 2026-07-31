#!/usr/bin/env python3
"""Restart TNexus with Panda gateway env and verify preview_url is http asset link."""
import json
import os
import subprocess
import sys
import time
import urllib.request
import http.cookiejar

ROOT = r"D:\SelfMadeTool\TNexus"
GW = os.environ.get("GW_BASE", "https://tnexus.relai.asia")
TN_API = "http://127.0.0.1:9000"


def ssh(cmd: str) -> str:
    out = subprocess.check_output(["ssh", "panda", cmd], text=True)
    return out.strip()


def wsl(cmd: str, check: bool = True) -> None:
    subprocess.run(["wsl", "bash", "-lc", cmd], check=check)


def main() -> int:
    print("==> gateway token from panda")
    gw_token = ssh(
        r"""PASS=$(grep AUTH_BOOTSTRAP_ADMIN_PASSWORD /root/gptimage-gateway-rs/secrets/gateway.env | cut -d= -f2-)
curl -fsS -c - -X POST http://127.0.0.1:8014/api/auth/login -H 'Content-Type: application/json' \
  -d "{\"username\":\"admin\",\"password\":\"$PASS\"}" -o /dev/null | awk '/gws_session/ {print $7}'"""
    )
    print(f"gw_token_len={len(gw_token)}")
    if len(gw_token) < 50:
        print("bad gateway token", file=sys.stderr)
        return 1

    print("==> restart tnexus api/worker")
    wsl("pkill -f tnexus-worker 2>/dev/null || true; pkill -f tnexus-api 2>/dev/null || true; true", check=False)
    env = (
        f'GPTIMAGE_BASE={GW} UPSTREAM_API_KEY="{gw_token}" '
        "DATABASE_URL=postgres://tnexus:tnexus@localhost:5432/tnexus "
        "REDIS_URL=redis://127.0.0.1:6379 "
        "JWT_SECRET=change-me-to-a-long-random-secret-at-least-32-chars "
        "LISTEN_ADDR=0.0.0.0:9000 CORS_ORIGINS=http://localhost:3000"
    )
    wsl(
        f"source ~/.cargo/env; cd /mnt/d/SelfMadeTool/TNexus; "
        f"cargo build -p tnexus-api -p tnexus-worker -q; "
        f"nohup env {env} ./target/debug/tnexus-api > /tmp/tnexus-api.log 2>&1 & "
        f"nohup env {env} DIRECTOR_MODEL=gpt-4o-mini CHATGPT_IMAGE_MODEL=gpt-image-2 "
        f"./target/debug/tnexus-worker > /tmp/tnexus-worker.log 2>&1 & sleep 4"
    )

    print("==> tnexus health")
    with urllib.request.urlopen(f"{TN_API}/health") as resp:
        print(resp.read().decode())

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

    post("/api/auth/login", {"email": "admin", "password": "123456"})
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
            results = detail.get("results") or []
            preview = (results[0] or {}).get("preview_url") if results else None
            print(f"preview_url={preview}")
            if not preview or not preview.startswith("http"):
                print(json.dumps(detail, indent=2)[:2000])
                return 1
            with urllib.request.urlopen(preview) as resp:
                nbytes = len(resp.read())
            print(f"preview_bytes={nbytes}")
            print("TNEXUS_URL_CHAIN_OK")
            return 0
        if status == "failed":
            print(json.dumps(detail, indent=2))
            wsl("tail -30 /tmp/tnexus-worker.log")
            return 1
        time.sleep(5)
    wsl("tail -30 /tmp/tnexus-worker.log")
    return 1


if __name__ == "__main__":
    sys.exit(main())
