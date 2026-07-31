# 24 — 生产有、Rust 没有：能力 gap 全量盘点（2026-07-26）

对照基准：`../gptimage-panda`（panda `:8012` 生产快照）
盘点方式：4 个并行只读 agent（路由面 / 数据面 / 调度面 / 运维面）
关联：[22 审计](22-audit-2026-07-26.md) · [23 进度量化](23-rewrite-progress.md)

---

## 0. 总账

| 维度 | 生产 | Rust 网关 | 覆盖 |
|------|------|----------|------|
| HTTP 端点 | 129 | 生产态 7 / 本地工作树 15 | 已实现 0 / 部分 10 / 缺失 65 / 永久非目标 54 |
| 上游数据面能力 | 42 条 | 1 条已接线（`classify_fault`） | **2%** |
| SSE 事件类型 | 20 种 | 0 种 | **0%** |
| 重试 / 退避策略 | 13 条 | 0 条 | **0%** |
| 调度面代码 | ~8,000 行 | 树内 Rust 1,107 + 网关 ~30 行 | 见 §3 |
| 配置键 | 71（可热更新） | 18 个环境变量（需重启） | 语义重合 1 项 |
| 持久化实体 | 20+ | 1（`users` 表） | — |
| 后台循环 | 9 | 0 | — |

> 「生产态 7」是 panda `:8013` 实测值，「工作树 15」是本地未提交代码的路由数 —— 二者差 8 条路由滞留在未跟踪工作树，按 git 链路当前到不了 panda。实测明细见 [25-panda-vs-rust-20260726.md](25-panda-vs-rust-20260726.md) §1.4。

**一句话**：Rust 网关目前是「HTTP 外壳 + 只读账号缓存 + JWT 鉴权」，数据面 100% 仍是 Python，且不是移植副本 —— 是通过 `sys.path` 反向挂载的**同一份生产代码**。Rust 二进制**不能脱离 gptimage Python 树独立部署**。

---

## 1. 路由面 gap

### 1.1 OpenAI 兼容面（`api/ai.py` 11 个）

| 方法+路径 | 职责 | Rust 状态 | 优先级 |
|-----------|------|----------|--------|
| POST `/v1/images/edits` | 图生图，multipart + asset_ids + mask | **缺失** —— 恒返 501 `image_edits_deferred` | P0 |
| POST `/v1/images/generations` | 文生图，同步/流式/异步隧道/任务轮询 | 部分，14 项字段差异见 §1.2 | P0 |
| POST `/v1/chat/completions` | 对话补全，含对话生图 | 部分 —— 只取最后一条 user 文本，丢弃多轮/system/多模态；usage 恒 0；SSE 纯字节透传不解析 | P0 |
| GET `/v1/models` | 静态 7 模型 + 按号池动态派生 | 部分 —— 硬编码 2 个，无动态派生，缺 created/permission/root/parent | P0 |
| POST `/v1/responses` | OpenAI Responses API（tools / image_generation tool） | 缺失 | P1 |
| POST `/v1/messages` | Anthropic Messages 兼容面 | 缺失 | P1 |
| POST `/v1/search` | 联网搜索 + 多图附件 | 缺失 | P2 |
| GET `/v1/editable-file-tasks`<br>POST `/v1/ppt/generations`<br>POST `/v1/psd/generations`<br>GET `/files/{path}` | PPT/PSD 生成与下载 | 缺失 | P2 |

### 1.2 `/v1/images/generations` 逐项差异

