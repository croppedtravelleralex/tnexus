#!/usr/bin/env python3
"""ETL grok2api SQLite backend.db → TNexus PostgreSQL grok_* tables.

Implements docs/39b-grok-schema.md §4 (full-table COPY in dependency order).

Design:
- Schema-driven: reads the SQLite source tables (PRAGMA table_info) and the PG
  target columns (information_schema) live, then copies the column intersection
  per row into the mapped grok_* target. No hardcoded Go column lists.
- Defensive: any table missing on either side (source SQLite or target PG,
  e.g. migrations 011-015 not yet applied) is skipped and logged, never fatal.
- Preserves `id` values (explicit insert) so shadow diff stays possible.
- Preserves ciphertext columns verbatim (identity_key, encrypted_primary,
  encrypted_refresh, encrypted_secret, encrypted_proxy_url, ...) — never touched.
- Re-runnable: truncates the present grok target tables (CASCADE RESTART
  IDENTITY) then re-inserts.

Env:
  GROK_ETL_SOURCE    path to grok2api backend.db (SQLite)
  GROK_ETL_PG_DSN    postgres:// URL
  GROK_CREDENTIAL_KEY  base64 32-byte AES-256 key, only for optional decrypt
                      smoke (matches Go infra/security/cipher.go). Never printed.

Usage:
  python scripts/grok_etl_sqlite_to_pg.py --dry-run       # no PG needed
  python scripts/grok_etl_sqlite_to_pg.py                 # full copy
  python scripts/grok_etl_sqlite_to_pg.py --limit 10      # smoke/sample only
"""
from __future__ import annotations

import argparse
import base64
import datetime as _dt
import os
import sqlite3
import sys
from dataclasses import dataclass, field

# --- Go table name -> TNexus grok_* PG name (docs/39b §3). ------------------
# Order matters: parent tables (FK source) precede children so explicit-id
# inserts satisfy referential integrity.
TABLE_MAP: list[tuple[str, str]] = [
    ("admins", "grok_admins"),
    ("admin_sessions", "grok_admin_sessions"),
    ("provider_accounts", "grok_accounts"),
    ("account_credentials", "grok_credentials"),
    ("account_provider_links", "grok_account_provider_links"),
    ("web_account_profiles", "grok_web_profiles"),
    ("account_quota_windows", "grok_quota_windows"),
    ("account_billing_snapshots", "grok_billing_snapshots"),
    ("account_pool_snapshots", "grok_pool_snapshots"),
    ("account_quota_recovery", "grok_quota_recovery"),
    ("client_keys", "grok_client_keys"),
    ("billing_reservations", "grok_billing_reservations"),
    ("model_routes", "grok_model_routes"),
    ("model_route_aliases", "grok_model_route_aliases"),
    ("model_route_accounts", "grok_model_route_accounts"),
    ("client_key_models", "grok_client_key_models"),
    ("account_model_capabilities", "grok_model_capabilities"),
    ("account_model_sync_states", "grok_model_sync_states"),
    ("account_model_quota_blocks", "grok_model_quota_blocks"),
    ("account_model_states", "grok_model_states"),
    ("egress_nodes", "grok_egress_nodes"),
    ("egress_traffic_hops", "grok_egress_traffic_hops"),
    ("request_audits", "grok_request_audits"),
    ("response_ownership", "grok_response_ownership"),
    ("web_response_states", "grok_web_response_states"),
    ("image_pipeline_traces", "grok_pipeline_traces"),
    ("image_pipeline_segments", "grok_pipeline_segments"),
    ("chrome_tickets", "grok_chrome_tickets"),
    ("media_jobs", "grok_media_jobs"),
    ("media_assets", "grok_media_assets"),
    ("runtime_settings", "grok_runtime_settings"),
]

# Columns we always prefer to copy verbatim when they exist on both sides.
_KEEP_RAW = {"identity_key", "encrypted_primary", "encrypted_refresh",
             "encrypted_access_token", "encrypted_refresh_token",
             "encrypted_secret", "encrypted_proxy_url",
             "encrypted_cloudflare_cookie"}

# SQLite system tables / GORM metadata never copied.
_SKIP_SQLITE = {"sqlite_sequence", "sqlite_master"}

# grok2api-rs 启动时用 GROK_ADMIN_* 自举管理员，ETL 跳过避免列映射问题。
_SKIP_ETL_TARGETS = {"grok_admins", "grok_admin_sessions"}


