#!/usr/bin/env bash
# Grok sidecar deploy (draft) — see docs/39-grok2api-rust-migration.md §G6-P2.
# ============================================================
# 草案：合并到 main 前需人工 review。禁止在 Panda 构建镜像。
# 链路：本地/CI 构建 → GHCR → 本脚本仅 pull + up（绝不 docker build / cargo build）。
#
# 用法：
#   GROK_DATABASE_URL=... ./grok-deploy.sh                 # 部署/升级 latest
#   GROK_TAG=<sha> ./grok-deploy.sh                         # 部署指定版本
#   ROLLBACK_TAG=<prev-sha> ./grok-deploy.sh rollback       # 回滚（G6-A4 ≤15min）
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

deploy() {
  local tag="${GROK_TAG:-latest}"
  echo ">>> merging .env + pulling ghcr.io/$ghcr_owner/grok*:$tag"
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  export GHCR_OWNER="$ghcr_owner" GROK_TAG="$tag"

  # git 里保留最新成功 tag，供 rollback 用（部署前记录）。
  echo "$tag" > "$TNEXUS_ROOT/.grok-last-deploy.txt"

  docker compose --env-file "$ENV_FILE" --profile admin \
    -f "$COMPOSE_FILE" pull
  docker compose --env-file "$ENV_FILE" --profile admin \
    -f "$COMPOSE_FILE" up -d --force-recreate

  sleep 4
  if curl -fsS http://127.0.0.1:8000/readyz >/dev/null 2>&1; then
    echo "grok:8000 ready"
  else
    echo "WARN: grok:8000 /readyz not healthy yet" >&2
  fi
}

rollback() {
  local tag="${ROLLBACK_TAG:?need ROLLBACK_TAG=<prev-sha>}"
  echo ">>> rollback to ghcr.io/$ghcr_owner/grok*:$tag"
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  export GHCR_OWNER="$ghcr_owner" GROK_TAG="$tag"

  docker compose --env-file "$ENV_FILE" --profile admin \
    -f "$COMPOSE_FILE" pull
  docker compose --env-file "$ENV_FILE" --profile admin \
    -f "$COMPOSE_FILE" up -d --force-recreate
  echo "$tag" > "$TNEXUS_ROOT/.grok-last-deploy.txt"
  echo ">>> rollback complete (downtime ≈ pull + up seconds)"
}

case "${1:-deploy}" in
  deploy) deploy ;;
  rollback) rollback ;;
  status)
    docker compose --env-file "$ENV_FILE" --profile admin -f "$COMPOSE_FILE" ps ;;
  *) echo "usage: $0 [deploy|rollback|status]" >&2; exit 2 ;;
esac