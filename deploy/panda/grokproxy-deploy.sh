#!/usr/bin/env bash
# grokProxy deploy on Panda — pull + up only.
# ============================================================
# 禁止在 Panda 构建镜像。链路：git push → GitHub Actions → GHCR → 本脚本 pull + up。
# 本脚本绝不执行 docker build / cargo build。
#
# 用法：
#   ./deploy.sh                        # 部署 latest
#   GROKPROXY_TAG=<sha> ./deploy.sh    # 部署指定版本
#   ./deploy.sh rollback               # 回滚到上一次部署的版本
#   ./deploy.sh status                 # 查看状态
set -euo pipefail

# Compose lives in the TNexus checkout that already exists on this box; only the
# runtime state (env + sqlite) sits under /opt/grokproxy.
TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ROOT="${GROKPROXY_ROOT:-/opt/grokproxy}"
COMPOSE_FILE="${COMPOSE_FILE:-$TNEXUS_ROOT/deploy/panda/grokproxy-compose.yml}"
ENV_FILE="${ENV_FILE:-$ROOT/.env}"
last_tag_file="$ROOT/.last-deploy"
prev_tag_file="$ROOT/.prev-deploy"

[[ -f "$COMPOSE_FILE" ]] || { echo "missing $COMPOSE_FILE — git -C $TNEXUS_ROOT pull first" >&2; exit 1; }
[[ -f "$ENV_FILE" ]] || {
  echo "missing $ENV_FILE — bootstrap it first:" >&2
  echo "  mkdir -p $ROOT/data" >&2
  echo "  cp $TNEXUS_ROOT/grokproxy/deploy-env.example $ENV_FILE && edit it" >&2
  exit 1
}

compose() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

up_and_probe() {
  compose pull
  compose up -d --force-recreate
  local port
  port="$(grep -E '^GROKPROXY_PORT=' "$ENV_FILE" | cut -d= -f2 | tr -d '"' || true)"
  port="${port:-8110}"
  for _ in $(seq 1 15); do
    if curl -fsS "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1; then
      echo "grokproxy:${port} healthy"
      # readyz is allowed to fail here: an empty pool is not a bad deploy.
      curl -fsS "http://127.0.0.1:${port}/readyz" || echo "(pool still empty)"
      return 0
    fi
    sleep 2
  done
  echo "WARN: grokproxy did not become healthy in 30s" >&2
  compose logs --tail 40 grokproxy >&2 || true
  return 1
}

case "${1:-deploy}" in
  deploy)
    tag="${GROKPROXY_TAG:-latest}"
    prev="$(cat "$last_tag_file" 2>/dev/null || true)"
    echo "$tag" > "$last_tag_file"
    [[ -n "$prev" && "$prev" != "$tag" ]] && echo "$prev" > "$prev_tag_file"
    echo ">>> deploying grokproxy:$tag"
    GROKPROXY_TAG="$tag" up_and_probe
    ;;
  rollback)
    tag="${ROLLBACK_TAG:-$(cat "$prev_tag_file" 2>/dev/null || true)}"
    [[ -n "$tag" ]] || { echo "no previous version recorded" >&2; exit 1; }
    echo ">>> rolling back to grokproxy:$tag"
    GROKPROXY_TAG="$tag" up_and_probe
    echo "$tag" > "$last_tag_file"
    ;;
  status) compose ps ;;
  logs) compose logs --tail "${2:-80}" grokproxy ;;
  *) echo "usage: $0 [deploy|rollback|status|logs]" >&2; exit 2 ;;
esac