| 维度 | 生产 | Rust | 后果 |
|------|------|------|------|
| `n` | `1..4` | `!= 1` 直接 400 | 多图请求全废 |
| `quality` | `"auto"` 透传 | **结构体无此字段**，静默丢弃 | 质量档位失效 |
| `size` | 默认 `None`，后端决策 | 写死 `"1024x1024"` | 默认行为漂移 |
| `response_format` | 支持 `url` | 接受但恒返 `b64_json` | `url` 客户端拿不到图 |
| `stream` | 支持 | 无此字段 | 流式生图不可用 |
| `history_disabled` | 默认 `True` 透传 | 无 | 上游会话留痕 |
| `panda_async` / `panda_task_id` | 异步入队与轮询 | 无 | 异步隧道断 |
| prompt 隧道 | `panda-async:` / `panda-status ` 前缀 | 无 | 旁路协议断 |
| 未知字段 | `extra="allow"` 全量透传 | serde 默认静默丢弃 | 扩展参数丢失 |
| 内容过滤 | `filter_or_log` → `check_request` | 无 | 无前置拦截 |
| 暂停闸 | 503 + `Retry-After: 300` | 无 | 保号闸失效 |
| 准入 429 | `image_service_busy` + `estimated_wait_secs` + `Retry-After` | 仅 semaphore 硬阻塞 | **无背压信号** |
| 重复 prompt | 429 `duplicate_prompt` | 无 | 去重失效 |
| 超时 | 504 + `task_id` 可续拉 | 无 | 超时即丢单 |

### 1.3 两个结构性不兼容

**① 鉴权语义互斥**
生产 `api/support.py:30` 用 `auth_service.authenticate(token)` 校验 **API key**；Rust `auth_routes.rs:206` 用 `verify_token()` 校验 **JWT**。两侧都读 `Authorization: Bearer`，但 token 语义互斥 —— **持 API key 的存量 OpenAI 客户端换到 Rust 端口必然 401**。bringup 脚本靠 `AUTH_DISABLE=1` 绕过了这个问题，掩盖至今。生产侧管理 API key 的 4 个端点在 Rust 无对应实现。

**② `error.type` 恒为 `gateway_error`**
生产按 OpenAI 语义分档（`authentication_error` / `rate_limit_error` / `invalid_request_error` / `server_error`），Rust `protocol/src/lib.rs:97` 恒填 `"gateway_error"` 并多一个自定义 `fault` 字段。OpenAI 官方 SDK 对 `rate_limit_error` 有特化重试行为 —— 恒定 type 会让客户端自动重试全部失效；且 429 不带 `Retry-After`，重试策略退化成打满。

### 1.4 其余分组（缺失数）

| 分组 | 端点数 | 缺失 | 最要紧的 |
|------|--------|------|---------|
| 号池管理 | 26 | 22 | `POST /api/accounts`（写入）、`/refresh`（token 刷新）、`/scheduling`（进出调度）、`/schedulable-breakdown`（不可调度归因） |
| 系统运维 | 29 | 27 | `/health` 运维载荷、`/api/ops/image-pipeline/snapshot`、`/version`、`GET+POST /api/settings` |
| 图片任务/资产 | 9 | 9 | `POST /api/image-tasks/generations`（异步队列入口）、`/api/image-assets/references`（参考图上传） |
| **永久非目标** | 54 | — | 注册机 7 / Outlook OTP 4 / 维护环 3 / Panda sync 4 / sub2api 8 / CPA 7 / risk 面板 4 / 养号 8 / CF 出口 3 / backup 6 |

> CPA 号池那 7 个端点未被 `plan.md` 明文点名，此处按「号源外置」同类归入非目标，**建议确认**。

---

## 2. 数据面 gap

### 2.1 能力清单（42 条，Rust 已接线仅 1 条）

