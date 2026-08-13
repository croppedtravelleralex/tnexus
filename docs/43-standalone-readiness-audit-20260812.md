# 43 — 独当一面就绪度审计（2026-08-12）

> 审计范围：本地代码（`TNexus` / 对照源 `gptimage` / `grokImage`）+ Panda 生产实测。
> 审计方式：只读。未在 Panda 执行任何构建、重启、部署或写操作。
> 对照基准：`docs/35`、`docs/37`、`docs/38`、`docs/40` —— **本文以代码与线上数据为准，与既有文档冲突处以本文为准**（见 §7）。

---

## 0. 结论

**能接管流量，不能无人值守。**

| 维度 | 判定 | 依据 |
|------|------|------|
| 承载生产流量 | ✅ **已做到** | NewAPI 近两日占比 57% → 65%；7 天配额反超老系统 |
| 性能 | ✅ **优于老系统** | 平均 25–42s vs 41–70s，快约 40% |
| 成功率 | ✅ **修正后优于老系统** | 68.4% vs 58.2%（扣除已修复的 JWT 项） |
| 服务稳定性 | ✅ 良好 | 7 容器 RestartCount=0，7 天无 OOM |
| **无人值守存活** | ❌ **不成立** | 号池 token 无自动续期，**2026-08-18 归零** |
| **故障可观测** | ❌ **失效** | `/health` 与 watchdog 全绿时号池已死一半 |
| Grok 对话 | ❌ **完全不可用** | 共享出口 IP 被 per-IP 限流 |

**一句话**：`能扛流量`靠代码质量，已经达标；`能无人值守`靠后台自愈环，而这类能力在 Rust 移植时被整体跳过（§4.1）。

---

## 1. 正面证据

### 1.1 流量已经切过来了

NewAPI 通道 `84 = chatgpt2api`（老 Python `:8012`）、`114 = tnexus-gateway`、`115 = tnexus-dedicated`。

| 日期 | ch84 老系统 | ch114+115 TNexus | TNexus 占比 |
|------|------------|------------------|-------------|
| 08-04 | 292 | 115 | 28% |
| 08-05 | 244 | 200 | 45% |
| 08-06 | 257 | 115 | 31% |
| 08-07 | 127 | 64 | 34% |
| 08-08 | 328 | 152 | 32% |
| 08-10 | 177 | 97 | 35% |
| 08-11 | 30 | 39 | **57%** |
| 08-12 | 17 | 31 | **65%** |

- 08-04 之前 TNexus 通道无任何流量，切流起点明确。
- 近 7 天配额：TNexus **45.1M** > 老系统 **44.6M**。
- 生命周期累计 ch84 = 1.2B，反映的是历史存量，非当前分工。

### 1.2 性能优于老系统

平均 `use_time`：ch114 = 42.1s、ch115 = 37.8s、ch84 = 67.7s（7 天）。最近两日 TNexus 降至 25–28s。

### 1.3 成功率（同口径拆解）

14 天窗口，`logs.type`：`2`=成功，`5`=错误。

| 通道 | 成功 | 失败 | 原始成功率 |
|------|------|------|-----------|
| ch84 老系统 | 2722 | 1955 | 58.2% |
| ch114+115 TNexus | 399 | 419 | 48.8% |

TNexus 419 次失败的构成：

| 错误 | 次数 | 归类 |
|------|------|------|
| `401 invalid session` | **235** | 系统缺陷 — **已于 08-11 修复** |
| `502 内容策略拦截`（暴力/色情/政策） | 55 | 上游策略，非系统故障 |
| `poll timeout` | 29 | 系统缺陷 |
| `500 upstream error: do request failed` | 20 | 上游/网络 |
| `401 user no longer exists` | 18 | 号池账号失效 |
| `502 sse ended before image file_id predicate` | 17 | 系统缺陷 — 协议解析 |
| `400 image field required` / `multipart 解析失败` | 12 | 客户端错误 |
| `429 duplicate-prompt` | 8 | 设计内限流 |
| `502 413 payload too large` | 6 | 客户端错误 |
| `502 image_instant_limit` | 3 | 上游限额 |

