#!/usr/bin/env python3
"""Migrate inline_preview_b64 rows to local IMAGE_STORE_PATH and set file keys.

Usage on Panda:
  export DATABASE_URL=postgres://tnexus:...@127.0.0.1:5433/tnexus
  export IMAGE_STORE_PATH=/data/images
  python3 scripts/backfill_inline_to_disk.py

Requires: psycopg2-binary, Pillow (optional for webp variants)
"""
from __future__ import annotations

import base64
import os
import sys
import uuid
from io import BytesIO
from pathlib import Path

try:
    import psycopg2
except ImportError:
    print("pip install psycopg2-binary", file=sys.stderr)
    raise

try:
    from PIL import Image
except ImportError:
    Image = None


def decode_b64(raw: str) -> bytes:
    if raw.startswith("data:"):
        raw = raw.split(",", 1)[1]
    return base64.b64decode(raw)


def write_variants(root: Path, user_id: str, job_id: str, image_bytes: bytes) -> tuple[str, str, str]:
    asset_id = uuid.uuid4()
    base = f"{user_id}/{job_id}"
    orig_key = f"{base}/original/{asset_id}.png"
    preview_key = f"{base}/preview/{asset_id}.webp"
    thumb_key = f"{base}/thumb/{asset_id}.webp"

    orig_path = root / orig_key
    orig_path.parent.mkdir(parents=True, exist_ok=True)
    orig_path.write_bytes(image_bytes)

    if Image is not None:
        img = Image.open(BytesIO(image_bytes))
        for key, max_side in ((preview_key, 512), (thumb_key, 256)):
            path = root / key
            path.parent.mkdir(parents=True, exist_ok=True)
            w, h = img.size
            scale = min(1.0, max_side / max(w, h))
            nw, nh = max(1, int(w * scale)), max(1, int(h * scale))
            resized = img.resize((nw, nh), Image.Resampling.LANCZOS)
            resized.save(path, format="WEBP", quality=85)
    else:
        # Fallback: store same bytes for preview/thumb (API will resize on read)
        for key in (preview_key, thumb_key):
            path = root / key
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(image_bytes)

    return orig_key, preview_key, thumb_key


def main() -> None:
    db_url = os.environ.get("DATABASE_URL")
    store_path = os.environ.get("IMAGE_STORE_PATH", "/data/images")
    if not db_url:
        print("DATABASE_URL required", file=sys.stderr)
        sys.exit(1)

    root = Path(store_path)
    root.mkdir(parents=True, exist_ok=True)

    conn = psycopg2.connect(db_url)
    conn.autocommit = False
    cur = conn.cursor()
    cur.execute(
        """
        SELECT jr.id, jr.job_id, jr.inline_preview_b64, j.user_id
        FROM job_results jr
        JOIN jobs j ON j.id = jr.job_id
        WHERE jr.inline_preview_b64 IS NOT NULL
          AND length(jr.inline_preview_b64) > 100
          AND (jr.r2_key_original IS NULL OR jr.r2_key_original = '')
        ORDER BY jr.created_at
        """
    )
    rows = cur.fetchall()
    print(f"found {len(rows)} rows to backfill")

    migrated = 0
    for result_id, job_id, b64, user_id in rows:
        try:
            image_bytes = decode_b64(b64)
            orig, preview, thumb = write_variants(root, str(user_id), str(job_id), image_bytes)
            cur.execute(
                """
                UPDATE job_results
                SET r2_key_original = %s,
                    r2_key_preview = %s,
                    r2_key_thumb = %s,
                    inline_preview_b64 = NULL
                WHERE id = %s
                """,
                (orig, preview, thumb, result_id),
            )
            migrated += 1
            print(f"ok {result_id}")
        except Exception as exc:  # noqa: BLE001
            print(f"fail {result_id}: {exc}", file=sys.stderr)

    conn.commit()
    cur.close()
    conn.close()
    print(f"migrated {migrated}/{len(rows)} rows into {root}")


if __name__ == "__main__":
    main()