| 能力 | 生产位置 | 行数 | Rust | 难度 |
|------|---------|------|------|------|
| TLS 指纹伪装（curl_cffi impersonate / 按账号 fp 固化） | `openai_backend_api.py:494-668,4320-4360` | ~240 | 缺失 | **高** |
| PoW 脚本源抓取（首页 HTML → 600s 缓存） | `:4487-4561` | 75 | 缺失 | **高** |
| PoW 求解（seed/difficulty → proof_token） | `:4620-4695,973-1000` | 105 | 缺失 | **高** |
| Turnstile 求解（dx → 本地 JS VM） | `:4620-4695` | 25 | 缺失 | **高** |
| Sentinel prepare/finalize 双段票据 | `:4563-4695` | 130 | 缺失 | 中 |
| SSE 队列化读取 + 早退 | `:93-355` | 260 | 缺失 | 中 |
| SSE 非阻塞 abort（daemon 线程避 curl 清理阻塞） | `:58-90` | 32 | 缺失 | 中 |
| 文件上传三段（files → Azure Blob PUT → uploaded） | `:1549-1633` | 85 | 缺失 | 中 |
| 按请求 proxy 绑定 + 双 session 隔离 | `:544-622` | 80 | 缺失 | 中 |
| bootstrap 软失败降级 | `:4462-4496` | 35 | 缺失 | 中 |
| SSE 行解析 → 事件包装 | `conversation.py:930-976` | 46 | 缺失 | 低 |
| Delta patch 应用（append/replace/patch） | `:773-805` | 33 | 缺失 | 低 |
| conversation 状态机 | `:601-610,868-912` | 55 | 缺失 | 低 |
| 图片资产指针抽取（file-service:// / sediment://） | `:814-854` | 40 | 缺失 | 低 |
| 文本净化（剥私有区标记/泄漏 tool call/历史） | `:658-698,743-760` | 55 | 缺失 | 低 |
| moderation 拦截判定 | `:903-906,1362-1367` | 45 | 缺失 | 低 |
| 请求体构造（prepare/start/start+refs/chat） | `chatgpt_web_request.py:213-439` | 226 | **仅 fixture 未接线**，17 处差异 | 低 |
| `client_contextual_info` 三形态（SHA256 seeded 抖动） | `:63-110` | 48 | 缺失 | 低 |
| `@Create image` + U+00A0 mention + custom_symbol_offsets | `:112-120` | 10 | 缺失 | 低 |
| 时区偏移动态计算（ZoneInfo，DST 感知） | `:32-44` | 13 | 缺失（硬编码 -480） | 低 |
| Header 构造全族（OAI-Device-Id / Sec-CH-UA 全套） | `openai_backend_api.py:624-771,1152-1231` | 220 | 缺失 | 低 |
| conversation/tasks 轮询解析 file_ids/sediment_ids | `:3808-3996` | 190 | 缺失 | 低 |
| 图片轮询主循环（多层退避 + 429/CF 熔断） | `:3415-3625` | 210 | 缺失 | 低 |
| 轮询预算三约束 | `image_poll_budget.py:45-100` | 112 | 缺失 | 低 |
| 死锁守卫（CPU p95 熔断 90%/60s trip） | `image_deadlock_guard_service.py:37-105` | 109 | 缺失 | 低 |
| 回图窗口（BoundedSemaphore 背压 180s） | `image_return_window_service.py:18-59` | 62 | 缺失 | 低 |
| 就绪缓冲背压（512MB/32 项，滞回恢复） | `image_pipeline/ready_buffer.py:15-88` | 92 | 缺失 | 低 |
| 同步适配（60-900s 等待，0.2-10s 轮询） | `image_sync_adapter.py:21-147` | 148 | 缺失 | 低 |
| Estuary 下载 | `openai_backend_api.py:3998-4028` | 30 | **仅头构造+校验**，无下载 | 低 |
| CF/403 边缘拦截识别 | `:788-813,4292-4297` | 40 | 仅字符串匹配 | 低 |
| 瞬态传输错误识别（curl 35/56/TLS/reset） | `:4300-4318` | 19 | 缺失 | 低 |
| 401 → InvalidAccessToken 快失败 | `:357-366,494-527` | 55 | 缺失 | 低 |
| Retry-After 头感知的 429 退避 | `:3501-3525` | 25 | 缺失 | 低 |
| 配额解析（limits_progress） | `:774-787,909-948` | 55 | **已接线**（经 bridge 代理，非 Rust 原生） | 低 |
| 错误信封成形（5 类 type/code 映射） | `protocol/error_response.py:39-124` | 86 | 仅 4 值 fault | 低 |

**难度分布**：高 5 项（~470 行）· 中 8 项 · 低 ~29 项（~2,400 行）。

