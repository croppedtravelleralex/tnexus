#!/usr/bin/env python3
"""TNexus shutdown readiness checklist — score progress toward replacing gptimage :8012.

Does NOT stop :8012. Run on Panda after deploy:

  python3 scripts/shutdown_readiness_check.py
  python3 scripts/shutdown_readiness_check.py --json
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any


@dataclass
class Check:
    name: str
    weight: float
    score: float  # 0..100
    detail: str


def load_env(path: str) -> dict[str, str]:
    out: dict[str, str] = {}
    if not os.path.isfile(path):
        return out
    for line in open(path, encoding="utf-8"):
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        out[k.strip()] = v.strip().strip('"')
    return out


def http_json(url: str, timeout: float = 8.0) -> tuple[bool, Any]:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return True, json.loads(resp.read())
    except Exception as e:
        return False, str(e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--env", default="/opt/tnexus/.env")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--api", default="http://127.0.0.1:9000")
    ap.add_argument("--gateway", default="http://127.0.0.1:8014")
    ap.add_argument("--account-ops", default="http://127.0.0.1:9011")
    ap.add_argument("--gptimage", default="http://127.0.0.1:8012")
    args = ap.parse_args()

    env = load_env(args.env)
    checks: list[Check] = []

    # D — 号池独立 (18%)
    backend = env.get("ACCOUNTS_BACKEND", "sqlite")
    accounts_db = env.get("ACCOUNTS_DB", "")
    if backend == "postgres" and accounts_db.startswith("postgres"):
        checks.append(Check("accounts_postgres", 18, 95, "ACCOUNTS_BACKEND=postgres"))
    elif "/gptimage" in accounts_db:
        checks.append(Check("accounts_postgres", 18, 55, f"still shared sqlite: {accounts_db}"))
    else:
        checks.append(Check("accounts_postgres", 18, 70, f"sqlite non-gptimage path: {accounts_db}"))

    # C — account-ops (18%)
    ok, body = http_json(f"{args.account_ops.rstrip('/')}/health")
    if ok and isinstance(body, dict) and body.get("runtime") == "rust":
        checks.append(Check("account_ops_rust", 18, 92, "tnexus-account-ops rust ok"))
    else:
        checks.append(Check("account_ops_rust", 18, 40, str(body)))

    # E — gateway (14%)
    ok, body = http_json(f"{args.gateway.rstrip('/')}/health")
    if ok and isinstance(body, dict) and body.get("image_enabled"):
        helper = body.get("helper_ok", False)
        acc = body.get("accounts", 0)
        score = 90 if helper else 85
        checks.append(
            Check("gateway_image", 14, score, f"accounts={acc} helper_ok={helper}")
        )
    else:
        checks.append(Check("gateway_image", 14, 30, str(body)))

    # F — 调度/背压 (22%) — env + gateway up
    gpt_base = env.get("GPTIMAGE_BASE", "")
    gw_base = env.get("GATEWAY_BASE", "")
    if "8014" in gpt_base or "8014" in gw_base:
        checks.append(Check("defaults_8014", 22, 80, f"GPTIMAGE_BASE={gpt_base}"))
    else:
        checks.append(Check("defaults_8014", 22, 50, f"GPTIMAGE_BASE={gpt_base} (expect :8014)"))

    # G — Studio/API (8%)
    ok, body = http_json(f"{args.api.rstrip('/')}/health")
    if ok:
        checks.append(Check("api_health", 8, 88, str(body)))
    else:
        checks.append(Check("api_health", 8, 40, str(body)))

    # A+B — UI/API 粗估 (18%)
    checks.append(Check("console_ui", 8, 94, "manual: /accounts /image-manager"))
    checks.append(Check("management_api", 10, 84, "jobs/chat/images CRUD deployed"))

    # Operational — external still on 8012 (not scored in weighted product %)
    ok8012, h8012 = http_json(f"{args.gptimage.rstrip('/')}/health?format=json")
    gptimage_live = ok8012 and isinstance(h8012, dict)

    total_w = sum(c.weight for c in checks)
    weighted = sum(c.weight * c.score for c in checks) / total_w

    result = {
        "weighted_shutdown_readiness_pct": round(weighted, 1),
        "target_for_safe_stop_gptimage": 95,
        "gptimage_8012_still_running": gptimage_live,
        "note": "Do not stop :8012 until weighted >= 95 and gray traffic cutover done",
        "checks": [
            {
                "name": c.name,
                "weight": c.weight,
                "score": c.score,
                "detail": c.detail,
            }
            for c in checks
        ],
    }

    if args.json:
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print(f"Shutdown readiness (weighted): {result['weighted_shutdown_readiness_pct']}%")
        print(f"Target before stopping :8012: {result['target_for_safe_stop_gptimage']}%")
        print(f"gptimage :8012 running: {gptimage_live}")
        print()
        for c in checks:
            print(f"  [{c.score:3.0f}%] {c.name} ({c.weight}% w) — {c.detail}")
        print()
        print(result["note"])

    return 0 if weighted >= 95 else 1


if __name__ == "__main__":
    sys.exit(main())
