# 39j — Grok 纯 HTTP 逆向抓包与实施指南

> 状态：**2026-08-10**。本文档汇总逆向结论，供下次续攻时直接引用。  
> **号池/环境验收矩阵**（必读）：[39k-pure-http-verification-matrix.md](39k-pure-http-verification-matrix.md)  
> 配套实现：`scripts/grok_pure_http_client.py`（Python 探针）、`crates/grok-pure-http`（Rust 探针）、`crates/grok-provider-web/src/direct.rs`（生产直连）。

---

## 1. 读序（下次逆向从这里开始）

| 顺序 | 资产 | 说明 |
|------|------|------|
| 1 | [39k-pure-http-verification-matrix.md](39k-pure-http-verification-matrix.md) | **号池 × 本机/Panda × 工具** 验收矩阵 |
| 2 | 本文档 | 协议总览 + 踩坑 |
| 3 | [39h-direct-signer.md](39h-direct-signer.md) | x-statsig-id 签名器攻坚 |
| 4 | `scripts/grok_pure_http_client.py` | 可跑通的 Python 参考实现（**yumail 池**） |
| 5 | `crates/grok-pure-http/src/main.rs` | Rust gate 对齐 |
| 6 | 抓包 JSON（见 §6） | 原始证据 |

---

## 2. 网页真实对话链路（2026-08-08 实测）

### 2.1 发消息：**WebSocket，不是** `conversations/new`

| 观察 | 结论 |
|------|------|
| 浏览器 DevTools 抓包 | `POST /rest/app-chat/conversations/new` **0 次** |
| 实际通道 | `wss://grok.com/ws/mgw/?uid=...` |
| 帧序列 | `session.create` → `conversation.item.create` → `response.create`（含 `castle_request_token`）→ `response.chunk` |

**纯 HTTP 路径**（`conversations/new` + `/responses`）可独立打通，但与 UI 默认链路不同；UI 依赖 Castle token（模块 `6037942`），无浏览器时 WS 手写帧会卡在 `response.create`。

### 2.2 纯 HTTP 端点（按号池区分）

| 方法 | 路径 | yumail 池 + `pure_http_client` | grok2api PG 老池 + node/pg probe |
|------|------|-------------------------------|----------------------------------|
| GET | `/rest/app-chat/conversations` | 200 | 200（~670/672） |
| POST | `/rest/app-chat/upload-file` | 200 | 未全量验证 |
| POST | `/rest/app-chat/conversations/new` | **200**（本机 gate；Panda udeal aharris 100%） | **403 anti-bot**（672/672，2026-08-10） |
| POST | `/rest/app-chat/conversations/{id}/responses` | 200 多轮 | 未成功 |

详见 [39k](39k-pure-http-verification-matrix.md)。

---

## 3. 认证与签名

### 3.1 请求头（必需）

```
Cookie: sso=<jwt>; sso-rw=<jwt>
x-statsig-id: <signed>
Content-Type: application/json
Origin: https://grok.com
Referer: https://grok.com/
User-Agent: Chrome/146 系
```

可选但推荐：`x-xai-request-id`（UUID）、完整 cookie 含 `cf_clearance`（Playwright 一次提取）。

### 3.2 x-statsig-id 两条可用路径

| 路径 | 实现 | 状态 |
|------|------|------|
| **Python statsig** | `meta48`(48B) + `fingerprint` + `generate_statsig(method,path)` | ✅ 本机 gate 通过 |
| **Node 1645e3** | `grok_sign_standalone.js` + `grok_sign_module_1645e3.js` | ✅ load-responses 等动态 path |
| Remote wodf.de | `POST https://grok.wodf.de/sign` | ❌ CF challenge，已死 |
| rquickjs LocalSigner | `crates/grok-signer` bundle | ⚠️ 真 bundle 为 Node 专用，Rust 引擎未接真模块 |

**Session keys 提取**（Playwright 一次，存 `pure_http_keys/{email}.json`）：

- `meta_b64`：48 字节 meta
- `fingerprint`：digest hook 捕获的 `obfiowerehiring` 后缀
- `trailer_hex`：默认 `03`
- `cookie`：含 `cf_clearance` 时成功率更高

Rust 对齐：`grok_signer::statsig_obfiowerehiring` + `SessionSigner`（`grok-provider-web`）。

---

## 4. SSE 响应格式（关键踩坑）

grok chat **不是**标准 `data: {...}` SSE；多为**每行一条裸 JSON**。

### 4.1 首轮 `conversations/new`

```json
{"result":{"conversation":{"conversationId":"..."},"response":{"modelResponse":{"message":"PONG","responseId":"..."}}}}
```

或 token 流：`result.response.token` + `messageTag: final`。

### 4.2 多轮 `/responses`（扁平结构）