高难度那 5 项**互相耦合**：PoW 脚本从首页 HTML 抓，而首页请求本身需要正确的 TLS 指纹才不被 CF 拦。这决定了它们要么一起做，要么一起留在 Python sidecar。

### 2.2 协议字段差异：实际 17 处，不是 7 处

[22 审计](22-audit-2026-07-26.md) §3 首轮抽查只列了 7 处（原文表述「≥7 处」，现已同步修正为 17），本轮逐字段复核得 **17 处**。修正如下：

**prepare body**（`image_contract.rs:31-56` vs `chatgpt_web_request.py:309-346`）

| # | 字段 | 生产 | Rust | 性质 |
|---|------|------|------|------|
| 1 | `partial_query.id` | `new_uuid()` | `"fixture-partial-query-id"` 字面量 | **发出去就是固定 ID，会被上游关联** |
| 2 | `client_contextual_info.has_web_push_capabilities` | `True` | 缺失 | 缺字段 |
| 3 | `client_contextual_info.web_push_notification_permission` | `"default"` | 缺失 | 缺字段 |
| 4 | `client_contextual_info.app_version` | **不存在** | `"fixture"` | 多出的伪字段 |
| 5 | `_fixture_prompt` | **不存在** | `prompt` | 多出的伪字段，**prompt 重复外泄** |
| 6 | `timezone_offset_min` | ZoneInfo 动态、DST 感知 | 硬编码 `-480` | 夏令时/换时区即错 |
| 7 | SPA 分支 | 5 处字段随 `image_spa_tool_path_enabled` 翻转 | 无分支，锁死 non-SPA | 整条 SPA 路由不可达 |

**start body（无 refs）**（`image_contract.rs:59-82` vs `:349-439`）

| # | 字段 | 生产 | Rust | 性质 |
|---|------|------|------|------|
| 8 | `messages[0].id` | `new_uuid()` | `"fixture-user-message-id"` | 固定 ID |
| 9 | `content.parts[0]` | `"@Create image" + U+00A0 + prompt` | 裸 `prompt` | **缺 mention 前缀，turn 不进 picture_v2** |
| 10 | `custom_symbol_offsets` | 真实计算值 | 硬编码 `[]` | 源码注释：空 offsets 与无 metadata **不等价**，上游会只发 code call 不追加图片 |
| 11 | `messages[0].create_time` | `time.time()`（non-SPA 必带） | 缺失 | 缺字段 |
| 12 | `client_contextual_info` | 10 键，seed + jitter | **整个键缺失** | 最大单点缺口 |
| 13 | `paragen_cot_summary_display_override` | `"allow"` | 缺失 | 缺字段 |
| 14 | `force_parallel_switch` | `"auto"` | 缺失 | 缺字段 |
| 15 | `timezone_offset_min` | 动态 | 硬编码 | 同 #6 |

**start body（带 refs）**：#16 `image_asset_pointer` 对象、#17 `metadata.attachments` —— **完全一致**，是唯一对齐的部分。其余 #8-#15 同样存在。

**文本体**（`protocol/src/lib.rs:139-158` vs `:213-264`）差得最远，像是照另一份更老的协议写的：缺 11 键（`client_prepare_state`、`enable_message_followups`、`supports_buffering`、`supported_encodings`、`system_hints`、`timezone`、`paragen_cot_summary_display_override`、`force_parallel_switch`、`client_contextual_info`、条件 `conversation_id`、条件 `thinking_effort`），多 3 个生产没有的键（`force_paragen`、`force_rate_limit`、`websocket_request_id`），且 `parent_message_id` 用 `uuid4()` 而非 `"client-created-root"`。

### 2.3 SSE 事件覆盖：0 / 20

Rust 从未见过上游 SSE —— `helper_client::run_text_stream` 返回裸 `reqwest::Response`，bridge 已经把 ChatGPT-web SSE 转成 OpenAI chunk 了，Rust 只做字节转发。

