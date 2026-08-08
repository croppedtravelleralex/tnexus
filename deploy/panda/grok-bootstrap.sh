#!/usr/bin/env bash
# Panda：grok2api SQLite 号池 ETL → PG + grok2api-rs 部署（禁止 build）
set -euo pipefail

TNEXUS_ROOT="${TNEXUS_ROOT:-/root/TNexus}"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
SQLITE="${GROK_ETL_SOURCE:-/opt/grok2api/data/backend.db}"
CRED_KEY_LINE="$(grep credentialEncryptionKey /opt/grok2api/config.yaml 2>/dev/null | head -1 || true)"

echo "[grok-bootstrap] TNexus root: $TNEXUS_ROOT"
echo "[grok-bootstrap] SQLite source: $SQLITE"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE" >&2
  exit 1
fi
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

if [[ -z "${GROK_DATABASE_URL:-}" ]]; then
  echo "GROK_DATABASE_URL missing in $ENV_FILE" >&2
  exit 1
fi

# 从 grok2api config 同步凭据解密密钥（若 .env 未设）
if [[ -z "${GROK_CREDENTIAL_KEY:-}" && -n "$CRED_KEY_LINE" ]]; then
  export GROK_CREDENTIAL_KEY="$(echo "$CRED_KEY_LINE" | cut -d: -f2- | tr -d ' \"')"
  echo "[grok-bootstrap] synced GROK_CREDENTIAL_KEY from grok2api config.yaml"
fi

export GROK_ETL_SOURCE="$SQLITE"
export GROK_ETL_PG_DSN="$GROK_DATABASE_URL"

if [[ -f "$SQLITE" ]]; then
  echo "[grok-bootstrap] running ETL SQLite → PG ..."
  python3 "$TNEXUS_ROOT/scripts/grok_etl_sqlite_to_pg.py" || {
    echo "WARN: ETL failed (migrations missing?); continuing deploy" >&2
  }
else
  echo "WARN: SQLite not found at $SQLITE — skip ETL" >&2
fi

echo "[grok-bootstrap] grok-deploy.sh ..."
bash "$TNEXUS_ROOT/deploy/panda/grok-deploy.sh" deploy

echo "[grok-bootstrap] health checks"
curl -fsS http://127.0.0.1:8000/readyz && echo " grok2api-rs ready"
curl -fsS http://127.0.0.1:8091/healthz 2>/dev/null && echo " grok-admin ready" || echo " grok-admin :8091 (embedded in grok2api-rs)"

echo "[grok-bootstrap] done"
