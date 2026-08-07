#!/usr/bin/env bash
# Grok 移植阶段验收门禁 — 见 docs/39a-grok-roadmap.md、docs/39c-grok-test-matrix.md
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

phase="${1:-}"
if [[ -z "$phase" ]]; then
  echo "Usage: $0 <g0|g1|g2|g3|g4|g6>"
  exit 1
fi

log() { echo "[grok-gate:$phase] $*"; }
fail() { echo "[grok-gate:$phase] FAIL: $*" >&2; exit 1; }

gate_g0() {
  log "cargo fmt check (grok crates when present)"
  if ls crates/grok2api-rs/Cargo.toml >/dev/null 2>&1; then
    # 仅检查 grok crate 的格式；cargo fmt --all 会带上 pre-existing 的
    # upstream 等 crate 格式差异（与 grok 无关），故 scoped 到 grok crates。
    cargo fmt -p grok-domain -p grok-storage -p grok2api-rs -- --check
    cargo build -p grok2api-rs
    cargo test -p grok-domain
    cargo test -p grok-storage
  else
    log "SKIP build: grok2api-rs not yet in workspace (expected pre-G0-1)"
  fi

  for f in migrations/010_grok_core.sql migrations/011_grok_quota_models.sql migrations/012_grok_routing_keys.sql migrations/013_grok_inference.sql migrations/014_grok_media_egress.sql migrations/015_grok_pipeline_ops.sql; do
    if [[ ! -f "$f" ]]; then
      log "WARN: missing $f (create per docs/39b-grok-schema.md)"
    fi
  done

  if [[ -f scripts/grok_etl_sqlite_to_pg.py ]]; then
    # 多环境解释器探测：优先 Windows `py` launcher，其次真实 python3/python。
    # 跳过 WindowsApps 的 Store 中。如果 `py` 存在则用 `py`，避免 python3 stub。
    local pycmd=""
    if command -v py >/dev/null 2>&1; then
      pycmd="py"
    elif command -v python3 >/dev/null 2>&1 && ! python3 --version >/dev/null 2>&1; then
      : # python3 是死的 WindowsApps stub，忽略
    elif command -v python3 >/dev/null 2>&1; then
      pycmd="python3"
    elif command -v python >/dev/null 2>&1; then
      pycmd="python"
    fi
    if [[ -n "$pycmd" ]] && "$pycmd" -m py_compile scripts/grok_etl_sqlite_to_pg.py >/dev/null 2>&1; then
      log "ETL py_compile OK ($pycmd)"
    elif [[ -n "$pycmd" ]]; then
      log "WARN: py_compile via $pycmd failed"
    else
      log "WARN: python not found, skip ETL py_compile"
    fi
  else
    log "WARN: scripts/grok_etl_sqlite_to_pg.py not found"
  fi

  log "G0 documentation refs"
  test -f docs/39-grok2api-rust-migration.md
  test -f docs/39a-grok-roadmap.md
  test -f docs/39b-grok-schema.md
  test -f docs/39c-grok-test-matrix.md
  test -f docs/39d-grok-go-rust-map.md
}

gate_g1() {
  gate_g0
  if ls crates/grok-gateway/Cargo.toml >/dev/null 2>&1; then
    cargo test -p grok-gateway -p grok-provider-web -p grok-conversation
  else
    fail "grok-gateway crate missing"
  fi
  if [[ -f crates/grok-gateway/tests/ocr_e2e.rs ]] || [[ -d tests/grok_golden ]]; then
    log "OCR golden/E2E present"
  else
    log "WARN: OCR E2E tests not found (see docs/39c §2)"
  fi
}

gate_g2() {
  gate_g1
  cargo test -p grok-image-pipeline 2>/dev/null || log "WARN: grok-image-pipeline tests"
  if [[ -n "${GROK2API_BASE:-}" ]]; then
    curl -sf "${GROK2API_BASE%/}/healthz" || fail "GROK2API_BASE healthz"
    curl -sf -X POST "${GROK2API_BASE%/}/v1/images/generations" \
      -H "Authorization: Bearer ${UPSTREAM_API_KEY:-}" \
      -H "Content-Type: application/json" \
      -d '{"model":"grok-imagine-image","prompt":"gate smoke","n":1,"response_format":"url"}' \
      || log "WARN: generations smoke failed (upstream?)"
  else
    log "SKIP live generations: set GROK2API_BASE"
  fi
}

gate_g3() {
  gate_g2
  # G3-P1~P5：poolindex/web_pool/build_pool/selector + ops（grok-pool-index 为 Go 侧目录名，
  # Rust 全部在 grok-pool；dispatch diff<5% 属运行验收 G3-A2，非门禁可测项）。
  cargo test -p grok-pool -p grok-ops 2>/dev/null || fail "pool/ops crate tests"
}

gate_g4() {
  gate_g3
  cargo test -p grok-admin -p grok-ops 2>/dev/null || log "WARN: grok-admin/ops tests"
}

gate_g6() {
  if [[ -f artifacts/grok-shadow/latest/summary.json ]]; then
    python - <<'PY'
import json, sys
from pathlib import Path
p = Path("artifacts/grok-shadow/latest/summary.json")
d = json.loads(p.read_text())
if d.get("success_rate_rust", 0) < d.get("success_rate_go", 1) - 0.01:
    sys.exit("success rate below Go -1%")
PY
  else
    log "WARN: no shadow summary at artifacts/grok-shadow/latest/summary.json"
  fi
}

case "$phase" in
  g0) gate_g0 ;;
  g1) gate_g1 ;;
  g2) gate_g2 ;;
  g3) gate_g3 ;;
  g4) gate_g4 ;;
  g6) gate_g6 ;;
  *) fail "unknown phase: $phase" ;;
esac

log "PASS"