未处理的 20 种：`[DONE]` · `conversation.done/delta/event/raw` · `moderation` · `server_ste_metadata` · `image_generation_call` · patch op `patch/append/replace` · patch 路径 `/message/content/parts/0` · author.role `assistant/tool/user` · content_type `multimodal_text/image_asset_pointer/text` · `async_task_type == "image_gen"` · 资产前缀 `file-service:// / sediment:// / file_00000000[a-f0-9]{24}`

两条最容易漏的避坑经验：

- `REAL_IMAGE_FILE_ID_RE`（`conversation.py:818`）专门用 `file_00000000` + 24 hex 过滤掉 `file_upload_business_upsell` 这类假 ID
- 图片资产准入不只看 `content_type`，还需满足「完整 tool 消息」或「先前 `tool_invoked` 已置位且非 user 消息」或「patch 事件引用资产」三选一（`:882-884`）

### 2.4 重试 / 退避：生产 13 条，Rust 全树 0 处

`grep -c 'retry\|backoff\|sleep' crates/` → **0**。Rust 只有两个裸超时（默认 120s、图片 180s）。

| 策略 | 参数 |
|------|------|
| 认证探针重试 | 3 次，`0.35*attempt`，403/429/502/503/520-524，401 不重试 |
| 图片上传重试 | 5 次，`3.0*2^(n-1)` 封顶 30s |
| chat-requirements 重试 | 3 次，`0.35*attempt + rand(0,0.2)` |
| **图片轮询多层退避** | 动态预算制；429 连击≥3 熔断；CF 连击≥1 熔断；`min(2^min(n,4),16)+rand(0,0.5)`；**尊重 Retry-After** |
| conversation POST CF 重试 | 3 次，`0.4*attempt + rand(0,0.25)` |
| 图片 SSE 开流重试 | 3 次，仅瞬态传输错误，**每次重建 session** |
| 图片下载重试 | 3 次，`1.5*attempt`，仅 404/403 |
| 文本回复重试 / 轮询超时重试 | 3 次 / 4 次，**换账号** |
| 前置会话瞬态重试 | 4 次，退避 1s |
| CF swap 重试 | **换代理** |
| 恢复轮询退避 | 首延 5s，base 5s，封顶 60s，总窗 240s |

这些是长期跑出来的经验值，不是可以重新拍脑袋定的参数。

---

## 3. 调度面 gap —— 双轨重复造轮子

### 3.1 三方对照

| 位置 | 规模 | 状态 |
|------|------|------|
| 生产 Python 调度面 | ~8,000 行 | 运行中 |
| **树内 Rust**（`gptimage/crates/`） | 1,107 行 | ✅ 编译为 `native/*.so`，**已上 panda 生产** |
| 网关 Rust（`gptimage-gateway-rs/crates/`） | ~30 行（1 个 Semaphore + 26 行 `resolve_account`） | ⛔ 另有 285 行 `ticket_pool` 编译失败且零引用 |

树内 Rust 已覆盖 4 个点：

| 能力 | 生产 Python | 树内 Rust | 完成度 |
|------|------------|----------|--------|
| 调度 trace（21 事件 + 11 阶段耗时模型） | `schedule_trace.py` + `_model.py` 350 | `image_schedule_trace` 510 | **100%** |
| 双槽账本（account + sS） | `slot_ledger.py` 321 | `slot_ledger.rs` 213 | **80%** |
| sediment:// 流式抽取 | `schedule_core.py:94-147` 54 | `sediment.rs` 58 | **100%** |
| 派发间隔门控 | `schedule_core.py:70-91` 57 | `dispatch_gate.rs` 12 | 20%（算术桩） |
| 账号租约池 | `account_lease_pool.py` 129 | `lease_pool.rs` 18 | 8%（算术桩） |

> 1,107 行 = `image_schedule_core` 597（`lib.rs` 296 + `slot_ledger` 213 + `sediment` 58 + `lease_pool` 18 + `dispatch_gate` 12）+ `image_schedule_trace` 510。上表只列有生产对位的模块，故不含 `lib.rs` 的 FFI 胶水层。

