# 35 — TNexus 距离彻底替代 Python gptimage 还差多少

最后更新：**2026-08-03（account-ops 后台 worker + dispatch_gate 接线 + Postgres 默认草案）**

## 结论（一句话）

**管理台已去生产 HTTP 代理**。  
account-ops 已切 **Rust 二进制**（OAuth + token refresh）；养号/Outlook/预热仍待迁移（当前 501）。  
号池支持 **`ACCOUNTS_BACKEND=postgres`**（`009` + ETL）；默认仍共享 sqlite。  
**数据面**走 gateway `:8014` Rust upstream。

---

## 进度总览（百分比）

| 维度 | 完成度 | 说明 |
|------|--------|------|
| **号池管理台 UI** | **~94%** | 对话页、号池、图片管理 |
| **管理 API（tnexus-api）** | **~82%** | 图片持久化、对话 CRUD |
| **运维执行面（account-ops）** | **~88%** | Rust：OAuth + refresh + **密码重登** + 养号/outlook/预热 worker |
| **号池数据独立** | **~78%** | Postgres 默认 + ETL + 对账脚本 |
| **Gateway OpenAI 兼容** | **~88%** | + `asset_ids` 多参考图 |
| **Gateway 调度 / dispatch / 背压** | **~78%** | scheduling_gate + humanlike + dispatch_gate |
| **错误码分档（OpenAI type）** | **~78%** | `error.type` + 动态 `estimated_wait_secs` |
| **彻底替代 Python gptimage（加权）** | **~90%** | 见下表加权模型 |

> 百分比对照 `docs/24-gap-inventory.md` 与 Panda `:8012`，**不含**明确不做项。

### 加权「彻底替代」计算公式

| 块 | 权重 | 当前 | 目标（你列的 6 项全做完） |
|----|------|------|---------------------------|
| A 管理台 UI | 8% | 94 | 98 |
| B 管理 API（jobs/图片/对话） | 10% | 82 | 92 |
| C account-ops Rust 重写 | 18% | 88 | 95 |
| D 号池 DB 独立（非共享 sqlite） | 18% | 78 | 95 |
| E Gateway 兼容（mask/n>1/errors） | 14% | 88 | 95 |
| F dispatch + humanlike + 背压队列 | 22% | 78 | 90 |
| G 数据面对话/Studio parity | 8% | 65 | 88 |
| **加权合计** | **100%** | **≈90%** | **≈93%** |

**当前加权 ≈ 90%**（停服就绪 ~91% 待本轮 deploy 复测）。到 **95%** 仍需：Panda 实切 Postgres（**不停 :8012 进程，仅迁数据**）、`:8012` humanlike 压测对照、Sentinel PoW 全自动化、灰度切流 7 天。见 [40-tnexus-shutdown-readiness.md](40-tnexus-shutdown-readiness.md)。

---

## 替代路线图（你提出的 6 项）

### 1. account-ops Rust 重写（C：22% → 95%）

| 阶段 | 交付 | 估工时 |
|------|------|--------|
| **C1** | 新 crate `tnexus-account-ops`（axum）+ `/health`；Docker 换 Rust 镜像 | 3d |
| **C2** | OAuth PKCE（迁 `oauth_login.py`）+ refresh token + `get_user_info` | 5d |
| **C3** | 密码重登（upstream 登录协议） | 5d |
| **C4** | 养号 worker（text nurture 队列） | 8d |
| **C5** | Outlook 恢复 + 自动恢复环 | 8d |
| **C6** | 窗口预热 + Webshare CF 扫描 + 代理 runtime | 6d |
| **C7** | 去掉 `GPTIMAGE_ROOT` 挂载；tnexus-api 直连同进程或 gRPC | 3d |

**验收**：Panda `account-ops` 容器无 `/gptimage` 卷；`refresh-one` / `nurture/status` 全绿；`helper/account_ops_face.py` 退役。

### 2. 号池数据独立（D：32% → 95%）

| 阶段 | 交付 | 估工时 |
|------|------|--------|
| **D1** | `migrations/009_tnexus_accounts.sql` 接线 + `AccountsStore` Postgres 后端 | 4d |
| **D2** | ETL：`accounts.db` → Postgres 一次性迁移脚本 | 2d |
| **D3** | gateway / scheduling_gate 读 Postgres（或 TNexus 独占 sqlite 文件） | 5d |
| **D4** | 双写对账期 → 切断 `/root/gptimage/data` 卷 | 3d |
| **D5** | 删除 `account_pool_sync.py`、Python `account_service` 写路径 | 2d |

