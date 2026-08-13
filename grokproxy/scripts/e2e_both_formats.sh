#!/usr/bin/env bash
# End-to-end: newapi -> grokProxy -> account pool -> xAI, in both protocols.
#
# Exercised through newapi rather than against grokProxy directly, because the
# translation only matters if newapi's own channel driver accepts the result.
set -u
NEWAPI="${NEWAPI:-http://127.0.0.1:8081}"

# Tokens are per-group, and the group is what selects the channel: `grok` lands
# on the OpenAI channel, `grok-claude` on the Anthropic one.
tok_openai="${TOKEN_OPENAI:-}"
tok_claude="${TOKEN_CLAUDE:-}"

# newapi soft-deletes tokens, so a row with status=1 can still be invisible to
# it. Without the deleted_at filter this picks a tombstone and every call comes
# back "Invalid token".
lookup() {
  docker exec new-api-postgres psql -U newapi -d new-api -A -t -c \
    "SELECT key FROM tokens
      WHERE \"group\"='$1' AND status=1 AND deleted_at IS NULL AND key NOT LIKE '%-%'
      ORDER BY unlimited_quota DESC, id DESC LIMIT 1;" | tr -d '[:space:]'
}
[[ -n "$tok_openai" ]] || tok_openai="$(lookup grok)"
[[ -n "$tok_claude" ]] || tok_claude="$(lookup grok-claude)"

# The Anthropic group is new, so it may have no token yet; mint one scoped to
# exactly that group. The key must be hyphen-free: newapi splits the presented
# key on "-" and looks up only the first segment, so any hyphen truncates it
# into a key that does not exist.
if [[ -z "$tok_claude" ]]; then
  echo ">>> no grok-claude token yet, creating one"
  key="gpanth$(tr -dc 'A-Za-z0-9' </dev/urandom | head -c 42)"
  docker exec new-api-postgres psql -U newapi -d new-api -q -c "
    INSERT INTO tokens (user_id, key, status, name, created_time, accessed_time,
                        expired_time, remain_quota, unlimited_quota, \"group\")
    SELECT user_id, '${key}', 1, 'grokproxy anthropic e2e',
           $(date +%s), $(date +%s), -1, 0, true, 'grok-claude'
      FROM tokens WHERE \"group\"='grok' AND deleted_at IS NULL LIMIT 1;"
  tok_claude="$key"
fi

pretty() { python3 -c 'import json,sys
raw = sys.stdin.read()
try:
    d = json.loads(raw)
except Exception:
    print("   非 JSON 响应:", raw[:200]); raise SystemExit
if "error" in d and d.get("error"):
    print("   FAIL:", json.dumps(d["error"], ensure_ascii=False)[:300]); raise SystemExit
if d.get("type") == "error":
    print("   FAIL:", json.dumps(d.get("error"), ensure_ascii=False)[:300]); raise SystemExit
if "choices" in d:
    print("   OK  reply:", repr(d["choices"][0]["message"]["content"][:80]))
    print("       usage:", d.get("usage", {}).get("total_tokens"), "tokens")
elif d.get("type") == "message":
    print("   OK  reply:", repr(d["content"][0]["text"][:80]))
    u = d.get("usage", {})
    print("       stop_reason:", d.get("stop_reason"),
          " in/out:", u.get("input_tokens"), "/", u.get("output_tokens"))
else:
    print("   意外的响应形状:", json.dumps(d, ensure_ascii=False)[:300])'
}

# Some stored keys already carry the prefix; double-prefixing reads as a
# different, non-existent token.
with_prefix() { case "$1" in sk-*) printf '%s' "$1" ;; *) printf 'sk-%s' "$1" ;; esac; }
key_openai="$(with_prefix "$tok_openai")"
key_claude="$(with_prefix "$tok_claude")"
echo ">>> openai token ...${key_openai: -8}   anthropic token ...${key_claude: -8}"

echo
echo "=== 1. OpenAI format: POST /v1/chat/completions (group=grok) ==="
curl -s --max-time 180 -X POST "$NEWAPI/v1/chat/completions" \
  -H "Authorization: Bearer ${key_openai}" -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","messages":[{"role":"user","content":"Reply with exactly: openai ok"}],"max_tokens":24}' \
  | pretty

echo
echo "=== 2. Anthropic format: POST /v1/messages (group=grok-claude) ==="
curl -s --max-time 180 -X POST "$NEWAPI/v1/messages" \
  -H "x-api-key: ${key_claude}" -H 'anthropic-version: 2023-06-01' \
  -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","max_tokens":24,"system":"Be terse.","messages":[{"role":"user","content":[{"type":"text","text":"Reply with exactly: anthropic ok"}]}]}' \
  | pretty

echo
echo "=== 3. grokProxy direct, both protocols (bypasses newapi) ==="
set -a; . /opt/grokproxy/.env; set +a
echo "--- /v1/chat/completions ---"
curl -s --max-time 180 -X POST http://127.0.0.1:8110/v1/chat/completions \
  -H "Authorization: Bearer $GROKPROXY_API_KEY" -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","messages":[{"role":"user","content":"say direct-openai"}],"max_tokens":16}' \
  | pretty
echo "--- /v1/messages ---"
curl -s --max-time 180 -X POST http://127.0.0.1:8110/v1/messages \
  -H "x-api-key: $GROKPROXY_API_KEY" -H 'Content-Type: application/json' \
  -d '{"model":"grok-4.6","max_tokens":16,"messages":[{"role":"user","content":"say direct-anthropic"}]}' \
  | pretty

echo
echo "=== scheduler state after the run ==="
curl -s --max-time 20 http://127.0.0.1:8110/api/v1/stats \
  -H "Authorization: Bearer $GROKPROXY_ADMIN_KEY" > /tmp/stats.json
python3 - <<'PY'
import json
d = json.load(open('/tmp/stats.json'))
print("  scheduler:", d.get("scheduler"))
q = (d.get("build") or {}).get("quota", {})
measured = q.get("measured_accounts")
remaining = q.get("remaining_tokens", 0)
print(f"  build measured={measured}  remaining={remaining:,} tokens")
PY