### 3.2 树内 crate 可被网关直接复用

**关键事实**：两个 crate 的 `crate-type = ["cdylib", "rlib"]`。**`rlib` 意味着 `gptimage-gateway-rs` 可以直接 `path` 依赖当普通 Rust 库用，完全绕开 Python 才需要的 C FFI 层**（`isc_*` / `ist_*` handle 注册表都不用碰）。

| crate / 模块 | 可直接调用的 API | 补上网关什么缺口 | 依赖负担 |
|-------------|-----------------|----------------|---------|
| `image_schedule_core::slot_ledger::SlotLedger` | `try_acquire_account` / `release_account` / `try_acquire_ss` / `release_ss` / `account_inflight_for_token` / `watchdog_tick(bool)` / `stats_json` | 账号级与 sS 级并发门 + TTL 自动回收。已有 `SharedSlotLedger = parking_lot::Mutex<SlotLedger>` 别名 | 仅 parking_lot |
| `image_schedule_core::dispatch_gate::DispatchGate::should_wait` | `(interval_ms, inflight, cap, queued) -> bool` | 派发背压，替代裸 semaphore | 无 |
| `image_schedule_core::sediment::SedimentParser` | `feed(&str) -> bool` / `ids_json()` | SSE 直连时抽 sediment:// | 无 |
| `image_schedule_trace`（全部） | `TraceRun::new/emit/to_json`、`EventKind`(21)、`model::build_model` | 阶段耗时模型 —— **白捡 510 行** | serde |
| `image_schedule_core::lease_pool::LeasePool` | `target_fill(inflight)` | **不建议** —— 18 行饱和减法，无实际资源管理 | — |

**阻塞项**：源码只在 `gptimage/crates/`，不在网关 workspace，也不在 panda（panda 只有 `image_schedule_trace` 源码 + 两个 `.so`）。复用需要跨仓 path 依赖 / vendor / git dependency 三选一。

### 3.3 `ticket_pool` 判定：在重造已完成的轮子

| 对比对象 | 重叠度 | 判定 |
|---------|--------|------|
| `ticket_pool` ↔ `pre_ticket_pool.py`(102 行) | **~80%** | 同一件事：per-account 凭据缓存 + TTL 清理。TTL 一个 300s 一个 120s |
| `ticket_pool` ↔ `slot_ledger.rs`(213 行) | ~25% | 只在「TTL 扫描 + 强制淘汰」撞（`refresh()` ≈ `watchdog_tick()`）。数据模型不同，**不能互相替代** |

三个硬问题：编译失败（`uuid` 缺 `serde`）· 零引用 · 语义自相矛盾（`acquire()` 仅在 `PerCallFinalize` 即"不复用"下放行）。

**结论**：不能用树内 crate 直接替代，但也不该保留现状。**建议冻结或删除**，真需要时按 `pre_ticket_pool.py` 语义（token→bundle、TTL 120s、`get_or_fetch`）重写约 60 行即可。

`control_client`(107 行) 同样是孤儿，且其目标端点 `/api/accounts/admission` 在生产 `api/` 下**根本不存在**。

### 3.4 两边 Rust 都缺的调度能力

**P0 —— 缺了就不构成多账号生产调度**

| # | 能力 | 生产行数 | 为什么必需 |
|---|------|---------|-----------|
| 1 | 异步任务队列 + 状态机 + 持久化 | 2,456 | 网关是纯同步请求-响应，semaphore 一满就阻塞 HTTP 连接；无 queued/running/timeout_pending 状态、无落盘、重启即全丢 |
| 2 | 账号选号排名（ACI 打分 + ε-greedy + 6 元组排序） | 104 | `resolve_account` 只做「header email → 查 map → 兜底 pin」，无打分、无排除、无轮换 |
| 3 | binding 级并发门（`image_binding_inflight_max=1`） | 24 | 同代理只允许 1 个在途是**防关联的核心约束**，缺失等于裸奔 |
| 4 | 代理 sticky 绑定 + CF403 转移 + 隔离持久化 | ~800 | 网关的 proxy 只是透传到 helper 的字符串，反检测面为零 |