**扣除已修复的 235 次 `invalid session` 后：399 /（399+184）= 68.4%，高于老系统 58.2%。**

对照老系统 ch84 的失败构成（更健康的鉴权，更糟的超时）：客户端参数错误 439、内容策略 432、硬超时 624、duplicate-prompt 215，**无任何鉴权类故障**。

### 1.4 `invalid session` 已结构性修复

按日分布：08-06=44、08-07=35、08-08=36、08-09=5、08-10=92、08-11=23、**08-12=0**。

`/etc/cron.d/tnexus-jwt-watchdog` 每 15 分钟探测，当前 TTL ≈ 83000s（23 小时）健康：

```
2026-08-12T16:30:01 [jwt-watchdog] env_key ttl=83562s
2026-08-12T16:30:01 [jwt-watchdog] env_key probe /v1/models -> 200
2026-08-12T16:30:01 [jwt-watchdog] channel_key probe /v1/models -> 200
2026-08-12T16:30:01 [jwt-watchdog] OK no refresh needed
```

### 1.5 交付验证脚本 11 项全通过

`bash /root/TNexus/scripts/verify_delivery.sh` → `=== ALL PASS ===`，含**一次真实生图**（`gateway_image` HTTP 200）、Grok 分页、养号状态、额度刷新、JWT 有效性、NewAPI 通道密钥同步、387 个 Grok session key、JWT cron。

### 1.6 主机与服务稳定

7 个 panda 容器 `RestartCount=0`、`OOMKilled=false`；7 天内 `dmesg` / `journalctl` 无 OOM 记录；TNexus 全栈内存合计约 95 MiB；磁盘 63%。

---

## 2. P0 阻塞：GPT 号池静默死亡（6 天倒计时）

### 2.1 现状

解码 `tnexus_accounts.access_token` 的 JWT `exp` 声明：

```
 total | expired_now | expiring_24h | earliest_exp        | latest_exp
-------+-------------+--------------+---------------------+---------------------
    26 |          11 |            0 | 2026-08-09 14:38+00 | 2026-08-18 01:35+00
```

- **11/26 已过期，15 个可用。**
- token 生命周期恰好 **10 天**；池内最新一次 `last_token_refresh_at` 是 **2026-08-08T01:35**，已 4 天无刷新。
- **2026-08-18 可用账号归零。**

### 2.2 过期账号与线上报错精确对应

24h 内 gateway `chat_requirements_prepare HTTP 401 / code=token_expired` 涉及的账号，与上述 11 个过期账号 **11/11 完全一致**，共 54 次 401、6 次 `image call failed`，对外表现为 502。

### 2.3 根因链

新鲜 token **就在同一台机器上**：老 Python 维护的 SQLite 号池 `/root/gptimage/data/accounts.db` 今日 06:51 仍在刷新，25/26 有效，到期日排到 08-19 ~ 08-22。TNexus 读不到，链条如下：

1. `ACCOUNTS_BACKEND=postgres` → 网关读 Postgres `tnexus_accounts`。
2. 网关本应通过 helper 拉取实时候选，但启动日志：
   ```
   WARN HELPER_INTERNAL_TOKEN unset — helper fails closed on /v1/internal/*
   WARN helper candidates unavailable; using pin/ACCOUNTS_DB only
   ```
   已验证 `HELPER_INTERNAL_TOKEN` 在 `/opt/tnexus/.env` 与 gateway 容器环境中**均不存在**（`grep -c` 均为 0）。
3. 退回读 Postgres，而该表最后同步于 **08-08**。
4. → `401 token_expired` → 对外 502。

### 2.4 为什么没人发现

| 机制 | 应有作用 | 实际行为 |
|------|---------|---------|
| `scripts/etl_accounts_to_postgres.py` | SQLite → PG 同步 token | 文档字符串写明 `One-shot ETL`；**cron 中不存在**，跑过一次即止 |
| `scripts/reconcile_accounts_postgres.py` | 同步后校验 | **只比对行数**（26 = 26 恒等），不看 token 新鲜度，永不告警 |
| `GET /health` | 号池健康 | 返回 `accounts:25` —— 只统计调度开关，**完全不校验 token** |
| `jwt_watchdog.sh` | JWT 看门狗 | 守的是 `UPSTREAM_API_KEY` / `GATEWAY_AUTH_KEY`（网关会话层），**与账号 token 无关**，持续报 OK |

