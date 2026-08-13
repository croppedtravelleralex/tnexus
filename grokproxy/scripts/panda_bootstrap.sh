#!/usr/bin/env bash
# One-time bootstrap for grokproxy on Panda: directories + .env only.
# 不构建、不传二进制。镜像由 CI 推到 GHCR，部署用 grokproxy-deploy.sh 拉取。
#
# Keys are generated here on the host so they never travel from a workstation.
# Re-running is safe: existing keys are kept.
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ROOT="${GROKPROXY_ROOT:-/opt/grokproxy}"
ENV_FILE="$ROOT/.env"
EXAMPLE="$TNEXUS_ROOT/grokproxy/deploy-env.example"

mkdir -p "$ROOT/data"

if [[ ! -f "$ENV_FILE" ]]; then
  [[ -f "$EXAMPLE" ]] || { echo "missing $EXAMPLE — git -C $TNEXUS_ROOT pull first" >&2; exit 1; }
  cp "$EXAMPLE" "$ENV_FILE"
  chmod 600 "$ENV_FILE"
  echo "created $ENV_FILE from example"
fi

fill_key() {
  local name="$1"
  local current
  current="$(grep -E "^${name}=" "$ENV_FILE" | cut -d= -f2- || true)"
  if [[ -z "$current" ]]; then
    local value
    value="$(openssl rand -hex 32)"
    # Portable in-place edit; the file is tiny.
    awk -v k="$name" -v v="$value" \
      'BEGIN{FS=OFS="="} $1==k{print k"="v; next} {print}' \
      "$ENV_FILE" > "$ENV_FILE.tmp" && mv "$ENV_FILE.tmp" "$ENV_FILE"
    chmod 600 "$ENV_FILE"
    echo "generated $name"
  else
    echo "$name already set — kept"
  fi
}

fill_key GROKPROXY_API_KEY
fill_key GROKPROXY_ADMIN_KEY

echo
echo "--- effective config (keys masked) ---"
sed -E 's/^(GROKPROXY_(API|ADMIN)_KEY)=.+/\1=***/' "$ENV_FILE" | grep -v '^\s*#' | grep -v '^\s*$'
echo
echo "next: bash $TNEXUS_ROOT/deploy/panda/grokproxy-deploy.sh"
