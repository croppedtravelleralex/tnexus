#!/usr/bin/env bash
# Grok sidecar deploy — see docs/39-grok2api-rust-migration.md §G6-P2.
# ============================================================
# 禁止在 Panda 构建镜像。链路：本地/CI 构建 → GHCR → 本脚本仅 pull + up（绝不 docker build / cargo build）。
#
# 用法：
#   GROK_DATABASE_URL=... ./grok-deploy.sh                 # 部署/升级 latest
#   GROK_TAG=<sha> ./grok-deploy.sh                         # 部署指定版本
#   ./grok-deploy.sh rollback                               # 回滚到上一次部署版本（自动读 .grok-prev-deploy.txt）
#   ROLLBACK_TAG=<prev-sha> ./grok-deploy.sh rollback       # 回滚到指定版本（G6-A4 ≤15min）
#
# 环境：TNEXUS_ROOT（默认 /root/TNexus）、ENV_FILE（默认 /opt/tnexus/.env）、
#       GHCR_OWNER、GROK_TAG（默认 latest）、ROLLBACK_TAG。
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$TNEXUS_ROOT/deploy/panda/grok-compose.yml"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"

if [[ ! -f "$COMPOSE_FILE" ]]; then
  echo "missing $COMPOSE_FILE — git pull $TNEXUS_ROOT first" >&2
  exit 1
fi
if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE — bootstrap from deploy/panda/.env.example first" >&2
  exit 1
fi

# 镜像 owner 默认 croppedtravelleralex（与 ghcr-image.yml 的 GHCR job 一致）。
ghcr_owner="${GHCR_OWNER:-croppedtravelleralex}"

# 版本记录：.grok-last-deploy.txt = 当前部署版本；.grok-prev-deploy.txt = 上一个版本。
last_tag_file="$TNEXUS_ROOT/.grok-last-deploy.txt"
prev_tag_file="$TNEXUS_ROOT/.grok-prev-deploy.txt"

load_env() {
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  export GHCR_OWNER="$ghcr_owner" GROK_TAG="$1"
}

up_and_probe() {
  docker compose --env-file "$ENV_FILE" \
    -f "$COMPOSE_FILE" pull
  docker compose --env-file "$ENV_FILE" \
    -f "$COMPOSE_FILE" up -d --force-recreate

  sleep 4
  if curl -fsS http://127.0.0.1:8000/readyz >/dev/null 2>&1; then
    echo "grok:8000 ready"
  else
    echo "WARN: grok:8000 /readyz not healthy yet" >&2
  fi
}

deploy() {
  local tag="${GROK_TAG:-latest}"
  echo ">>> merging .env + pulling ghcr.io/$ghcr_owner/grok*:$tag"
  load_env "$tag"

  # 部署前记录上一版本，供 rollback 自动回退。
  local prev=""
  if [[ -f "$last_tag_file" ]]; then
    prev="$(cat "$last_tag_file")"
  fi
  echo "$tag" > "$last_tag_file"
  if [[ -n "$prev" && "$prev" != "$tag" ]]; then
    echo "$prev" > "$prev_tag_file"
  fi

  up_and_probe

  # 部署后按 pure_http_keys 同步 grok_web enabled（有 key 启用，无 key 禁用）。
  if [[ -x "$TNEXUS_ROOT/scripts/sync_grok_enabled_from_keys.sh" ]] \
    && [[ -n "${GROK_DATABASE_URL:-}" ]]; then
    bash "$TNEXUS_ROOT/scripts/sync_grok_enabled_from_keys.sh" \
      --keys-dir "${GROK_PURE_HTTP_KEYS_DIR:-/opt/tnexus/pure_http_keys}" \
      --apply || echo "WARN: grok keys sync failed (non-fatal)" >&2
  fi
}

rollback() {
  local tag="${ROLLBACK_TAG:-}"
  if [[ -z "$tag" ]]; then
    # 未显式指定 → 自动读上一版本（无则尝试当前版本，再失败即退出）。
    tag="$(cat "$prev_tag_file" 2>/dev/null || true)"
    if [[ -z "$tag" ]]; then
      tag="$(cat "$last_tag_file" 2>/dev/null || true)"
    fi
    if [[ -z "$tag" ]]; then
      echo "ROLLBACK_TAG 未设置且无版本记录（.grok-prev-deploy.txt / .grok-last-deploy.txt）" >&2
      exit 1
    fi
    echo ">>> 未传 ROLLBACK_TAG，自动回滚到记录版本 $tag"
  fi
  echo ">>> rollback to ghcr.io/$ghcr_owner/grok*:$tag"
  load_env "$tag"

  up_and_probe
  echo "$tag" > "$last_tag_file"
  echo ">>> rollback complete (downtime ≈ pull + up seconds)"
}

case "${1:-deploy}" in
  deploy) deploy ;;
  rollback) rollback ;;
  status)
    docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" ps ;;
  *) echo "usage: $0 [deploy|rollback|status]" >&2; exit 2 ;;
esac