@dataclass
class TablePlan:
    source: str
    target: str
    columns: list[str] = field(default_factory=list)  # intersection, ordered
    src_exists: bool = False
    dst_exists: bool = False


def _parse_ts(value):
    """Normalize SQLite datetime (naive 'YYYY-MM-DD HH:MM:SS' / ISO) -> datetime."""
    if value is None:
        return None
    if isinstance(value, _dt.datetime):
        return value
    s = str(value).strip()
    if not s:
        return None
    try:
        return _dt.datetime.fromisoformat(s.replace(" ", "T"))
    except ValueError:
        return s  # let PG attempt the cast


def _coerce(pg_type: str, raw):
    """Map a SQLite Python value onto a target PG column's expected type."""
    if raw is None:
        return None
    t = (pg_type or "").lower()
    if "bool" in t:
        if isinstance(raw, str):
            low = raw.strip().lower()
            if low in ("1", "true", "t", "yes", "on"):
                return True
            if low in ("0", "false", "f", "no", "off", "", "0.0"):
                return False
            return None
        return bool(raw)
    if "int" in t:
        try:
            return int(raw)
        except (TypeError, ValueError):
            return None
    if "numeric" in t or "real" in t or "double" in t or "float" in t:
        try:
            return float(raw) if isinstance(raw, (str, float)) else int(raw)
        except (TypeError, ValueError):
            return None
    if "json" in t:
        return raw  # wrapped to Json by caller
    if "time" in t or "date" in t:
        return _parse_ts(raw)
    if "bytea" in t:
        return raw if isinstance(raw, (bytes, bytearray)) else str(raw).encode()
    return raw  # text/varchar/etc.: pass through verbatim


def sqlite_columns(con: sqlite3.Connection, table: str) -> list[str]:
    cur = con.execute(f'PRAGMA table_info("{table}")')
    return [row[1] for row in cur.fetchall()]


def sqlite_tables(con: sqlite3.Connection) -> set[str]:
    rows = con.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
    return {r[0] for r in rows}


def pg_column_types(pg, schema: str) -> dict[str, dict[str, str]]:
    """Return {table: {column: data_type}} for tables in the given schema."""
    out: dict[str, dict[str, str]] = {}
    cur = pg.cursor()
    cur.execute(
        """
        SELECT table_name, column_name, data_type
        FROM information_schema.columns
        WHERE table_schema = %s
        ORDER BY table_name, ordinal_position
        """,
        (schema,),
    )
    for tname, cname, dtype in cur.fetchall():
        out.setdefault(tname, {})[cname] = dtype
    cur.close()
    return out


def truncate_present(pg, schema: str, targets: list[str], existing: dict[str, dict[str, str]]):
    """Truncate the grok target tables that actually exist (CASCADE, restart seq)."""
    present = [f'"{schema}"."{t}"' for t in targets if t in existing]
    if not present:
        return
    cur = pg.cursor()
    cur.execute(f"TRUNCATE {', '.join(present)} RESTART IDENTITY CASCADE")
    pg.commit()
    cur.close()
    print(f"[truncate] {len(present)} grok target table(s) reset")


def build_plans(con, pg, schema) -> list[TablePlan]:
    src_tables = sqlite_tables(con)
    dst_types = pg_column_types(pg, schema) if pg else {}
    plans = []
    for go_name, pg_name in TABLE_MAP:
        p = TablePlan(source=go_name, target=pg_name)
        p.src_exists = go_name in src_tables and go_name not in _SKIP_SQLITE
        p.dst_exists = pg_name in dst_types
        if p.src_exists and p.dst_exists:
            src_cols = sqlite_columns(con, go_name)
            dst_cols = set(dst_types[pg_name].keys())
            # preferred raw-copy columns first, then the rest in source order
            ordered = [c for c in src_cols if c in _KEEP_RAW and c in dst_cols]
            ordered += [c for c in src_cols if c not in _KEEP_RAW and c in dst_cols]
            p.columns = [c for c in ordered if c not in ("rowid",)]
        plans.append(p)
    return plans


def _iter_source(con, table, cols, limit):
    col_sql = ", ".join(f'"{c}"' for c in cols)
    cur = con.execute(f'SELECT {col_sql} FROM "{table}"')
    n = 0
    try:
        for row in cur:
            yield row
            n += 1
            if limit is not None and n >= limit:
                break
    finally:
        cur.close()


def _batched(gen, size):
    batch = []
    for item in gen:
        batch.append(item)
        if len(batch) >= size:
            yield batch
            batch = []
    if batch:
        yield batch


