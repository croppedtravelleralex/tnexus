#!/usr/bin/env bash
# Set grok-4.6 pricing in newapi.
#
# newapi stores model pricing as JSON in the options table:
#   ModelRatio       - prompt price, in units of $0.002 / 1K tokens
#   CompletionRatio  - completion price as a multiple of the prompt price
# Editing the JSON in place keeps every other model's pricing untouched.
set -euo pipefail

MODEL="${PRICE_MODEL:-grok-4.6}"
# grok-4.6 upstream list price is $3 / M input, $15 / M output.
#   3 / 1e6 * 1000 / 0.002 = 1.5
MODEL_RATIO="${PRICE_MODEL_RATIO:-1.5}"
#   15 / 3 = 5
COMPLETION_RATIO="${PRICE_COMPLETION_RATIO:-5}"

q() { docker exec -i new-api-postgres psql -U newapi -d new-api -A -t -c "$1"; }

patch_option() {
  local key="$1" model="$2" value="$3"
  local current
  current="$(q "select value from options where key='${key}';")"
  [[ -n "$current" ]] || current='{}'
  printf '%s' "$current" > /tmp/newapi_opt.json
  python3 - "$model" "$value" <<'PY'
import json, sys
model, value = sys.argv[1], float(sys.argv[2])
path = "/tmp/newapi_opt.json"
with open(path, encoding="utf-8") as fh:
    text = fh.read().strip() or "{}"
data = json.loads(text)
data[model] = value
with open(path, "w", encoding="utf-8") as fh:
    json.dump(data, fh, ensure_ascii=False, separators=(",", ":"))
PY
  local updated
  updated="$(cat /tmp/newapi_opt.json)"
  docker exec -i new-api-postgres psql -U newapi -d new-api -v ON_ERROR_STOP=1 -q <<SQL
INSERT INTO options (key, value) VALUES ('${key}', \$json\$${updated}\$json\$)
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value;
SQL
  echo "${key}: ${model} = ${value}"
}

patch_option ModelRatio "$MODEL" "$MODEL_RATIO"
patch_option CompletionRatio "$MODEL" "$COMPLETION_RATIO"

echo
echo "--- stored ---"
q "select key, (value::json->>'${MODEL}') from options where key in ('ModelRatio','CompletionRatio');"
echo
echo "newapi 需要重启或等配置热加载后生效"