**验收**：`ACCOUNTS_DB` 不再指向 gptimage 路径；8012 与 TNexus 可并存但**不同步**（符合 plan 红线）。

### 3. dispatch_gate + humanlike + 背压（F：42% → 90%）

| 能力 | 现状 | 待补 |
|------|------|------|
| `scheduling_gate` | ✅ 过滤/inflight/quota | — |
| `dispatch_gate` | ⚠️ `dispatch_gate.rs` 骨架 + 测试 | 接入 per-account interval + 队列深度 |
| `humanlike_scheduler` | ❌ | 时段权重、Poisson、workload、ACI/ε-greedy（~350 行 Python 对等） |
| 背压队列 | ⚠️ 全局 semaphore 429 | `image_task_service` 级：ready_buffer、return_window、`estimated_wait` 动态 |
| lease_pool / slot_ledger | ❌ | vendor `image_schedule_core` 或自研子集 |

**验收**：高并发压测下 429 带动态 `estimated_wait_secs`；选号分布与 `:8012` humanlike 对照偏差 < 15%。

### 4. mask / n>1 / 错误码（E：76% → 95%）

| 能力 | 现状 |
|------|------|
| mask edits（JSON + multipart） | ✅ |
| n=1..4 批处理 | ✅ |
| `error.type` OpenAI 映射 | ✅ **2026-08-03** |
| `estimated_wait_secs` on 429 | ✅ 默认 30（Gate 类） |
| asset_ids / 多参考图 | ❌ |
| duplicate_prompt 429 | ❌ |

---

## 本轮已完成（2026-08-03 第四轮 — 90%）

| 项 | 说明 |
|----|------|
| `relogin.rs` | 密码重登：authorize/continue + password/verify + workspace/token 兑换 |
| `asset_ids` | `ImageGenerationRequest` / `ImageEditRequest` + upstream 多参考图 |
| `scripts/reconcile_accounts_postgres.py` | sqlite vs Postgres 行数对账 |
| `deploy/panda/docker-compose.yml` | 注释 gptimage 卷切除步骤 |

## 本轮已完成（2026-08-03 第三轮 — 85% 推进）

| 项 | 说明 |
|----|------|
| `nurture.rs` + worker | 后台队列消费；直连 `/backend-api/conversation` 文本养号 |
| `workers.rs` | outlook token refresh 环、quota-window prime 调 gateway `/v1/images/generations` |
| `ops.rs` | Webshare CF 扫描（`WEBSHARE_API_KEY`）、代理 runtime 持久化 |
| `dispatch_gate` 接线 | `image_generations` / `edits` 在 semaphore 前检查 inflight + 队列深度 |
| `humanlike` | `collect_image_accounts` 排序后首账号派发 |
| `duplicate_prompt` | 429 + 动态 `estimated_wait_secs` |
| Panda `.env.example` | 默认 `ACCOUNTS_BACKEND=postgres` |

## 本轮已完成（2026-08-03 续）

| 项 | 说明 |
|----|------|
| `tnexus-account-ops` crate | Rust 二进制：OAuth PKCE、refresh-one（token 轮换） |
| `Dockerfile.account-ops` | 改为 Rust 多阶段构建（不再 Python 镜像） |
| `AccountsBackend` | sqlite / postgres 双后端；`ACCOUNTS_BACKEND=postgres` |
| `scripts/etl_accounts_to_postgres.py` | sqlite → `tnexus_accounts` 一次性迁移 |

## 本轮已完成（2026-08-03）

| 项 | 说明 |
|----|------|
| OpenAI `error.type` | `invalid_request_error` / `rate_limit_error` / `server_error` / `authentication_error` |
| `estimated_wait_secs` | Gate 类错误 JSON 内嵌 30 |
| `dispatch_gate.rs` | 算术门控 + 单元测试（待队列接线） |
| `migrations/009_tnexus_accounts.sql` | Postgres 号池 + runtime 表草案 |
| 对话 UI | 流式/生图默认开、去掉「走号池…」文案 |

### Studio / 号池 UI