def run_copy(pg, schema: str, plans: list[TablePlan],
             existing: dict[str, dict[str, str]], limit: int | None,
             con: sqlite3.Connection) -> dict[str, int]:
    """Execute COPY for all copyable tables. Returns {target: rows_copied}."""
    from psycopg2 import extras

    copied: dict[str, int] = {}
    for plan in plans:
        if not (plan.src_exists and plan.dst_exists):
            print(f"[skip]   {plan.source} -> {plan.target}: missing source or target")
            continue
        pg_types = existing.get(plan.target, {})
        cols = [c for c in plan.columns if c in pg_types]
        if not cols:
            print(f"[skip]   {plan.source} -> {plan.target}: no intersecting columns")
            continue

        col_list = ", ".join(f'"{c}"' for c in cols)
        insert_sql = f'INSERT INTO "{schema}"."{plan.target}" ({col_list}) VALUES %s'

        def make_row(raw):
            out = []
            for c, v in zip(cols, raw):
                t = pg_types[c]
                if "json" in t.lower():
                    out.append(extras.Json(v))
                else:
                    out.append(_coerce(t, v))
            return tuple(out)

        rows = 0
        with pg.cursor() as cur:
            gen = (make_row(r) for r in _iter_source(con, plan.source, cols, limit))
            try:
                for batch in _batched(gen, 200):
                    extras.execute_values(cur, insert_sql, batch, page_size=200)
                    rows += len(batch)
            except Exception as exc:  # noqa: BLE001
                pg.rollback()
                print(f"[warn]   {plan.source} -> {plan.target}: copy failed ({exc})", file=sys.stderr)
                continue
        pg.commit()
        copied[plan.target] = rows
        print(f"[copy]    {plan.source} -> {plan.target}: {rows} rows ({len(cols)} cols)")
    return copied


def _safe_counts(con, pg, schema, src, dst):
    sc = con.execute(f'SELECT COUNT(*) FROM "{src}"').fetchone()[0]
    with pg.cursor() as cur:
        cur.execute(f'SELECT COUNT(*) FROM "{schema}"."{dst}"')
        pc = cur.fetchone()[0]
    return sc, pc


def identity_key_smoke(con, pg, schema, limit=10) -> tuple[int, int]:
    """Compare identity_key by id between source and PG accounts (sample)."""
    src_rows = con.execute(
        'SELECT id, identity_key FROM provider_accounts ORDER BY id LIMIT ?', (limit,)).fetchall()
    src = dict(src_rows)
    with pg.cursor() as cur:
        cur.execute(
            f'SELECT id, identity_key FROM "{schema}"."grok_accounts" ORDER BY id LIMIT %s',
            (limit,))
        dst = dict(cur.fetchall())
    if not src:
        return 0, 0
    matched = sum(1 for k, v in src.items() if dst.get(k) == v)
    return len(src), matched


