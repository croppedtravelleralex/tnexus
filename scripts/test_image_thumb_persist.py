#!/usr/bin/env python3
"""Smoke test: inline_preview_b64 -> /api/images list + thumb endpoint."""
import json
import subprocess
import sys
import urllib.request
import http.cookiejar

API = "https://tnexus.relai.asia"
B64_1PX = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
)


def main() -> int:
    job_id = subprocess.check_output(
        [
            "ssh",
            "panda",
            "docker exec panda-postgres-1 psql -U tnexus -d tnexus -t -A -c "
            "'SELECT id FROM jobs ORDER BY created_at DESC LIMIT 1;'",
        ],
        text=True,
    ).strip()
    if not job_id:
        print("no job_id", file=sys.stderr)
        return 1

    insert_sql = (
        "INSERT INTO job_results (job_id, provider, variant_index, inline_preview_b64, agent_prompt) "
        f"VALUES ('{job_id}', 'test:persist', 99, '{B64_1PX}', 'thumb persistence smoke test') "
        "RETURNING id;"
    )
    result_id = subprocess.check_output(
        [
            "ssh",
            "panda",
            f"docker exec panda-postgres-1 psql -U tnexus -d tnexus -t -A -c \"{insert_sql}\"",
        ],
        text=True,
    ).strip()
    if "\n" in result_id:
        result_id = result_id.splitlines()[0].strip()
    print(f"inserted result_id={result_id}")

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

    post("/api/auth/login", {"email": "admin", "password": "123456"})
    images = get("/api/images")
    item = next((x for x in images.get("items", []) if x.get("rel") == result_id), None)
    if not item:
        print("image not listed", file=sys.stderr)
        print(json.dumps(images, indent=2)[:2000])
        return 1

    thumb_api = item.get("thumb_api_url")
    print(f"thumb_api_url={thumb_api}")
    if not thumb_api:
        print("missing thumb_api_url", file=sys.stderr)
        return 1

    with opener.open(f"{API}{thumb_api}", timeout=60) as resp:
        data = resp.read()
        ctype = resp.headers.get("Content-Type", "")
    print(f"thumb_status=200 bytes={len(data)} content_type={ctype}")
    if len(data) < 50:
        print("thumb too small", file=sys.stderr)
        return 1

    print("IMAGE_THUMB_PERSIST_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