**P1 —— 缺了掉稳定性**：timeout_pending 续轮询体系（188 行）· 冷却体系（429→900s / 终态→1800-5400s / cohort 2 次→24h）· inflight 漂移对账 · 额度 lazy-refresh + 每账号 sha256 抖动 0-6h（防惊群）· pS/sS 编号槽位 FIFO 池 + ready buffer 背压

**P2 —— 缺了被风控盯上**：拟人化调度全套（350 行：soft-band 燃尽、夜 0.4/午 0.85 权重、Poisson λ=8 抖动）· 工作负载策略（368 行：Rimg 图片预留、IMAGE/TEXT/IDLE 路由、shadow/live 双模、canary allowlist）· 每账号浏览器指纹（219 行：6 profile 确定性选择、impersonate 与 sec-ch-ua 对齐）· 预开票池 · burst 并发 / prompt 去重 / 死锁熔断 · 重启恢复 + 终态清理

---

## 4. 运维面 gap

### 4.1 配置

生产 71 键**全部可热更新**（`ConfigStore.update()` 跑 30+ 个 `_normalize_*` 校验/钳位，写盘后替换内存，消费方经 `@property` 惰性读取）。Rust 是一次性 `env::var`，无校验、无钳位、无持久化、无热更。

**71 键中 Rust 有等价物的：1 个**（`image_global_concurrency` → `IMAGE_GLOBAL_CONCURRENCY`）。

按域分组的缺失重点（非目标已剔除）：

| 域 | 键数 | 缺失影响 |
|----|------|---------|
| `proxy_runtime`（含 `clearance`） | 17 | 出口策略、CF cookie/UA 全无 |
| 生图轮询节奏与各形态超时 | 6 | 轮询全在 helper，Rust 无从调 |
| 并发闸（账号/绑定/并行/热号） | 4 | 只有全局一个 |
| 前置会话超时/重试/退避 | 3 | 无 |
| `image_token_max_attempts`（钳位 20..1000） | 1 | Rust 单次 HashMap 查找，**查不到就造空 token 账号继续走** |
| 预检退避、配额新鲜度、沉降、回收窗口、队列超时 | 10 | 无 |
| `image_generation_paused` | 1 | Rust `IMAGE_ENABLED` 只在启动读一次，**线上出事只能重启** |
| `chat_completion_cache` / 系统提示词 / 敏感词 / AI 复审 | 14 | 无 |

### 4.2 可观测

Rust **完全没有**：结构化日志落盘 · 日志查询/删除 · 日志脱敏 · 请求形状遥测 · 阶段计时 · 调度 trace · 风控指标/巡检/看板 · watchdog · LLM 诊断 agent。

生产侧对应能力：

| 能力 | 实现 | 落盘 |
|------|------|------|
| 调用日志 + 反向分块读取 | `log_service.py` | `data/logs.jsonl` |
| 日志脱敏 | `_strip_internal_response_fields` / `_account_hash` / `_mask_base64` | — |
| 请求形状遥测 | `request_shape.py`，8 个敏感 header 黑名单 | — |
| 阶段计时 | `request_phase.py` 12 相 | — |
| 调度 trace | 21 事件 → 9 个 `phases_ms` + **自动归因**（如 account_queue>5s 提示并发瓶颈） | 内存 + snapshot 端点 |
| 风控指标 / 巡检 / 看板 | `risk_metrics_store` / `risk_audit_service` / `risk_dashboard_service` | JSONL 滚动 |

> ⚠️ Rust `main.rs:501,512` 把 `account.email` **明文**写进 tracing。生产侧对同一信息是 hash 的。

### 4.3 持久化

`grep 'File::create\|fs::write\|OpenOptions' crates/` → **0 处命中**。

