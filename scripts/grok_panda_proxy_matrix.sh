#!/usr/bin/env bash
# Panda：Rust grok-pure-http 四路代理 gate 矩阵
# 直连 / udeal / webshare 机房 / webshare 住宅
#
# 前置：GHCR 镜像已含 grok-pure-http 二进制，或本机 scp 静态二进制到 Panda。
# 禁止在 Panda 上 cargo build / docker build（见 .cursor/rules/panda-no-remote-build.mdc）
#
# 用法（Panda）：
#   KEYS_JSON=/opt/grok2api/data/pure_http_keys/nancybaker_at_yumail.co.json \
#   IMAGE=/tmp/probe.png \
#   bash /root/TNexus/scripts/grok_panda_proxy_matrix.sh

set -euo pipefail

ROOT="${TNEXUS_ROOT:-/root/TNexus}"
KEYS_JSON="${KEYS_JSON:-/opt/grok2api/data/pure_http_keys/nancybaker2jyy_at_yumail.co.json}"
IMAGE="${IMAGE:-/tmp/grok_ocr_probe.png}"
OUT_DIR="${OUT_DIR:-/tmp/grok_proxy_matrix}"
BIN="${GROK_PURE_HTTP_BIN:-grok-pure-http}"
LOCAL_PROXY="${GROK_LOCAL_PROXY:-http://127.0.0.1:7897}"

# udeal：SQLite egress_nodes.id=110 或显式 URL
UDEA_PROXY="${GROK_EGRESS_PROXY:-}"
WS_DC_FILE="${WEBSHARE_DC_FILE:-/opt/tnexus/webshare-dc-proxies.txt}"
WS_RES_FILE="${WEBSHARE_RES_FILE:-/opt/tnexus/webshare-proxies.txt}"

mkdir -p "$OUT_DIR"

pick_first_proxy() {
  local file="$1"
  [[ -f "$file" ]] || return 1
  grep -v '^#' "$file" | grep -v '^[[:space:]]*$' | head -1
}

fmt_proxy_url() {
  local line="$1"
  if [[ "$line" == http* ]]; then
    echo "$line"
  elif [[ "$line" == *@* ]]; then
    echo "http://$line"
  else
    # host:port:user:pass
    local host port user pass
    IFS=':' read -r host port user pass <<<"$line"
    echo "http://${user}:${pass}@${host}:${port}"
  fi
}

run_case() {
  local label="$1"
  local upstream="${2:-}"
  local out="$OUT_DIR/gate_${label}.json"
  echo "=== [$label] upstream=${upstream:-<direct>} ==="
  env GROK_LOCAL_PROXY="$LOCAL_PROXY" GROK_UPSTREAM_PROXY="$upstream" \
    "$BIN" \
      --keys "$KEYS_JSON" \
      --image "$IMAGE" \
      --proxy-label "$label" \
      --gate \
    | tee "$out" || echo "FAIL: $label" | tee -a "$OUT_DIR/failures.txt"
}

# 1) 直连（仅 local proxy 出 meta/签）
run_case "direct" ""

# 2) udeal
if [[ -n "$UDEA_PROXY" ]]; then
  run_case "udeal" "$UDEA_PROXY"
else
  echo "SKIP udeal: set GROK_EGRESS_PROXY" | tee -a "$OUT_DIR/skipped.txt"
fi

# 3) webshare 机房（取列表首行）
if line="$(pick_first_proxy "$WS_DC_FILE")"; then
  run_case "webshare_dc" "$(fmt_proxy_url "$line")"
else
  echo "SKIP webshare_dc: missing $WS_DC_FILE" | tee -a "$OUT_DIR/skipped.txt"
fi

# 4) webshare 住宅
if line="$(pick_first_proxy "$WS_RES_FILE")"; then
  run_case "webshare_residential" "$(fmt_proxy_url "$line")"
else
  echo "SKIP webshare_residential: missing $WS_RES_FILE" | tee -a "$OUT_DIR/skipped.txt"
fi

echo "--- summary in $OUT_DIR ---"
ls -la "$OUT_DIR"