**所有仪表盘绿灯，同时号池已死一半。**

### 2.5 叠加问题：号池只在启动时加载一次

```
docker logs panda-gateway-1 | grep -c "accounts hydrated from pool backend"   # → 1
```

唯一一次发生在 `2026-08-11T15:05:26Z`。即使修好 Postgres，运行中的网关仍会沿用旧快照。**好消息**：`POST /api/accounts/reload-from-storage` 在 `tnexus-api` 与 `gateway` 均已实现，无需重启容器。

### 2.6 修复路径

| 方案 | 动作 | 代价 | 是否解除对老 Python 的依赖 |
|------|------|------|---------------------------|
| **止血（推荐先做）** | 跑 `etl_accounts_to_postgres.py` → `POST /api/accounts/reload-from-storage` → 给 ETL 挂 cron | 无需改代码、无需构建 | ❌ 仍依赖 SQLite 由 Python 刷新 |
| 补监控 | 把 token `exp` 纳入 `jwt_watchdog.sh` 与 `/health` | 改脚本 + 提交 | ❌ |
| **根治** | 在 `tnexus-account-ops` 实现原生 token 续期环（26 个账号均已存有可用 `refresh_token`） | 需开发 | ✅ |

---

## 3. P0 阻塞：Grok 对话链路完全不可用

### 3.1 现象

`POST /v1/chat/completions` 对 `grok-chat` 与 `grok-vision-ocr` 均在 **0.3 秒**内返回 502：

```json
{"error":{"code":null,"message":"chat status 429 Too Many Requests","param":null,"type":"upstream_error"}}
```

`grok_request_audits` 最后一条停在 **2026-08-08 02:53 UTC**，故障已持续 4 天。

### 3.2 根因：698 个账号共用单一出口 IP，被 per-IP 端点级限流

决定性实验（容器为 host 网络，DNS 解析与宿主一致，无观测盲区）：45 秒采样窗口内发生 2 次 nurture chat 失败，**到 grok.com 边缘 IP 的直连 socket 数为 0**，同时到代理的连接池稳定保持 2 条 established。

→ chat 流量确实经代理出站，429 是代理 IP 上返回的上游响应。

| 出口 | IP | grok.com 根路径 | chat 端点 |
|------|----|----------------|-----------|
| 宿主直连 | `43.156.233.219`（腾讯云） | **403**（~50ms 硬封） | 403 |
| 代理 | `70.39.164.200:30000` | 200（16.5s） | 401（未带凭据的正确响应） |

- `GROK2API_PROXY_LIST` **仅 1 个条目**，698 个账号全部共用。
- 该 IP 日流量约 **3.2 万次**（`web_quota_refresh` ≈ 30,600 + nurture 1,357）。
- 匿名 GET 返回 200/401 → 不是 IP 黑名单，是**端点级速率限制**：廉价的 `/rest/rate-limits` 放行，昂贵的 `/rest/app-chat/conversations/new` 限流。
- 宿主 IP 已被硬封，**无备用出口**。

> 注：`GROK2API_DIRECT=1` 含义是「无 Chrome 的纯 HTTP 模式」，**不是**「绕过代理」。

### 3.3 被证伪的假设

| 假设 | 证据 |
|------|------|
| session key 缺失导致 bot 检测拦截 | ❌ 363 个 available 账号 **100% 有 key**，缺失 0；挂载正确（宿主 574 = 容器 574）；key 每分钟实时生成，最新在检查前几分钟 |
| 模型 soft_stop / 配额冷却封死 chat | ❌ `grok_model_quota_blocks` **整表 0 行**；账号冷却 0 个；chat 模型在 `grok_model_states` 零行；配额剩余 auto 4753 / fast 20283 |
| `web_quota_refresh` 成功说明链路健康 | ❌ 该端点在 `remainingQueries=0` 时**仍返回 200**，`ok=32` 是假象 |

### 3.4 代码侧的两个独立缺陷