| 能力 | 实现 |
|------|------|
| 出图角标：分辨率 + 文件大小 | ✅ migration `008` + `output-panel`；历史记录 HEAD/原图探测 |
| 号池「同步全部额度」（无选中 → `refresh-all`） | ✅ `accounts/page.tsx` |
| Worker URL 模式元数据落库 | ✅ 无 R2 时下载 `source_url` 写 `width/height/size_bytes` |

### Gateway / upstream

| 能力 | 实现 |
|------|------|
| `POST /v1/images/edits`（单图 base64 → upload → multimodal SSE） | ✅ `1ab5d25`；`IMAGE_ENABLED=1` + `DATA_PLANE=upstream` |
| `capabilities.image_edits` | ✅ 同上条件为 `true` |
| mask / 多参考图 / multipart | ❌ 待补 |

---

## 本轮已完成（2026-07-31）

### 去代理化（P0）

| 项 | 状态 |
|----|------|
| 删除 `gptimage_proxy.rs` 及所有 `:8012` HTTP 代理 | ✅ |
| `GPTIMAGE_ADMIN_TOKEN` 废弃（`.env.example` 已标注） | ✅ |
| 无本地能力时返回 503 /「暂无」，不借用生产 | ✅ |

### 号池 UI / API

| 能力 | 实现 |
|------|------|
| 全列单点升/降序排序 | ✅ `account-sort.ts` + 表头 |
| 图片管理缩略图 / 懒加载 / WebP thumb API | ✅ |
| 灯箱滚轮 0.05×–20× + 拖拽 | ✅ |
| 本地 stats / schedulable-breakdown / activity 流水 | ✅ `usage_events` + 号池 |
| 全量慢刷 refresh-all | ✅ 本地 `refresh_all.rs`；号池工具栏无选中时一键触发 |
| 软封 / 删除 / 更新账号 | ✅ 共享 sqlite（`tnexus-accounts-db`） |
| 养号日历预设/绑定 | ✅ 本地 JSON + `ip-nurture` API |
| 生图 b64 持久化（worker 写 `inline_preview_b64`） | ✅ `a54d057` |
| 图片管理优先内联 b64 缩略图 | ✅ `9dcb82a` |
| Studio 轮询 `GET /api/jobs/{id}/status`（仅 status/error，无 b64） | ✅ |
| Job 详情/列表 thumb 走 `/api/images/thumb/{id}`（普通用户可读自己的图） | ✅ |
| 生图 UI 实时耗时（500ms tick，不再固定 1 秒） | ✅ |

### 运维执行面（account-ops → gptimage 库）

| 能力 | 端点 | 前置 |
|------|------|------|
| 养号 status/enable/enqueue/process-one | `/v1/nurture/*` | `ACCOUNT_OPS_TOKEN` + `GPTIMAGE_ROOT` |
| Outlook 单账号恢复 + 进度 | `/v1/outlook/recover-*` | 同上 + Outlook 凭据 |
| Outlook 自动恢复环 | 启动时 `start_background()` | 同上 |
| 窗口预热 quota-window prime | `/v1/quota-window/prime` | 同上；回退 `GATEWAY_BASE` 生图 |
| TNexus API 委托 | `/api/ops/nurture/*`、`/api/accounts/recover-outlook` 等 | account-ops 可达 |

### Gateway（需单独部署 `:8014` 二进制/镜像）

| 能力 | 状态 |
|------|------|
| `scheduling_gate.rs` 读 `SCHEDULING_STATE_FILE` + live `ACCOUNTS_DB` | ✅ |
| 生图选号过滤（状态/调度/额度/软封/inflight） | ✅ |
| `scheduling_bulk` 写调度状态 | ✅ |
| 生图 inflight 计数 | ✅ |
| 429 `Retry-After: 30` | ✅ |
| `POST /v1/images/edits` | ✅ upstream `1ab5d25`（单图；mask 待补） |

---

## 号池「相同账号、8012 通 TNexus 不通」—— 根因（2026-07-31 复核）

**结论：不是「号池 OAuth session 集体过期」。** 生产 `chatgpt2api-local :8012` 与 TNexus `:8014` 是**两套运行时**，即便账号 email 相同，数据与鉴权链路也不同。

### 对照表

