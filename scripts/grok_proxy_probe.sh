#!/usr/bin/env bash
# 采购代理前的准入筛选：哪些 IP 能真正到达 grok.com。
#
# 用法（在 Panda 上执行，只读，不改任何配置）：
#   bash scripts/grok_proxy_probe.sh /opt/tnexus/webshare_proxies.txt
#
# 列表格式：每行 host:port:user:pass（与 GROK2API_PROXY_LIST 一致），
# 或无认证的 host:port。
#
# 判读要点：
# - 会先用【当前生产代理】跑同一套请求作对照。对照拿不到 200 说明是测试环境
#   或 grok 侧出了问题，此时不要据此判定新代理不合格。
# - grok=200 才算可用；403 且耗时极短（<1s）是 IP 级封禁，加任何请求头都救不回来。
# - 宿主直连一直是 403（IP 已被硬封），属预期，仅作基线参考。
set -uo pipefail

LIST="${1:-/opt/tnexus/webshare_proxies.txt}"
ENV_FILE="${ENV_FILE:-/opt/tnexus/.env}"
UA='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
export UA

[[ -f "$LIST" ]] || { echo "missing proxy list: $LIST" >&2; exit 1; }

build_url() {
  local raw="$1" h p u pw
  if [[ "$raw" == http* ]]; then echo "$raw"; return; fi
  IFS=':' read -r h p u pw <<<"$raw"
  if [[ -n "${pw:-}" ]]; then echo "http://${u}:${pw}@${h}:${p}"; else echo "http://${h}:${p}"; fi
}
export -f build_url

probe() {
  local raw="$1" url code egress
  url=$(build_url "$raw")
  egress=$(curl -sS --max-time 20 -x "$url" https://api.ipify.org 2>/dev/null)
  code=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 30 -x "$url" \
    -H "User-Agent: $UA" \
    -H 'Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8' \
    -H 'Accept-Language: en-US,en;q=0.9' \
    https://grok.com/ 2>/dev/null || echo 000)
  printf '%-24s egress=%-16s grok=%s\n' "${raw%%:*}:$(echo "$raw" | cut -d: -f2)" "${egress:-NONE}" "$code"
}
export -f probe

echo "=== 基线：宿主直连（预期 403，IP 已被硬封）==="
curl -sS -o /dev/null -w '  direct grok.com=%{http_code} %{time_total}s\n' --max-time 15 https://grok.com/ \
  || echo '  direct 失败'

if [[ -f "$ENV_FILE" ]]; then
  CUR=$(grep -E '^(GROK2API_PROXY_LIST|GROK_UPSTREAM_PROXY)=' "$ENV_FILE" | head -1 | cut -d= -f2- | tr ',' '\n' | head -1)
  if [[ -n "${CUR:-}" ]]; then
    echo "=== 对照：当前生产代理（必须 200，否则本次结果不可信）==="
    probe "$CUR"
  fi
fi

echo
echo "=== 待检代理 ==="
grep -v '^[[:space:]]*$' "$LIST" | xargs -P 10 -I{} bash -c 'probe "$@"' _ {} | sort

echo
echo "=== 汇总 ==="
grep -v '^[[:space:]]*$' "$LIST" | xargs -P 10 -I{} bash -c 'probe "$@"' _ {} \
  | grep -oE 'grok=[0-9]+' | sort | uniq -c
echo "（只有 grok=200 的条目可以写进 GROK2API_PROXY_LIST）"