**（1）对话路径无跨账号重试。**
`crates/grok-provider-web/src/engine.rs:80-83,150-154` — 选中 1 个账号，失败即返回 502，不会尝试其余账号。使用的 `SimplifiedPool`（`crates/grok-pool/src/lib.rs:74-86`）仅按 `provider=grok_web AND enabled=true` 过滤，**不读 PG 的 `cooldown_until` / `auth_status` / `failure_count`**，且**启动时加载一次不再 reload**。失败仅进入 2 秒内存冷却，不写 PG。

这解释了「后台显示 137 个可用，请求却在 0.3 秒内全灭」——它只试了 1 个。

错误产生位置：`crates/grok-provider-web/src/direct.rs:373-376`，为 `resp.status()` 的直译，经 `crates/grok-gateway/src/error.rs:81-92` 包装为 502 / `upstream_error`。

**（2）养号间隔实际值是配置值的 7.5 倍。**
`grok-compose.yml:30` 配置 `GROK_NURTURE_INTERVAL_SECS=480`，实测日志间隔精确为 **30 秒**（失败退避封顶 30s，`crates/grok-ops/src/scheduler.rs:182-189`），24h 失败 **1357 次**（按配置应为 180 次）。形成「限流 → 失败 → 30s 快速重试 → 维持限流」正反馈，并持续污染号池状态（6 小时内 698 行 `grok_accounts` 全部被改写，禁用/启用抖动）。

### 3.5 修复路径

1. **扩充 `GROK2API_PROXY_LIST` 为多 IP 池**（根本解，代理按账号哈希分散已实现于 `proxy.rs:62-68`）。
2. 对话路径接入完整 `Selector`，增加跨账号重试与 429 → PG 冷却写入。
3. 修复养号退避不遵守配置间隔的缺陷；临时可置 `GROK_NURTURE_ENABLED=0`（纯配置开关）。

**确认头号假设的单一诊断**：用同一账号 + 同一 session key，分别经 `70.39.164.200` 与**一个全新未使用过的出口 IP** 各发一次已签名 chat 请求，比对状态码。**不可用宿主 IP 作对照组**（已被硬封 403，会产生假阴性）。

---

## 4. P1：后台自愈能力整体缺失

### 4.1 20 个 legacy 后台环的移植状况

老系统在 `api/app.py:38-83` 的 lifespan 启动约 20 个后台环，Rust 侧对等实现情况：

| Legacy 后台服务 | 作用 | Rust 对等 | 状态 |
|----------------|------|-----------|------|
| `proactive_refresh_loop_service` | 主动 token 刷新 | 无 | ❌ **missing** — 直接导致 §2 |
| `account_maintenance_loop_service` | 号池维护 | 无 | ❌ missing |
| `image_quota_refresh_service` | 周期额度刷新 | 无 | ❌ missing |
| `quota_refresh_schedule_service` | 额度刷新计划 | 无 | ❌ missing |
| `account_cf_refresh_service` | 账号 CF 刷新 | 无 | ❌ missing |
| `risk_audit_service` | 风险审计 | 无 | ❌ missing |
| `account_warmup_service` | 新号预热 | 无 | ❌ missing |
| `start_limited_account_watcher` | 受限号周期 refresh | 无 | ❌ missing |
| `text_conversation_expiry_service` | 对话过期清理 | 无 | ❌ missing |
| `start_image_cleanup_scheduler` | 磁盘旧图清理 | 无 | ❌ missing |
| `webshare_cf_scan_service` | 周期 CF 扫描 | 仅 `run-once` API | ⚠️ 无后台 interval |
| `pipeline_watchdog_service` | inflight 泄漏检测 | `reconcile_stale_inflight`（5min） | ⚠️ partial |
| `outlook_auto_recovery_loop_service` | Outlook 恢复 | `account-ops/workers.rs` | ⚠️ 仅 token refresh，无 OTP 链 |
| `quota_window_prime_service` | 窗口预热 | `quota_prime_loop` | ⚠️ partial |
| `image_task_service` | 异步生图队列 + 背压 | `tnexus-worker` Redis BLPOP | ⚠️ 协议不同，无 ready_buffer / return_window / deadlock_guard |
| `text_nurture_service` | 文本养号 | `account-ops` `nurture_loop` | ✅ covered |
| `panda_staging_service` | 暂存同步 | 无 | 🚫 intentionally dropped |
| `register_service` | 自动注册 | 无 | 🚫 intentionally dropped |
| `backup_service` | 定时备份 | 无 | 🚫 intentionally dropped |