```json
{"result":{"userResponse":{"responseId":"...","message":"..."}}}
{"result":{"token":"P","messageTag":"final","responseId":"..."}}
{"result":{"modelResponse":{"responseId":"...","message":"PONG2","parentResponseId":"..."}}}
```

**解析要点**：

- `responseId` 可能在 `result` 顶层，不一定在 `result.response` 下
- followup 响应**通常不含** `conversationId` → 多轮应用**首轮**的 `conversation_id`
- 完整正文优先取 `modelResponse.message`，其次拼接 `token`

---

## 5. 图片上传与 OCR 默认链路

### 5.1 上传

```http
POST /rest/app-chat/upload-file
{"fileName":"x.png","fileMimeType":"image/png","content":"<base64>"}
→ {"fileMetadataId":"uuid", "fileUri":"users/.../content"}
```

最小尺寸：1×1 PNG 会 **400**；用真实图片（如用户探针 PNG）可通过。

### 5.2 OCR chat payload

```json
{
  "model": "grok-chat-fast",
  "enableImageGeneration": false,
  "enableImageStreaming": false,
  "fileAttachments": ["<fileMetadataId>"],
  "messages": [
    {"role":"system","content":"提取图中全部可见文字…"},
    {"role":"user","content":"提取图中全部可见文字，若无文字则描述画面。"}
  ]
}
```

TNexus 默认：`grok-vision-ocr` → 上游 `grok-chat-fast`（见 `grok-domain::provider`）。  
Gate 默认探针图：`C:\Users\Lenovo\Downloads\image-1785287126849-...png`（`--image` 可覆盖）。

---

## 6. 抓包资产索引

