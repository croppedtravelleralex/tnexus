# 00 — PROTOCOL_CONTRACT

最后更新：2026-07-23  
版本：`PROTOCOL_CONTRACT_VERSION=1`

Breaking 变更必须：本文件 bump + `gptimage` / 本仓 CHANGELOG 双边记录。

## 1. 错误归因

| class | 含义 | MVP 验收 |
|-------|------|----------|
| `upstream` | CF/429/上游拒/内容政策/账号被上游拒绝 | 允许；对照不得显著恶化 |
| `client` | 缺参/坏请求 | 稳定 4xx |
| `self` | 协议错、无 Bearer、假 ready、空 data、超时自伤、槽泄漏 | **必须 = 0** |
| `gate` | pause / admission / duplicate-prompt 窗 | 记门禁，非 self |

映射现网：`llm_ops.outcome_code` / 字符串前缀 → 上表。Rust 实现：`crates/protocol/src/error_class.rs` 的 `classify_fault()`。

## 1b. 鉴权（gateway face）

| 路径 | 角色 |
|------|------|
| `/api/auth/login` | 公开 |
| `/api/auth/register` | admin（或 `AUTH_ALLOW_PUBLIC_REGISTER=1`） |
| `/api/auth/me`、`/api/auth/logout` | 已登录 |
| `/api/backend/capabilities` | 公开 |
| `/v1/chat/*`、`/v1/models` | member + admin |
| `/v1/accounts/candidates`、`/v1/quota*`、`/api/admin/*` | admin only |
| `/v1/images/*` | member + admin；**默认 `IMAGE_ENABLED=0` → 501 deferred** |

详见 `docs/21-auth-and-ui.md`。

## 1c. Phase B 图像契约（运行时后置）

- **契约层（✅）**：`fixtures/protocol/` 全量；`protocol::image_contract`（prepare/start/edits/estuary 头校验）
- **运行时（⏸️）**：生图/edits/estuary 下载待后端管线接入；开启 `IMAGE_ENABLED=1` 后验收
- estuary：**必须** API session + `Authorization: Bearer`；resource PUT **禁止** Bearer

## 2. URL → Session（硬表）

| URL / 用途 | Session | Authorization |
|------------|---------|---------------|
| `/backend-api/conversation`、`f/conversation*`、chat requirements、bootstrap | API | Bearer + OAI 头 |
| estuary / 文件流下载 | **API（主 session）** | **必须 Bearer** |
| S3 / upload PUT | resource | **禁止** Bearer/OAI |
| 负例 | estuary 走 resource | **必须失败** |

## 3. SSE / 生图语义

- ready 谓词：SSE payload **含 `conversation_id`**
- **禁止** `post_ready=15s` 墙钟假 ready
- `skipped_mainline`：工具调用载荷，**继续 poll**，禁止当换号失败
- CF bootstrap soft-fail：HTML challenge → 默认 PoW 继续（对齐 Python）
- Arkose `required`：显式失败，不伪装成功、不偷偷换号

## 4. 文本

- 默认 Temporary Chat：`history_and_training_disabled=true`，不传生图 cid
- 养号：`text_chat_persist_history` / 账号开关 → false + 独立 text cid 续聊
- tz / `OAI-Language` 跟 sticky egress（生产习惯 SG）

## 5. Config 子集（Rust 可读）

`text_chat_persist_history`、`text_chat_reuse_conversation`、`proactive_refresh.timezone` / `timezone_from_egress`、`image_pre_conversation_*`、`image_poll_*`、`image_settle_*`、`image_generation_poll_timeout_secs`、`image_edit_poll_*`、`image_multi_reference_*`、`image_account_concurrency`、`image_global_concurrency`、`image_generation_paused`、`proxy_runtime`（**clearance 保持关，不实现 FlareSolverr**）。

`OAI-Client-Version`：代码常量，双边同步策略写 CHANGELOG。

## 6. MVP Pin（不经池）

L1/L2：固定 `access_token` + sticky proxy 注入；**不调用** `get_available_access_token`。  
官方脚本约定（未来）：`scripts/protocol_l1_pin_smoke.py`；禁止 `scripts/archive/tmp-*`。

## 7. Helper

- 本机 HTTP（例 `127.0.0.1:19001`）流式透传 SSE
- 归属本仓最小 Python 侧车，**不** import 整棵 gptimage
- 未证明 rustls≡impersonate 前禁止直连上游

## 8. Fixtures

目录：`fixtures/protocol/`（**已齐**，见该目录 README）。  
Rust 差分：`cargo test -p protocol --test fixtures`（8 项）。  
禁止夹具含真实 token。

## 9. 仓边界

- 未授权不写生产 `accounts.db` / `image_tasks.db`
- 测试环境独立端口；公网仍指 Python 直至 R2