| 维度 | 生产 gpt-image `:8012` | TNexus `:8014` + worker |
|------|------------------------|-------------------------|
| 进程 | `chatgpt2api-local` | `panda-gateway-1`（`tnexus-gateway`） |
| 号池数据源 | **live** `/root/gptimage/data/accounts.db` | **同一文件**（容器内 `/gptimage/data/accounts.db`） |
| 同步时机 | 实时读写 sqlite | 实时读写 sqlite（WAL + 事务，无 JSON 快照） |
| 调度/额度 | Python `humanlike_scheduler`、预检、背压 | Rust `scheduling_gate` + `DATA_PLANE=upstream` |
| 调用方鉴权 | API Key（`Authorization: Bearer`） | Worker 用 **`UPSTREAM_API_KEY`（Gateway JWT，≈24h TTL）** |
| 生图实现 | Python `image_task_service` 全链路 | Rust `upstream` SSE（能力子集） |

### 本次 TNexus 401 的真实原因（已复现并修复）

1. **`UPSTREAM_API_KEY` 过期**（`exp` 早于 `now`）→ Gateway 返回 `{"ok":false,"error":"invalid session"}`。  
   这是 **Worker→Gateway 的 JWT**，与号池里 ChatGPT `access_token` 无关。
2. **历史 JSON 快照滞后**（已移除）：曾 3/40 账号 token 与 sqlite 不一致；现 8012 与 TNexus 直读同一 db。
3. **`pin_account.json` 过期**：`alexnnnmmm@proton.me` 的 pin token 与 db/pool **均不一致**（gateway 部分路径仍读 pin）。
4. **历史误判**：将 `invalid session` 写成「号池 session 过期」是错误的。

### 修复后验证（2026-07-31 19:44 CST）

```bash
python3 /root/gptimage-gateway-rs/scripts/panda_setup_tnexus_env.py  # 刷新 JWT
bash deploy/panda/deploy.sh      # 重建 worker / api / gateway
```

| 检查 | 结果 |
|------|------|
| `UPSTREAM_API_KEY` 未过期 | ✅ |
| pool vs sqlite token 不一致数 | **0/40**（共享 db，无快照） |
| 全链路 job `status=done` | ✅（preview 为 `data:image/png;base64,...`） |
| `job_results.inline_preview_b64` | ✅ 有值（≈1.5MB），`source_url` 空 |

### 运维建议（P1）

| 项 | 建议 |
|----|------|
| JWT 续期 | cron 每日 `panda_setup_tnexus_env.py` + `deploy.sh`，或 gateway 改 `apikey` 模式 |
| 号池 | 确认 `ACCOUNTS_DB=/gptimage/data/accounts.db`；删除旧 `accounts_pool.json` |
| pin 同步 | refresh 后更新 `/root/gptimage-gateway-rs/secrets/pin_account.json` |
| 冒烟脚本 | `prod_url_chain_test.py` 需接受 `data:` preview（持久化后不再强制 gateway URL） |

---

## 仍缺 —— 管理面

| # | 能力 | 状态 | 优先级 |
|---|------|------|--------|
| 1 | 养号/预热结果自动 merge 回 TNexus JSON | ⚠️ 部分（refresh 有，预热/养号待加强） | P1 |
| 2 | 号池 Outlook 恢复 UI（进度条/自动恢复卡片） | ❌ API 已有 | P1 |
| 3 | `image_inflight` 与 gateway 双写一致性 | ✅ 同库 sqlite 事务更新 | — |
| 4 | Outlook 凭据 / YUMAIL 生产配置文档 | ⚠️ 依赖 Panda secrets | P2 |
| 5 | Panda sync / backup / CPA | ❌（明确不做或低优） | — |
| 6 | settings 七卡片 / 完整 ops-dashboard | ❌（明确不做） | — |
| 7 | 账号 Postgres 持久化 | ❌ 仍 sqlite 共享文件 | P2-D |

**管理面可交付缺口**：约 **8%**（主要为 UI 与状态回写）。

---

## 仍缺 —— Gateway / 数据面

| 模块 | 状态 |
|------|------|
| `dispatch_gate` / lease_pool / interval 门 | ❌ |
| `ticket_pool` 接入 upstream | ❌ 冻结 |
| `POST /v1/images/edits` | ✅ upstream（`1ab5d25`）；mask/multipart 待补 |
| `n>1`、`quality`、duplicate_prompt | ❌ |
| OpenAI 标准 error `type` 分档 | ⚠️ 部分 |
| humanlike 调度 / workload 策略 | ❌ |
| `image_task_service` 同级队列背压 | ⚠️ TNexus worker 子集 |

**数据面功能加权**：约 **48%**（edits + 元数据落库 +5%）。

