#!/usr/bin/env python3
"""Minimal Cloudflare DNS helper for exposing grokProxy's admin page.

Credentials come from the environment, never the command line, so they do not
land in shell history:

    CF_API_EMAIL   account email (global key auth)
    CF_API_KEY     global API key

    python cf_dns.py zones
    python cf_dns.py list --zone closeapi.top
    python cf_dns.py upsert --zone closeapi.top --name grok --type A \
        --content 1.2.3.4 --proxied
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

API = "https://api.cloudflare.com/client/v4"


def call(method: str, path: str, payload: dict | None = None, attempts: int = 4) -> dict:
    """Cloudflare call with retry.

    This workstation's TLS to api.cloudflare.com drops often enough
    (UNEXPECTED_EOF) that a single attempt regularly fails on an otherwise
    healthy request.
    """
    email = os.environ.get("CF_API_EMAIL", "").strip()
    key = os.environ.get("CF_API_KEY", "").strip()
    if not email or not key:
        raise SystemExit("CF_API_EMAIL / CF_API_KEY not set")
    headers = {
        "X-Auth-Email": email,
        "X-Auth-Key": key,
        "Content-Type": "application/json",
    }
    data = None if payload is None else json.dumps(payload).encode()
    last = ""
    for attempt in range(1, max(1, attempts) + 1):
        request = urllib.request.Request(API + path, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=40) as response:
                return json.loads(response.read() or b"{}")
        except urllib.error.HTTPError as exc:
            raw = exc.read()
            try:
                body = json.loads(raw or b"{}")
            except Exception:
                body = {"raw": raw[:400].decode("utf-8", "replace")}
            # An HTTP error is a real answer; retrying will not change it.
            return {"success": False, "status": exc.code, "errors": body.get("errors", body)}
        except Exception as exc:  # noqa: BLE001
            last = str(exc)[:160]
            if attempt < attempts:
                time.sleep(2.0 * attempt)
    return {"success": False, "status": 0, "errors": f"transport failed after {attempts}: {last}"}


def zone_id(name: str) -> str:
    result = call("GET", f"/zones?name={name}&per_page=50")
    for zone in result.get("result") or []:
        if zone.get("name") == name:
            return zone["id"]
    raise SystemExit(f"zone not found: {name} ({result.get('errors')})")


def main() -> int:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    sub.add_parser("zones")

    lister = sub.add_parser("list")
    lister.add_argument("--zone", required=True)
    lister.add_argument("--contains", default="")

    upsert = sub.add_parser("upsert")
    upsert.add_argument("--zone", required=True)
    upsert.add_argument("--name", required=True, help="subdomain label or FQDN")
    upsert.add_argument("--type", default="A")
    upsert.add_argument("--content", required=True)
    upsert.add_argument("--proxied", action="store_true")
    upsert.add_argument("--ttl", type=int, default=1)

    args = ap.parse_args()

    if args.cmd == "zones":
        result = call("GET", "/zones?per_page=50")
        for zone in result.get("result") or []:
            print(f"{zone['name']:<34} {zone['status']:<10} {zone['id']}")
        return 0

    zid = zone_id(args.zone)

    if args.cmd == "list":
        result = call("GET", f"/zones/{zid}/dns_records?per_page=200")
        for record in result.get("result") or []:
            if args.contains and args.contains not in record["name"]:
                continue
            print(
                f"{record['type']:<6} {record['name']:<44} "
                f"{str(record['content'])[:40]:<42} proxied={record.get('proxied')}"
            )
        return 0

    fqdn = args.name if args.name.endswith(args.zone) else f"{args.name}.{args.zone}"
    existing = call("GET", f"/zones/{zid}/dns_records?name={fqdn}")
    records = existing.get("result") or []
    body = {
        "type": args.type,
        "name": fqdn,
        "content": args.content,
        "ttl": args.ttl,
        "proxied": bool(args.proxied),
    }
    if records:
        rid = records[0]["id"]
        result = call("PUT", f"/zones/{zid}/dns_records/{rid}", body)
        action = "updated"
    else:
        result = call("POST", f"/zones/{zid}/dns_records", body)
        action = "created"
    if not result.get("success"):
        print(json.dumps(result, ensure_ascii=False)[:500], file=sys.stderr)
        return 2
    record = result["result"]
    print(f"{action}: {record['type']} {record['name']} -> {record['content']} proxied={record['proxied']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