**约 10 个 missing、5 个 partial。** 这不是「某个脚本忘了跑」，是一整类自愈能力尚未移植 —— §2 的号池衰减是它的第一个显性后果。

### 4.2 其余能力缺口（按严重度）

| 严重度 | 缺口 | 说明 |
|--------|------|------|
| major | `/v1/responses`、`/v1/messages`、`/v1/search` | gateway 无路由 |
| major | 参考图资产 API（`POST /api/image-assets/references` + `panda-asset://`） | 多参考图 edits 工作流断裂 |
| major | API Key 管理面不兼容 | legacy 是 key CRUD，TNexus 是 Postgres 用户表；持旧 key 的客户端可能 401 |
| major | 对话生图 + 多轮会话持久化 | `docs/38` Phase 1 未完成 |
| major | 热配置 / 暂停闸（`image_generation_paused`） | 无法运维紧急保号 |
| minor | 异步任务协议（`panda_async`、`/api/image-tasks`） | **对当前生产链路不构成阻塞**，见 §7.2 |
| minor | PPT/PSD、`/version`、磁盘 backup、CPA/sub2api | 明确非目标或低流量 |
| minor | ops UI 缺 warmup / risk-calendar / agent 面板 | `web/src/app/(console)/ops/page.tsx` 为简化版 |

---

## 5. P2：其余问题

### 5.1 4 个任务永久卡死

| id 前缀 | 状态 | 创建时间 | 年龄 |
|---------|------|---------|------|
| `2df8da8b` | generating | 2026-07-30 06:46 | 13.1 天 |
| `18120f93` | queued | 2026-08-01 03:32 | 11.2 天 |
| `91dbc671` | queued | 2026-08-01 03:33 | 11.2 天 |
| `59038866` | queued | 2026-08-01 03:33 | 11.2 天 |

3 个 queued 的 `updated_at == created_at`，从未被触碰；`generating` 那个在创建后 4 秒被 worker 认领后即失联。

Redis 侧 `tnexus:jobs` 键**不存在**（`EXISTS` → 0，`DBSIZE` → 1）。配置为 `appendonly no` + 粗粒度 RDB，2026-08-08 重启时队列条目全部丢失，而 Postgres 保留了行。**系统无 reaper、无启动重排逻辑**，这 4 个任务永远不会被处理。

worker 本身健康：`RestartCount=0`，全时段日志仅 2 行启动横幅 —— 这是 `blpop(..., 5.0)` 空转的正常特征，不是故障。

14 个 failed 任务中 **10 个（71%）是 HTTP 401 鉴权失败**，与 §2 同源。

### 5.2 主机内存余量

```
Mem:  3.6Gi total, 2.0Gi used, 145Mi free
Swap: 1.9Gi total, 1.8Gi used, 171Mi free   ← 94%
```

无 OOM 历史，`available` 名义 1.6Gi，非现行事故；但突发吸收能力很低。`new-api-postgres` 已用硬上限 512MiB 的 29.6%。

### 5.3 安全观察

`/opt/tnexus/.env` 中 `GROK_ADMIN_SECRET`、`GROK_CREDENTIAL_KEY`、`GROK_ADMIN_PASSWORD` 及代理凭据均明文存储；`GROK2API_PROXY_LIST` 为 `host:port:user:pass` 明文格式。建议后续收敛到密钥管理。

---

## 6. 建议执行顺序

| 优先级 | 动作 | 需要构建？ |
|--------|------|-----------|
| **P0-1** | 跑 ETL 同步 token → `POST /api/accounts/reload-from-storage` → 给 ETL 挂 cron | 否，现成脚本 |
| **P0-2** | 把 token `exp` 纳入 `jwt_watchdog.sh` 告警与 `/health` | 改脚本，需提交 |
| **P0-3** | Grok：扩充多 IP 代理池 | 否，配置 + 采购 |
| P1-1 | `tnexus-account-ops` 实现原生 token 续期环（解除对 Python 的依赖） | 是 |
| P1-2 | Grok 对话接入完整 Selector：跨账号重试 + 429 写 PG 冷却 | 是 |
| P1-3 | 修养号退避不遵守配置间隔的缺陷 | 是 |
| P2-1 | Redis 开 `appendonly` 或实现 job reaper / 启动重排 | 是 |
| P2-2 | 补齐 §4.1 中 missing 的后台环 | 是 |

