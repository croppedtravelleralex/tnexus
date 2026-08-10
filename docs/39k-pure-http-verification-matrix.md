# 39k — 纯 HTTP / 网页对话验证矩阵（号池 × 环境 × 工具）

> 状态：**2026-08-10**。与 [39j-grok-pure-http-reverse-engineering.md](39j-grok-pure-http-reverse-engineering.md) 配套。  
> **验收口径**：`POST /rest/app-chat/conversations/new` 返回 **200** 且 SSE/JSON 含 `modelResponse.message`（或等价 token 流结束）。  
> **不算**：GET 只读、`load-responses` 读历史、WebSocket `mgw` PONG（另计）。

---

## 1. 两套号池（勿混）

| 池 | 来源 | 规模 | TNexus PG | 用途 |
|----|------|------|-----------|------|
| **grok2api 老池** | `/opt/grok2api/data/backend.db` ETL | ~706 → PG **672** `grok_web` | ✅ 生产调度用这批 | 历史批量注册/OAuth，**POST 全死** |
| **yumail 注册池** | `AutoRegister/grok` + mailmanage | `web_auths/` **700+** | ❌ 未灌 PG | 7 月批量 `@yumail.co`，**纯 HTTP 可活** |

gptimage **26 个 GPT 号**与 Grok 无关，不得导入 grok 池。

---

## 2. 两套探测工具（结果不可直接对比）

| | ✅ 能 POST chat 200 | ❌ Panda 全池扫用的 |
|--|---------------------|---------------------|
| 脚本 | `scripts/grok_pure_http_client.py` | `scripts/grok_pure_http_chat_probe.py`、`scripts/grok_pg_chat_probe.py` |
| 签名 | Python `generate_statsig`（`--signer auto`）+ `pure_http_keys` | Node `grok_sign_standalone.js`（1645e3） |
| Payload | 前端 schema（`message`/`modeId`/`deviceEnvInfo`） | 早期简化 body / OpenAI 风格 |
| Cookie | Playwright 提取的完整 session（含 `cf_clearance`） | 多数仅 `sso`/`sso-rw` |
| 典型号 | yumail 老号 | grok2api PG/SQLite 全量 |

---

## 3. 总矩阵（上次实测汇总）

### 3.1 `POST /rest/app-chat/conversations/new`（纯 HTTP 发消息）

| 号池 | 本机 Clash `7897` | Panda `70.39.164.200:30000` |
|------|-------------------|------------------------------|
| **grok2api 老池** | ❌ node/简化 probe 全扫 | ❌ **672/672 无活号**（node 签 + 裸 sso） |
| **grok2api 老池 + pure_http keys** | ✅ **86/304** gate 100%（Rust `grok-pure-http`） | ✅ **86/304** gate 100%（Panda udeal + Python，2026-08-10） |
| **yumail 老号**（nancybaker / aharris / aclark） | ✅ `grok_pure_http_client --gate` | ✅ 文档记 `aharris` + udeal 3 轮 chat/OCR **100%**（须 `GROK_UPSTREAM_PROXY`） |
| **yumail 新号** kevin（8/8 注册 id=1701） | ❌ node/简化探测 403 | ❌ 403（含今日全池扫） |

### 3.2 其他链路（辅助判断，非「纯 HTTP chat」）

| 能力 | grok2api 老池 | yumail 池 | 环境 |
|------|---------------|-----------|------|
| GET `/rest/app-chat/conversations` + 签名 | ✅ ~670/672 | ✅ | Panda |
| POST `load-responses`（读历史）node 签 | ✅ kevin 亦 200 | ✅ | 本机 |
| WS `mgw` UI 发消息收 PONG | 未系统验证 | ✅ nancybaker | 本机 Playwright |
| 网页 SSO → Chrome 对话（WS mgw） | ✅ 86/304/92（2026-08-10 本机） | 未测 | 本机 |

---

## 4. 时间线（关键实测）

| 日期 | 事件 | 结论 |
|------|------|------|
| 2026-08-07 | Panda SQLite 686 账号 + 海外代理 POST 全扫 | **686×403 + 1×401**（`39h`） |
| 2026-08-08 上午 | Panda `grok_pure_http_chat_probe` 697 账号 | **0 POST 200**；GET 670 成功 |
| 2026-08-08 下午 | 本机 `grok_pure_http_client` + nancybaker keys | **POST chat/OCR/多轮 200** |
| 2026-08-08 | Panda udeal + aharris 可靠性 3 轮 | chat/OCR **100%**（`39j` §11） |
| 2026-08-08 | 新注册 kevin → grok2api id=1701 | 注册 ✅；POST **403**（与老池同类） |
| 2026-08-08 | kevin `grok_ab_gate_20260808.json` | node 签 GET 200；**POST /new 403**（浏览器签亦 403） |
| 2026-08-10 | 老池 **86/304** + `grok-pure-http --gate`（session keys + Python statsig） | **POST chat/OCR/多轮 200**；92 单号 403 |
| 2026-08-10 | Rust `SessionKeyStore` + `GROK_PURE_HTTP_KEYS_DIR` 接线 | 生产按 `account_{id}.json` 自动 SessionSigner |

---

## 5. 生产含义（TNexus / grok2api-rs）

1. **PG 672 号不能靠裸 sso + node 签扛 chat**——需 per-account `pure_http_keys`（meta48 + fingerprint + cf cookie）。  
2. **方案 A（进行中）**：老池活号批量 extract keys → `GROK_PURE_HTTP_KEYS_DIR` → `grok2api-rs` `SessionSigner`；已验证 86/304 gate。  
3. **备选路径**（未接线生产）：yumail 灌 PG、WS mgw、新号 gate 筛活。  
4. **Rust 生产路径**已支持 `SessionKeyStore`；无 keys 时仍回退 `NativeSigner`（与 node probe 同类，易 403）。

---

## 6. 脚本索引

| 脚本 | 用途 |
|------|------|
| `scripts/extract_old_pool_session_keys.py` | 老池 SSO → `account_{id}.json` |
| `scripts/grok_old_pool_panda_gate.sh` | Panda 老池 gate（`/opt/tnexus/pure_http_keys`） |
| `scripts/grok_panda_chat_e2e.sh` | Panda curl /v1/chat/completions 探测 |
| `scripts/grok_local_e2e_chat.sh` | 本机 grok2api-rs + keys 端到端 |
| `scripts/grok_pure_http_client.py` | yumail 参考实现；`--gate` |
| `scripts/grok_pure_http_chat_probe.py` | Panda SQLite 批量（node 签） |
| `scripts/grok_pg_chat_probe.py` | Panda PG 批量（2026-08-10 全扫） |
| `scripts/grok_old_pool_chrome_probe.py` | 老池 SSO → 本机 Chrome 网页对话 |
| `scripts/fetch_grok2api_sso.py` | 从 SQLite 解密 SSO（Panda 上跑） |
| `AutoRegister/.../grok_ws_chat_probe.py` | WS mgw `ui` 模式 |

---

## 7. 变更记录

| 日期 | 变更 |
|------|------|
| 2026-08-10 | 初版：号池/工具/环境矩阵、672 全扫、与 39j 分工 |
| 2026-08-10 | Panda udeal gate：老池 86/304 pure_http keys 全绿（与 yumail 同级） |
| 2026-08-10 | Panda curl /v1/chat/completions 当前 GHCR 镜像 → 502 chat 403，待 SessionKeyStore 镜像 |