Rust 唯一持久化是 `data/auth.db` 的 `users` 表（且是**新增**的 UI 用户体系，不是账号密钥表）。生产侧 20+ 实体全部无对应：账号（SQLite/JSON/PG/Git 四后端可切）· 图片任务（SQLite WAL）· 参考图资产 · 图片文件+缩略图+索引 · 图片标签 · 运行日志 · 风控指标 · 备份状态 · 配置本身。

**账号池在 Rust 侧是 `HashMap`，无 TTL、无失效、无落盘 —— 进程重启即清空。**

### 4.4 运维动作

生产 102+ 个 admin 端点，Rust 4 个。整类缺失：配置读写 · 日志 · 指标看板 · 流水线诊断 · LLM 运维 · 代理治理 · 备份恢复 · 图片存储运维 · 任务重试/取消 · 9 个后台循环的开关。

---

## 5. R2 切流前必须补齐（阻断级）

前提：R2 = Rust face 接管公网 `:8012`，Python helper 仍持有全部数据面。以下是**这条流量路径上归 Rust 的**部分。

| # | 能力 | 为什么切流后没它不行 |
|---|------|-------------------|
| 1 | **日志脱敏** | 公网入口全量请求日志会持续把号池邮箱明文泄给日志系统 |
| 2 | **运行期熔断开关** | `IMAGE_ENABLED` 只在启动读一次，线上出事只能重启进程 = 全量 5xx |
| 3 | **健康探针可用于编排** | `/health` 恒返 200（helper 挂了也只是 `helper_ok: false`），负载均衡器摘不掉坏节点 |
| 4 | **限流** | 生产侧也没有，但生产不在公网。Rust 切流后是公网入口，无限流 + 无 CSRF + CORS `Any` × `allow_credentials` |
| 5 | **结构化日志落盘 + 查询** | 事后 RCA 的唯一依据。现在 stdout 一走，容器重启即失 |
| 6 | **请求阶段计时 + trace_id** | `plan.md` 红线写死「`self` 失败率 = 0 才可晋级」，**没有阶段计时这条红线在 Rust 侧不可验证** |
| 7 | **账号池持久化 + TTL 失效** | 重启 = 号池清空，只能靠 helper 重拉，期间全量失败 |
| 8 | **取号重试上限** | `resolve_account` 查不到就返回 `access_token: ""` 的空账号继续走，把「取不到号」静默降级成「用空 token 请求上游」 |
| 9 | **出口绑定上限** | `proxy_binding_max_accounts` 是防同一出口挂太多号触发风控的闸；Rust 无出口概念 |
| 10 | **配置热更新** | 71 键在生产侧全可热改，Rust 全要重启。超时/并发/退避正是故障时最需要现改的 |

**建议顺序**：先清 [22](22-audit-2026-07-26.md) §1 编译阻断 → 1/2/3/4（安全与止血，工作量小）→ 5/6（RCA 前置，是红线判据基础）→ 7/8/9（号池正确性）→ 10（配置面重构，工作量最大）。

### 明确「Rust 不需要」的（职责留在 Python）

账号运维五环 · 注册机 / Outlook OTP / CPA / sub2api · 拟人调度与养号 · backup / panda_sync · Webshare CF 扫描 · risk 巡检与 LLM agent · 图片存储运维与索引标签 · 异步图片队列（按 `plan.md` 非目标；但见 §1.4 的 P0 争议）

---

## 6. 与进度百分比的关系

本文盘出的 gap 已反映在 [23](23-rewrite-progress.md) 的修正数字里：分母 23,256 行、分子 1,933 行（含树内 Rust 1,107）、功能加权 **12.8%**。

三条影响判读的事实：

1. **12.8% 里有 1,107 行不在本仓** —— 是 `gptimage/crates/` 的树内 crate，走 FFI 被 Python 调用
2. **网关本仓的 826 行数据面代码，接线的只有 232 行，且当前编译失败** —— 本仓实际贡献接近 0
3. **剩余 87% 里，~470 行是真正的技术未知**（TLS 指纹 / PoW / Turnstile），其余是工作量