def decrypt_smoke(encoded_key, con, limit=10) -> tuple[int, int]:
    """Optional AES-256-GCM decrypt smoke on account_credentials ciphertext.

    Matches Go infra/security/cipher.go: base64 RawStdEncoding of nonce(12)||ct.
    key = base64 StdEncoding of 32 bytes. Non-blocking (records only).
    """
    try:
        key = base64.b64decode(encoded_key, validate=True)
    except Exception:  # noqa: BLE001
        print("[warn] GROK_CREDENTIAL_KEY invalid base64 — skip decrypt smoke", file=sys.stderr)
        return 0, 0
    if len(key) != 32:
        print("[warn] GROK_CREDENTIAL_KEY must be 32 bytes — skip decrypt smoke", file=sys.stderr)
        return 0, 0
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM  # type: ignore

    cur = con.execute(
        "SELECT encrypted_primary FROM account_credentials "
        "WHERE encrypted_primary <> '' LIMIT ?", (limit,))
    rows = [r[0] for r in cur.fetchall()]
    cur.close()
    ok = 0
    for enc in rows:
        try:
            data = base64.b64decode(enc)
        except Exception:  # noqa: BLE001
            continue
        if not data:
            ok += 1
            continue
        try:
            nonce, ct = data[:12], data[12:]
            AESGCM(key).decrypt(nonce, ct, None)
            ok += 1
        except Exception:  # noqa: BLE001
            pass
    return len(rows), ok


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="grok2api SQLite -> TNexus PG ETL")
    ap.add_argument("--dry-run", action="store_true",
                    help="read SQLite, print plan; do not require/contact PG")
    ap.add_argument("--limit", type=int, default=None,
                    help="only copy first N rows per table (smoke)")
    ap.add_argument("--schema", default="public", help="PG schema for grok tables")
    ap.add_argument("--identity-smoke", type=int, default=10,
                    help="accounts to sample for identity_key compare (0=skip)")
    ap.add_argument("--decrypt-smoke", type=int, default=10,
                    help="credentials to attempt AES-GCM decrypt (0=skip)")
    args = ap.parse_args(argv)

    source = os.environ.get("GROK_ETL_SOURCE", "")
    dsn = os.environ.get("GROK_ETL_PG_DSN", "")
    key = os.environ.get("GROK_CREDENTIAL_KEY", "")

    if not source:
        print("Set GROK_ETL_SOURCE (path to backend.db)", file=sys.stderr)
        return 2
    if not os.path.isfile(source):
        print(f"Missing SQLite: {source}", file=sys.stderr)
        return 1

    try:
        con = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    except sqlite3.Error as exc:
        print(f"Failed to open SQLite {source}: {exc}", file=sys.stderr)
        return 1

    src_tables = sqlite_tables(con)
    print(f"[sqlite] {len(src_tables)} tables in {source}")
    for go_name, _ in TABLE_MAP:
        if go_name not in src_tables:
            print(f"  [missing-source] {go_name}")

    # --- PG connect (optional in dry-run) ------------------------------------
    pg = None
    pg_ok = False
    if args.dry_run:
        print("[mode] dry-run — PG not contacted")
    elif not dsn:
        print("[warn] GROK_ETL_PG_DSN not set — drawing plan only (no copy)", file=sys.stderr)
    else:
        try:
            import psycopg2
            pg = psycopg2.connect(dsn)
            pg_ok = True
        except Exception as exc:  # noqa: BLE001
            print(f"[warn] PG connect failed: {exc}", file=sys.stderr)

    plans = build_plans(con, pg, args.schema)
    active = [p for p in plans if p.target not in _SKIP_ETL_TARGETS]
    copyable = [p for p in active if p.src_exists and p.dst_exists]
    print(f"\n[plan] {len(copyable)}/{len(TABLE_MAP)} table families copyable "
          f"({sum(len(p.columns) for p in active)} intersecting columns)")

    copied: dict[str, int] = {}
    if pg_ok:
        existing = pg_column_types(pg, args.schema)
        truncate_present(pg, args.schema, [p.target for p in active], existing)
        copied = run_copy(pg, args.schema, active, existing, args.limit, con)
        pg.commit()

    # --- validation -----------------------------------------------------------
    if pg_ok:
        print("\n[validate] per-table count compare (sqlite vs pg):")
        for p in plans:
            if not (p.src_exists and p.dst_exists):
                continue
            sc, pc = _safe_counts(con, pg, args.schema, p.source, p.target)
            mark = "OK " if sc == pc else "DIFF"
            print(f"  [{mark}] {p.source:<28} sqlite={sc:<6} pg={pc}")

        if args.identity_smoke:
            n_src, n_match = identity_key_smoke(con, pg, args.schema, args.identity_smoke)
            print(f"[validate] identity_key smoke: {n_match}/{n_src} sampled accounts match")

        if args.decrypt_smoke and key:
            n_cred, n_ok = decrypt_smoke(key, con, args.decrypt_smoke)
            status = ""
            if n_cred and n_ok < n_cred:
                status = " (partial — often wrong/absent key)"
            print(f"[validate] decrypt smoke: {n_ok}/{n_cred} credential sample(s){status}")
        else:
            print("[validate] decrypt smoke skipped (GROK_CREDENTIAL_KEY unset or --decrypt-smoke 0)")
        if copied and args.limit:
            print(f"[note] --limit {args.limit} used (smoke only; re-run without to load all rows)")
        pg.close()
    else:
        # No PG: still surface source counts from SQLite.
        print("\n[validate] source-only counts (PG not connected):")
        for go_name, _ in TABLE_MAP:
            if go_name in src_tables and go_name not in _SKIP_SQLITE:
                row = con.execute(f'SELECT COUNT(*) FROM "{go_name}"').fetchone()[0]
                print(f"  [src] {go_name:<28} sqlite={row}")

    provider = con.execute("SELECT COUNT(*) FROM provider_accounts").fetchone()[0]
    print(f"\nprovider_accounts rows: {provider}")
    con.close()

    if not pg_ok and not args.dry_run:
        print("[result] PG copy not performed (no DSN / connect) — plan + source validation only")
        return 0
    print("[result] ETL done")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