路径根：`D:\SelfMadeTool\AutoRegister\grok\grok_bytao\grok_bytao\reports\`

| 文件 | 内容 |
|------|------|
| `grok_manual_capture.json` | HTTP 请求/响应 |
| `grok_ws_capture.json` | WS mgw 协议（1096 帧） |
| `grok_manual_capture_ab.json` | Node vs browser 签名 A/B |
| `pure_http_keys/*.json` | session 签名材料 + cookie |
| `gate_*.json` | Python/Rust gate 结果 |

分析脚本：`scripts/_analyze_ws_capture.py`、`scripts/grok_browser_capture_live.py`、`scripts/grok_replay_capture.py`。

---

## 7. 代理拓扑（Panda 四路对比）

| 模式 | 环境变量 | 用途 |
|------|----------|------|
| **直连** | 不设 `GROK_UPSTREAM_PROXY`；`GROK_LOCAL_PROXY` 可选 | meta/签名走本地 Clash |
| **udeal** | `GROK_UPSTREAM_PROXY=http://user:pass@70.39.164.200:30000` 或 SQLite `egress_nodes.id=110` | 海外固定节点 |
| **webshare 机房** | `GROK2API_PROXY_FILE` / `GROK_UPSTREAM_PROXY` 填 DC 列表 | 账号出口隔离 |
| **webshare 住宅** | 同上，住宅节点文件 | 对齐生产 webshare 池 |

脚本：`scripts/grok_panda_proxy_matrix.sh`（Rust `grok-pure-http --gate` 四路跑分）。

**红线**：Panda 上禁止 `docker build` / `cargo build`；镜像走 GHCR + `deploy.sh`。

---

## 8. 命令速查

```bash
# Python gate（本机，默认 OCR 图）
python scripts/grok_pure_http_client.py \
  --email nancybaker2jyy@yumail.co --gate --signer auto

# Rust gate（本机）
GROK_LOCAL_PROXY=http://127.0.0.1:7897 \
  cargo run -p grok-pure-http -- \
  --keys /path/to/pure_http_keys/email_at_domain.json --gate

# Panda 四路代理矩阵
bash scripts/grok_panda_proxy_matrix.sh

# yumail 批量 gate + HTTP 可靠性（udeal）
py -3.12 scripts/grok_batch_yumail_gate.py --quota-scan-json reports/pure_http_keys/quota_scan_*.json
py -3.12 scripts/grok_http_reliability_probe.py --email aclarkdc8c@yumail.co --rounds 5
bash scripts/grok_panda_udeal_batch.sh
```

---

## 9. 未攻克 / 后续

| 项 | 说明 |
|----|------|
| WS 无 UI 纯 Python | `castle_request_token`（Castle SDK `6037942`） |
| TLS 指纹 | Python 用 `curl_cffi chrome131`；Rust `reqwest` 默认 TLS |
| 真流式 SSE | gateway 当前缓冲全 body 后解析 |
| UI 默认对话 | 走 `wss://grok.com/ws/mgw/`，非纯 HTTP |

**Free 号池**：无 Pro imagine；**Lite 生图**走 `conversations/new` + `enableImageGeneration`，图片在 **HTTP SSE** `cardAttachment.image_chunk` 返回，**不经过** `imagine/listen` WS。

---

## 12. 脚本索引（2026-08-08）

| 脚本 | 用途 |
|------|------|
| `grok_pure_http_client.py` | extract / gate / 单轮 chat |
| `grok_account_quota_scan.py` | 浏览器拦截 `rate-limits`（勿手动 fetch，会 404） |
| `grok_quota_probe.py` | 纯 HTTP 读额度（需 keys） |
| `grok_batch_yumail_gate.py` | yumail 有额度账号批量 extract + gate |
| `grok_http_reliability_probe.py` | 多轮 chat/OCR/额度/Lite 生图成功率 |
| `grok_panda_udeal_reliability.sh` | Panda + udeal 多轮 chat/OCR/额度/Lite 生图 |
| `grok_panda_udeal_gate.sh` | Panda + udeal 批量 gate |
| `grok_get_udeal_proxy.py` | 从 Panda 解密 udeal 出口 URL |
| `grok_playwright_common.py` | Playwright 共享常量 |
| `grok_webshare_browser_probe.py` | webshare Playwright 探测（本机直连多 RESET） |

---

## 13. Lite 生图（Free / HTTP）

```json
{
  "message": "Drawing: a red apple",
  "enableImageGeneration": true,
  "enableImageStreaming": true,
  "imageGenerationCount": 2,
  "modeId": "fast"
}
```

响应 SSE 含 `image_chunk.imageUrl`（`users/.../generated/.../image.jpg`），`imageModel`: `imagine_x_1`。

---

## 14. Panda udeal 批量测试

```bash
# 本机先扫额度
py -3.12 scripts/grok_account_quota_scan.py --domain yumail.co --limit 25 --min-remaining 1

# 本机批量 gate（经 udeal，需 export GROK_UPSTREAM_PROXY）
py -3.12 scripts/grok_batch_yumail_gate.py --quota-scan-json path/to/quota_scan_*.json --limit 10

# Panda 上 udeal 批量（禁止 build）
LIMIT=5 ROUNDS=3 EMAIL=aclarkdc8c@yumail.co bash scripts/grok_panda_udeal_batch.sh
```

**udeal 解密**：`egress_nodes.id=110`，见 `scripts/grok_panda_proxy_matrix_run.sh`。

---

## 10. 变更记录

| 日期 | 变更 |
|------|------|
| 2026-08-08 | 初版：WS 抓包结论、纯 HTTP 全链路、SSE 扁平解析、OCR 默认图、Rust `grok-pure-http` |
| 2026-08-08 | 补充额度 API、yumail 批量扫描、Lite HTTP 生图、Panda udeal 批量脚本 |
| 2026-08-08 | 补充 `POST /rest/rate-limits` 额度 API；udeal OCR 失败根因 429（非上传） |
| 2026-08-10 | 新增 [39k](39k-pure-http-verification-matrix.md)；PG 672 全扫 0 活号；区分两套号池/探测工具 |

## 11. 额度 API（可读）

`POST /rest/rate-limits`，body `{}`（需有效 x-statsig-id + cookie）：

```json
{
  "windowSizeSeconds": 86400,
  "remainingQueries": 30,
  "totalQueries": 30,
  "waitTimeSeconds": 0,
  "lowEffortRateLimits": null,
  "highEffortRateLimits": null
}
```

- **对话额度**：`remainingQueries` / `totalQueries`（24h 窗口）
- **上传/生图独立额度**：抓包中未见单独 upload 计数；上传失败看 HTTP 400
- **Lite 生图**：扣 fast 对话额度；HTTP SSE 返回 `imageUrl`，非 Pro WS
- 账号耗尽时：`remainingQueries: 0`，对话/OCR 返回 **429**
- 探测脚本：
  - `scripts/grok_quota_probe.py` — 纯 HTTP（需 keys）
  - `scripts/grok_account_quota_scan.py` — 浏览器批量（拦截页面自动 `rate-limits`）

**额度扫描注意**：页面加载时 grok 会**自动** POST `rate-limits` 并返回 200；在 `page.evaluate` 里手动 `fetch('/rest/rate-limits')` 会得到 **404 Model not found**。

**2026-08-08 yumail 池（前 25）**：23/25 `remainingQueries=30`；`nancybaker2jyy@yumail.co` 已耗尽（0）。

**2026-08-08 Panda+udeal 可靠性（`aharrisd00r@yumail.co`，3 轮）**：

| 项 | 成功率 |
|----|--------|
| rate-limits | 100% |
| chat | 100% |
| OCR | 100% |
| Lite 生图 | 67%（2/3） |

本机直连 udeal 会 `Connection reset`；须在 **Panda 上**设 `GROK_UPSTREAM_PROXY=udeal`。