> 一切改动遵守红线：本地/CI 构建 → `git push` → GHCR Actions → Panda 仅 `deploy.sh`（pull + up）。

---

## 7. 与既有文档的冲突

### 7.1 就绪度自相矛盾

| 文档 | 声称 | 实查 |
|------|------|------|
| `35-tnexus-gptimage-gap.md` | 加权 ~90% | 该文档内部「仍缺 dispatch/duplicate/n>1」段落**已过时**，这些均已实现 |
| `40-tnexus-shutdown-readiness.md` | 停服就绪 ~91% | 衡量的是「TNexus 自身闭环 + 管理台」，非「外部 API 1:1 替代」 |
| `37-gptimage-tnexus-comparison.md` | ~50% 不可替代 | **更接近本次实查结论** |
| `24-gap-inventory.md` | `n!=1` → 400、恒返 b64、0 后台循环 | **大量过时**：现支持 n≤4、url 模式、3 个 account-ops 环 |
| `35` | account-ops 需 `GPTIMAGE_ROOT` | 实际仅查 `ACCOUNT_OPS_TOKEN`（`account_ops.rs:147-149`）；错误提示文案仍写 `GPTIMAGE_ROOT`（`routes/ops.rs:129`），**文档与提示均过时** |
| `35` | humanlike ❌ | 已有简化版 `humanlike.rs`（ε-greedy），非 Python 完整 scheduler |

**结论：文档不可作为就绪度依据，以代码与线上数据为准。**

### 7.2 一处需要修正的推断

初版差距分析将「异步生图协议缺失（`panda_async`、`/api/image-tasks`）」列为切流 blocker。**线上数据反驳了这一点**：NewAPI 通道 114/115 的平均 `use_time` 为 25–72 秒，若走异步立即返回 + 轮询应为 1–2 秒；该耗时特征证明真实流量走的是**同步阻塞调用**，且 399 次成功、配额真实扣减。

→ 异步协议对**当前生产链路**不构成阻塞，仅对可能存在的其他客户端有影响，降级为 minor。

---

## 8. 附：关键验证命令

```bash
# 全量交付验证（含真实生图）
ssh panda 'cd /root/TNexus && bash scripts/verify_delivery.sh'

# 号池 token 过期盘点（解码 JWT exp）
ssh panda 'docker exec panda-postgres-1 psql -U tnexus -d tnexus -c "..."'   # 见 §2.1

# 通道流量与成功率
ssh panda 'docker exec new-api-postgres psql -U newapi -d "new-api" -t -A -F" | " \
  -c "select (to_timestamp(created_at))::date d, channel_id, count(*), round(avg(use_time),1) \
      from logs where created_at > extract(epoch from now())::bigint - 1209600 \
      and channel_id in (84,114,115) group by 1,2 order by 1 desc,2;"'

# 号池是否只加载一次
ssh panda 'docker logs panda-gateway-1 2>&1 | grep -c "accounts hydrated from pool backend"'

# Redis 队列是否存在
ssh panda 'docker exec panda-redis-1 redis-cli EXISTS "tnexus:jobs"; docker exec panda-redis-1 redis-cli DBSIZE'

# Grok session key 覆盖率
ssh panda 'ls /opt/tnexus/pure_http_keys | wc -l; docker exec panda-grok2api-rs-1 ls /opt/tnexus/pure_http_keys | wc -l'
```

> PowerShell 下调用 `ssh panda '...'` 必须用**单引号**包裹外层，否则 `$` 会被本地插值。

---

## 9. 变更记录

| 日期 | 变更 |
|------|------|
| 2026-08-12 | 初版：四路并行审计（代码 / GPT 号池 / Grok 号池 / 能力矩阵）+ 线上流量取证 |