---

## 部署与验收（Panda）

### 环境变量（`/opt/tnexus/.env`）

```bash
# 必配
ACCOUNTS_DB=/gptimage/data/accounts.db
SCHEDULING_STATE_FILE=/data/pool/scheduling_state.json
ACCOUNT_OPS_BASE=http://127.0.0.1:9011
ACCOUNT_OPS_TOKEN=<随机>
GPTIMAGE_ROOT=/gptimage          # account-ops 只读挂载

# 窗口预热回退（account-ops 不可用时）
GATEWAY_BASE=http://127.0.0.1:8014
GATEWAY_AUTH_KEY=<gateway JWT 或 API key>
QUOTA_PRIME_PROMPT=a tiny red dot on white background

# Gateway 容器/进程（与 TNexus 镜像分离）
IMAGE_ENABLED=1
# api + gateway 需挂载同一 /root/gptimage/data → /gptimage/data
```

**禁止**：`GPTIMAGE_ADMIN_TOKEN` 指向生产 `:8012`。

### 部署命令

```bash
export TNEXUS_ROOT=/root/TNexus
cd "$TNEXUS_ROOT" && git pull
bash deploy/panda/deploy.sh
# gateway :8014：docker compose -f deploy/panda/gateway-compose.yml up -d --force-recreate
```

### 验收清单

```bash
# 健康
curl -fsS http://127.0.0.1:9000/health
curl -fsS http://127.0.0.1:9011/health

# 登录后（cookie）
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts?limit=1'
curl -fsS -b cj 'https://tnexus.relai.asia/api/ops/nurture/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/outlook-recovery/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/quota-window/prime/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/schedulable-breakdown'

# 静态页
curl -fsS -o /dev/null -w '%{http_code}\n' https://tnexus.relai.asia/accounts
curl -fsS -o /dev/null -w '%{http_code}\n' https://tnexus.relai.asia/image-manager

# 全链路（可选）
python3 /root/TNexus/scripts/prod_url_chain_test.py
```

### UI 验收要点

1. **号池** — 各列表头点击排序；慢刷面板；行内预热/对话/软封  
2. **图片管理** — 小卡片、懒加载；点击灯箱滚轮缩放/拖拽  
3. **运维** — 养号 Tab（需 account-ops + GPTIMAGE_ROOT）；无配置时应显示不可用而非连生产  
4. **Outlook** — API 可通；UI 卡片待下一版  

---

## 架构示意

```text
浏览器 → tnexus.relai.asia (nginx)
           ├─ /api/*     → tnexus-api :9000  (JSON 号池 + 委托 account-ops)
           ├─ /v1/*      → tnexus-gateway :8014  (生图/对话 + scheduling_gate)
           └─ /*         → 静态 UI

chatgpt2api-local :8012 → live accounts.db（生产 gpt-image）

tnexus-api :9000 + tnexus-gateway :8014 → 同一 accounts.db（`tnexus-accounts-db`，WAL）

account-ops :9011 → GPTIMAGE_ROOT Python 库

禁止 → TNexus 运行时 HTTP 调生产 :8012 管理 API
```

---

## 验收部署记录

| 时间 | commit | 结果 |
|------|--------|------|
| 2026-07-31 15:25 CST | `dd4b758` | 去代理化 + account-ops 桥接 |
| 2026-07-31 17:35 CST | `863fe28` | 顶栏原生导航 |
| 2026-07-31 17:35 CST | `a54d057` | 生图持久化 `inline_preview_b64` |
| 2026-07-31 18:03 CST | `9dcb82a` | 图片管理优先内联 b64 |
| 2026-07-31 20:xx CST | — | 删除 `accounts_pool.json` 快照；8012 与 TNexus 共享 `accounts.db` |

**探测（Panda 回环，2026-07-31 19:44）**

| 检查项 | 结果 |
|--------|------|
| `GET :9000/health` | `{"status":"ok","static_ui":true}` |
| `GET :8014/health` | `tnexus-gateway`，40 accounts，`DATA_PLANE=upstream` |
| `GET :8012/health?format=json` | 40 账号可调度（生产，独立进程） |
| pool token vs sqlite | **N/A**（同一 db，无快照） |
| TNexus job 生图 | `done` + `inline_preview_b64` |

**历史误判（已更正）**：`401 invalid session` 曾为 **Gateway JWT 过期**，非号池 ChatGPT token 过期。
