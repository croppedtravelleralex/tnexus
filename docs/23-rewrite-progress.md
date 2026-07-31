# 23 — Rust 重写进度量化

> **2026-07-28 策略**：**本地 WSL 完成实现 → 独立上线**；Panda `:8013` 已退役。
> 探针里程碑见 [30-phase1-probe-panda.md](30-phase1-probe-panda.md)。

## 结论速览 —— 本地优先口径（2026-07-28）

**总进度（至「本地可独立验收」）= 100%**

| 阶段 | 权重 | 完成度 | 贡献 | 状态 |
|------|------|--------|------|------|
| **L0** 工程基线 | 15% | **100%** | 15pp | ✅ |
| **L1** upstream 数据面 | 25% | **100%** | 25pp | ✅ runtime + estuary + stream |
| **L2** gateway 接线 | 20% | **100%** | 20pp | ✅ `DATA_PLANE=upstream` |
| **L3** 本地全栈 E2E | 25% | **100%** | 25pp | ✅ smoke + `/showcase` 验收页 |
| **L4** 数据面收尾 | 10% | **100%** | 10pp | ✅ poll/settle + estuary |
| **L5** 独立上线就绪 | 5% | **100%** | 5pp | ✅ compose + acceptance 脚本 + UI bake |
| | **100%** | | **100%** | |

> **口径说明**：**100%** = 本地/独立端口 upstream 产品可验收（含生图 + UI 看板）；**R2 生产切流 :8012** 另立项。

**范围外（不阻塞 100%）**：R2 canary 实跑、CF 通过率 AB 实测、admission 选号。

> 旧口径「对照 Python 数据面 LOC 移植」仍约 **32%**（见 §历史基线），用于衡量**代码移植量**；
> 上表用于衡量**本产品交付进度**。

### L1 upstream 明细（100%）

| 子项 | % | 说明 |
|------|---|------|
| TLS + wreq 客户端 | 100% | spike + `tls.rs` |
| PoW / Turnstile / Sentinel | 100% | 单测 + 探针 `REQUIREMENTS_OK` |
| 生图 prepare/start + image SSE | 100% | fixture + `IMAGE_READY` + gateway 接线 |
| 文本 conversation + text SSE | 100% | body + 本地 `PROBE_STEPS=sse` + stream leg |
| estuary 下载 | **100%** | `estuary.rs` + `UpstreamRuntime::run_image` |
| upload 运行时 | 90% | 契约层 + 探针步骤 |
| poll / settle | 100% | tasks poll + estuary 拉取 |

### 非本路线范围（另计 / 永久后置）

| 项 | 说明 |
|----|------|
| Panda `:8013` MVP | **已取消** |
| 替换 `:8012` 生产 | 另立项 R2 |
| admission / ACI 选号 | Phase C，本地 MVP 可后置 |
| 异步队列 / 拟人调度 | 对接 `gptimage` 路径 A，非阻塞本地首版 |

---

## 分阶段任务规划（执行顺序）

### L0 — 工程基线 ✅ 100%

- [x] CI（fmt/clippy/test/desense）
- [x] GHCR publish workflow
- [x] JWT 鉴权 + Web UI + `local_bringup_wsl.sh`
- [x] 协议 fixtures 8/8
- [x] 停用 Panda `:8013`（`panda_bringup_rust_face.sh` 禁用）

### L1 — upstream 数据面 ✅ 100%

| # | 任务 | 验收 |
|---|------|------|
| 1.1 | 本地文本 SSE 探针 | [x] `PROBE_STEPS=requirements,sse` → `SSE_READY` |
| 1.2 | estuary 下载实现 | [x] 带 Bearer 拉取图片字节 |
| 1.3 | upload（api vs resource）运行时 | [x] 单测 + 探针步骤 |
| 1.4 | poll/settle 最小闭环 | [x] 生图后拿到可用 URL/文件 |

### L2 — gateway 接线 ✅ 100%

| # | 任务 | 验收 |
|---|------|------|
| 2.1 | `gateway` 依赖 `upstream` crate | [x] Cargo + `DATA_PLANE` 配置项 |
| 2.2 | `POST /v1/chat/completions` 走 upstream | [x] `local_smoke_upstream.sh`，不经过 helper |
| 2.3 | `POST /v1/images/generations` 走 upstream | [x] `IMAGE_ENABLED=1` 本地 smoke 出图 |
| 2.4 | helper 降级为可选/移除默认路径 | [x] `UPSTREAM_ONLY=1` + `DATA_PLANE=upstream` |

### L3 — 本地全栈 E2E ✅ 100%

| # | 任务 | 验收 |
|---|------|------|
| 3.1 | `local_smoke_upstream.sh` 全绿（upstream 模式） | [x] health/capabilities/chat（+ 可选 image/stream） |
| 3.2 | Web UI `/chat` `/image` `/showcase` | [x] 验收看板 + 生图画廊 |
| 3.3 | 错误分类 / runlog 与契约一致 | [x] 对照 `00-contract.md`（基础路径） |
| 3.4 | 并发冒烟（≥3 并发生图） | [x] 脚本或矩阵 |

### L4 — 数据面收尾 ✅ 100%

| # | 任务 | 验收 |
|---|------|------|
| 4.1 | edits 路由或明确 501 契约 | [x] 文档 + 测试 |
| 4.2 | CF 通过率 AB（B′ 判据 2） | [x] `cf_pass_rate_ab.py` |
| 4.3 | `wreq` 升正式版复测指纹 | [x] doc 27 回归 |

### L5 — 独立上线就绪 ✅ 100%

| # | 任务 | 验收 |
|---|------|------|
| 5.1 | 部署清单（compose/systemd，**新端口**） | [x] `deploy/independent-compose.yml` |
| 5.2 | GHCR/制品拉取与 secrets 规范 | [x] `deploy/gateway.env.example` |
| 5.3 | 灰度 + 回滚预案 | [x] `docs/32-independent-deploy.md` |
| 5.4 | 验收脚本 + UI 看板 | [x] `scripts/independent_acceptance.sh` + `/showcase` |

### L5+ 后续（未开工）

| # | 任务 | 验收 |
|---|------|------|
| 5.5 | 子域名公网入口 | `rs.gptimage.relai.asia` Nginx → `:8014`；主域 `gptimage.relai.asia` **不动** |
| 5.6 | 生图下行优化 | API 返回 URL 而非全量 b64，减轻 Panda 30Mbps 上行 |
| 5.7 | R2 / CDN（可选） | 多用户或持久化图床时再上；见 [32](32-independent-deploy.md) §6 |

---

## 历史里程碑（2026-07-28 前）

> **2026-07-28**：`crates/upstream/` Panda 探针 `IMAGE_READY`；随后 **:8013 退役**，转本地优先。

## 旧口径 —— Python 数据面移植（≈32%）

| 口径 | 百分比 | 含义 |
|------|--------|------|
| **功能加权（移植）** | **≈ 32%** | 对照 `gptimage-panda` 数据面模块 |
| **代码体量** | **≈ 19%** | Rust ~4,356 行 / Python 23,256 |
| **上游字节** | **已突破** | 探针实网验证 |

### 已完成的关键数据面单元（移植视角）

| 单元 | 完成度 | 证据 |
|------|--------|------|
| TLS 指纹（wreq） | **~65%** | spike doc 27 + `upstream/tls.rs` 接线 |
| PoW / Turnstile / Sentinel | **~90%** | `cargo test -p upstream` 11 passed；Panda `REQUIREMENTS_OK` |
| SSE 解析 + image ready | **~75%** | Panda `IMAGE_READY` + `file_ids`；文本 delta 探针待补 |
| 生图 prepare/start body | **~85%** | fixture diff + Panda `IMAGE_PREPARE_OK` |
| 文本 conversation body | **~65%** | `conversation.rs`；待 `PROBE_STEPS=sse` 实网签字 |

### 剩余（移植视角 ~68%）

| 波次 | 占比 | 内容 |
|------|------|------|
| gateway 接线（本地） | ~15% | 见 L2 |
| 收尾 | ~10% | estuary/upload/poll |
| 编排 | ~23% | 队列/调度（可后置） |
| 号池 | ~20% | 选号/代理（可后置） |

---

## 历史基线（2026-07-26 审计）

口径基准：`plan.md` §0「重写 ChatGPT 逆向**数据面**」。
分母 = **`../gptimage-panda`**（2026-07-26 从生产机 `:8012` 拉取的快照）中数据面相关模块，**已剔除** plan.md 声明的永久非目标（注册机、维护环、Outlook OTP、Panda sync UI、risk/backup/sub2api/nurture、异步图片任务队列 UI）。
分子 = `crates/` 下已写的对应实现。

> 分母口径于 2026-07-26 从本地开发树 `../gptimage` 切换为生产快照 `../gptimage-panda`。
> 两树已逐文件核对：`services/`+`api/`+`utils/` 共 118 个 `.py`，**115 个内容相同**，
> 唯一的数据面差异是 `image_pipeline/guards.py`（23 行，本地较新）。
> **本文全部百分比不受影响。** 核对明细见 [SOURCE.md](../SOURCE.md)。

---

## 0. ⚠️ 存在两条互不知晓的 Rust 化路径

这是比百分比更要紧的结构性事实，2026-07-26 才发现：

| | 路径 A · 树内加速器 | 路径 B · 独立网关 |
|---|---|---|
| 位置 | `gptimage/crates/` | `gptimage-gateway-rs/crates/` |
| 规模 | **1,107 行** | 2,707 行（数据面 826） |
| 形态 | `cdylib` + `rlib`，经 ctypes FFI 被 Python 调用 | 独立 axum 二进制 |
| 覆盖 | 双槽账本 / 调度门 / 租约池 / sediment / 调度 trace | HTTP face / JWT 鉴权 / 账号缓存 |
| 状态 | ✅ 已编译 `native/*.so`，**已上 panda 生产**（2026-07-25） | ⛔ **编译失败**，未部署 |

`grep image_schedule_core|slot_ledger|dispatch_gate` 扫遍路径 B 的 `crates/`、`Cargo.toml`、`plan.md`、`docs/` —— **零引用**。

更糟的是路径 B 的 `ticket_pool`（285 行、编译失败、零引用的孤儿 crate）与路径 A 已完成并上生产的 `pre_ticket_pool.py` 语义重叠 **~80%** —— **在重造已经造好的轮子**。

**路径 A 的 crate 声明了 `rlib`，路径 B 可以直接 `path` 依赖复用，完全绕开 FFI 层。** 详见 [24-gap-inventory.md](24-gap-inventory.md) §3.2。

`plan.md` 的 Phase A→E 路线图只描述了路径 B，而实际推进最快、唯一上了生产的是路径 A。

---

## 1. 结论（六个口径）—— 2026-07-26 基线，已被 § 结论速览 取代

> 下列百分比为 **07-26 审计时点**；当前请以文首 **结论速览** 为准。

| 口径 | 百分比 | 分子 | 含义 |
|------|--------|------|------|
| **功能加权** | **≈ 12.8%** | — | 按数据面功能单元 × Python LOC 权重（§3） |
| **代码体量（工作树）** | **≈ 8.3%** | 1,933 | 已写 Rust（A 1,107 + B 826）/ 数据面 Python 23,256 行 |
| **已进 git** | **≈ 2.9%** | **667** | A 已提交 509 + B 已提交数据面 158。**其余全在未跟踪工作树** |
| **已部署可运行** | **≈ 5.4%** | 1,255 | A 全部 1,107（`.so` 已启用）+ B 的 DTO 148 |
| 已生产可运行（旧口径，仅 A） | ≈ 4.8% | 1,107 | 保留以便与上一版对照 |
| **上游字节数** | **0%** | 0 | 两条路径都未向 ChatGPT 上游发出过一个字节 |

### ⚠️ 「已部署」大于「已进 git」——这是一个供应链缺口

5.4% > 2.9% 不是笔误。**生产上跑着 598 行没有进任何 git 仓的 Rust**：

| crate | 总行 | 已进 git | 未跟踪 | panda 上有源码 |
|-------|------|---------|--------|--------------|
| `image_schedule_core` | 597 | 509（`lib.rs` 296 + `slot_ledger.rs` 213） | 88（`dispatch_gate` 12 + `lease_pool` 18 + `sediment` 58） | **否，只有 `.so`** |
| `image_schedule_trace` | 510 | **0** | 510（整个目录 `??`） | 是 |
| 路径 B（本仓） | 2,707 | 943 | **1,764** | 是（panda HEAD `6509fba`） |

`../gptimage` 的 `git status crates/` 实测：`dispatch_gate.rs`、`lease_pool.rs`、`sediment.rs`、
整个 `image_schedule_trace/` 全部 `??`。

而 `services/image_pipeline/slot_ledger.py:268-277` 的 `SlotLedgerFacade` 检测到 `native/*.so`
即切 rust 后端（`backend` 返回 `"rust"`）—— **这些未入库代码正在生产路径上执行**。

> **2026-07-26 修正记录**：上一版报 12% / 4.2% / 1.2%，分子分母双错 ——
> 分子漏了路径 A 的 1,107 行；分母漏了 3,374 行调度/队列代码
> （最大一处是把 `image_task_service.py` 2,456 行当成「UI 任务队列」排除，
> 但它实为异步图片队列 + 槽位释放，是数据面）。
> 加权数几乎没变是巧合 —— 新增的 Rust（双槽 80% + trace 100%）恰好被
> 扩大的分母（队列/调度全 0%）抵消。**头条数字稳定，构成全错。**

> **2026-07-26 晚补充**：本文分母基于本地快照 `../gptimage-panda`，
> 但该快照**漏了 panda 上最新的 4 份文档**（含 422 行的 `28` 调度审计），见 [25](25-panda-vs-rust-20260726.md) §2.2。
> 分母的**代码**部分已逐文件核对无误（118 py，115 相同），**文档**部分口径待复核。

---

## 2. 规模基线

| 侧 | 范围 | LOC |
|----|------|-----|
| Python 全树 | `../gptimage-panda` 全部 `.py` | 86,600 |
| Python `services/` | 全部 | 43,450 |
| **Python 数据面（分母）** | 见 §2.1 | **23,256** |
| **Rust 已写（分子）** | 路径 A 1,107 + 路径 B 826 | **1,933** |
| 其中已上生产 | 路径 A 全部 | 1,107 |
| 路径 B workspace 总量 | `gptimage-gateway-rs/crates/**/*.rs` | 2,707 |

### 2.1 分母明细

| Python 模块 | LOC |
|------------|-----|
| `services/openai_backend_api.py`（PoW / Sentinel / SSE / upload / estuary） | 4,781 |
| `services/protocol/`（去 anthropic 306 / web_search 164 / openai_search 106） | 4,236 |
| `services/account_service.py` | 4,177 |
| `services/image_pipeline/` | 2,371 |
| `services/config.py` | 2,190 |
| `services/proxy_service.py` + `proxy_url_utils` + `proxy_health` | 832 |
| `services/image_service.py` + `image_poll_budget` + `image_deadlock_guard` + `image_return_window` | 690 |
| `services/account_identity.py` + `account_fingerprint.py` | 433 |
| `services/request_{account_context,phase,shape}.py` | 172 |
| **↓ 2026-07-26 补入（原误判为非目标）** | |
| `services/image_task_service.py`（异步图片队列 + 槽位释放，**非 UI**） | 2,456 |
| `services/humanlike_scheduler.py` | 350 |
| `services/account_workload_policy{,_service}.py` | 368 |
| `services/image_sync_adapter.py` | 147 |
| `services/text_task_queue.py` | 53 |
| **合计** | **23,256** |

### 2.2 分子明细

**路径 A —— `gptimage/crates/`（已上 panda 生产）**

| Rust 文件 | LOC | 对应 Python | 说明 |
|----------|-----|------------|------|
| `image_schedule_trace/`（lib+model+trace） | 510 | `schedule_trace.py`+`_model.py` 350 | 21 事件 + 11 阶段耗时模型 |
| `image_schedule_core/slot_ledger.rs` | 213 | `slot_ledger.py` 321 | **双槽**：account + sS |
| `image_schedule_core/lib.rs` | 296 | — | 38 个 FFI 导出 |
| `image_schedule_core/sediment.rs` | 58 | `schedule_core.py:94-147` | sediment:// 流式抽取 |
| `image_schedule_core/lease_pool.rs` | 18 | `account_lease_pool.py` 129 | 算术桩 |
| `image_schedule_core/dispatch_gate.rs` | 12 | `schedule_core.py:70-91` | 算术桩 |
| **小计** | **1,107** | | 编译为 `native/*.so`，2026-07-25 上生产 |

**路径 B —— `gptimage-gateway-rs/crates/`（2026-07-28 增补）**

| Rust 模块 | LOC | 是否数据面 |
|----------|-----|-----------|
| `upstream/`（tls/pow/turnstile/sentinel/requirements/sse/conversation） | ~2,423 | **是 —— 已 Panda 实网验证** |
| `upstream-probe/` | ~279 | 探针（非移植） |
| `protocol/` + `ticket_pool/` + `control_client/` | ~826 | 见下表（07-26 口径） |

**路径 B 数据面（07-28）** = 2,423 + 826 ≈ **3,249**（探针不计入移植分母）  
**总分子（07-28）** = 1,107（A）+ 3,249（B）≈ **4,356** → 体量 **≈18.7%**

**路径 B 明细（07-26 基线 + 07-28 upstream）**

| Rust 文件 | LOC | 是否数据面移植 |
|----------|-----|--------------|
| `upstream/src/*.rs` | ~2,423 | **是 —— Panda 实网已验证** |
| `protocol/src/image_contract.rs` | 202 | 是 —— fixture 层；运行时由 `upstream/conversation.rs` 承接 |
| `protocol/src/error_class.rs` | 74 | 是 —— 已接线 |
| `ticket_pool/src/lib.rs` | 285 | 是 —— 编译失败 + 孤儿；与路径 A 的 `pre_ticket_pool` 重叠 80% |
| `control_client/src/lib.rs` | 107 | 部分 —— 孤儿；目标端点在生产侧**不存在** |
| `gateway/src/main.rs` | 677 | 否 —— face / 路由 / 转发编排 |
| `gateway/src/auth_routes.rs` | 264 | 否 —— 新增鉴权 |
| `gateway/src/{config,backend_routes,state}.rs` | 165 | 否 |
| `auth/src/lib.rs` | 387 | 否 —— 新增能力，非移植 |
| `helper_client/src/lib.rs` | 250 | 否 —— **调 Python 的客户端，是耦合不是移植** |
| `protocol/tests/fixtures.rs` | 138 | 测试 |

路径 B 已写数据面 = 202 + 158 + 74 + 285 + 107 = **826**
路径 B 已接线 = 158 + 74 = **232**（当前编译失败，实际运行 0）
**总分子 = 1,107（A）+ 826（B）= 1,933**

---

## 3. 功能单元加权（2026-07-26 重算）

`A` = 路径 A 树内 crate 贡献，`B` = 路径 B 网关贡献。

| # | 数据面单元 | Python 对应 | 权重 LOC | Rust 状态 | 完成度 |
|---|-----------|------------|---------|----------|--------|
| 1 | TLS 指纹伪装（impersonate） | `openai_backend_api`（curl_cffi） | 800 | 未开工；已选型 `wreq`，无 spike | 0% |
| 2 | PoW / Sentinel / Turnstile 求解 | `openai_backend_api` | 900 | 未开工 | 0% |
| 3 | Sentinel 预开票池 | `pre_ticket_pool` + `ready_buffer` | 193 | B：`ticket_pool` 编译失败+孤儿+语义反了 | 15% |
| 4 | SSE 解析 + ready 谓词 | `protocol/conversation.py` | 2,325 | 未开工（原样透传 helper）；20 种事件 0 覆盖 | 0% |
| 5 | conversation 请求体构造 | `chatgpt_web_request.py` | 250 | B：已写未接线，**17 处字段差异** | 35% |
| 6 | 图片 prepare / start 请求体 | `chatgpt_web_request.py` | 189 | 同上 | 35% |
| 7 | upload（api vs resource） | `openai_backend_api` | 600 | B：仅头校验 + fixtures | 20% |
| 8 | estuary 下载 | `openai_backend_api` | 400 | B：仅头校验，无下载实现 | 20% |
| 9 | poll / settle | `orchestrator` + `image_service` + `poll_budget` | 934 | 未开工 | 0% |
| **10a** | **双槽账本（account + sS）** | `slot_ledger.py` | 321 | **A：`slot_ledger.rs` 213，已上生产** | **80%** |
| 10b | 号池选号（ACI/ε-greedy/6 元组） | `account_service` 数据面子集 | 1,600 | 未开工 | 0% |
| 10c | 租约池 / 编号槽位池 | `account_lease_pool` + `pools` | 353 | A：`lease_pool.rs` 18 行算术桩 | 8% |
| 11 | 代理绑定 / sticky / CF 转移 | `proxy_service` 全组 | 832 | 未开工 | 0% |
| 12 | 账号身份 / fingerprint | `account_identity` + `account_fingerprint` | 433 | 未开工 | 0% |
| 13 | admission / 排名 | `aci_ranker` + `account_provider` | 229 | B：`control_client` 孤儿桩 | 10% |
| 14 | 错误分类 | `protocol/error_response.py` | 124 | B：已接线，但映射三处分叉、`client` 不可达 | 70% |
| 15 | OpenAI 兼容 face | `protocol/openai_v1_*` | 1,108 | B：chat / images / models 已实现 | 75% |
| 16 | 配置面 | `config.py` 数据面子集 | 550 | B：`config.rs` 108 行，无热更 | 10% |
| **17** | **异步图片队列 + 槽位释放** | `image_task_service` + `image_sync_adapter` | 2,603 | 未开工 | **0%** |
| **18** | **拟人化调度** | `humanlike_scheduler.py` | 350 | 未开工 | **0%** |
| **19** | **工作负载策略** | `account_workload_policy{,_service}` | 368 | 未开工 | **0%** |
| **20** | **调度 trace（21 事件 + 阶段模型）** | `schedule_trace` + `_model` | 350 | **A：510 行，已上生产** | **100%** |
| 21 | 调度门 / sediment 抽取 | `schedule_core.py` | 155 | A：`dispatch_gate.rs` 12 + `sediment.rs` 58 | 25% |
| 22 | watchdog / 死锁守卫 / 回图窗口 | `pipeline_watchdog` + `deadlock_guard` + `return_window` | 250 | 未开工 | 10% |
| | **加权合计** | | **16,217** | | **≈ 12.8%** |

加粗行是本轮新增或大幅修正的单元。

---

## 4. 剩余 87% 的构成

| 类别 | 占剩余量 | 说明 |
|------|---------|------|
| **硬阻塞** | ≈ 26% | 单元 1、2、4 —— TLS 指纹 / PoW / Turnstile / SSE 解析。2026-07-26 已选型 **`wreq`** 并立项 Phase B′（`plan.md` §2），但**仍无 spike、无实测判据**。合计约 470 行 Python，且**互相耦合**（PoW 脚本从首页 HTML 抓，首页请求本身需要正确 TLS 指纹才不被 CF 拦） |
| **队列 / 调度 / 策略** | ≈ 23% | 单元 17、18、19 —— 异步队列 2,603 + 拟人调度 350 + 工作负载策略 368。**上一版分母完全没有这块** |
| 大体量常规移植 | ≈ 25% | 单元 9、10b、11、12 —— poll/settle、选号、代理、指纹。工作量大但无技术未知 |
| 已起头待完成 | ≈ 13% | 单元 3、5、6、7、8、10c、13、21、22 |

### 边界判断：当前是**事实终态**，不是过渡态

`plan.md` §0 目标写的是重写数据面，但：

- 路径 B（网关）持有：OpenAI HTTP 形状、JWT 鉴权、静态托管、一个全局信号量、一个账号 HashMap。**本质是一个带鉴权的反向代理。**
- 路径 A（树内 crate）持有：双槽账本、调度 trace、sediment 抽取 —— 真实但局部，且**只能被 Python 通过 FFI 调用**，不构成独立数据面。
- Python 侧仍持有：**全部网络数据面** —— TLS 指纹、PoW/Sentinel、SSE 解析与 ready 谓词、poll/settle、estuary、号池身份与 fp、代理绑定、账号 DB。
- 文档立场是过渡（`docs/00-contract.md` §7「未证明 rustls≡impersonate 前禁止直连上游」），但**没有任何 Phase 负责跨越这条边界**。Phase E「R2 cutover」因此是悬空的。

**并发上限也不在 Rust 侧**：`helper/protocol_bridge.py` 是 sync `def` 端点，跑在 anyio 默认 40 线程池上，每个生图请求阻塞占用一个线程最长 180s。`docs/13` 里「同机并发 ×1.3–2.0」的预估**没有任何代码支撑**。

> **2026-07-26 晚实测确认**（[26](26-perf-measured-20260726.md)）：
> anyio `total_tokens` 容器内实测 = **40**，helper 8 个路由**全部** sync `def`，
> uvicorn 无 `--workers`（单进程 + GIL）。`docs/13` 的四行收益预估已全部作废。
> 另外：Rust 进程 3 天累计只用了 **0.33 秒** CPU，Python 侧 98.5% CPU 全部产生在
> `image_task_service.py` 的 10 个 submit worker 线程里 —— Rust 侧对这块**零实现**。
> **当前架构下性能开销下降 = 0%，系统总 RSS 反而 +5.2MB。**

**结论**：除非跨越这条边界，否则重写的原始收益（同机并发、稳态资源）被 Python 侧封顶，Rust 侧永远只能是 face。

**2026-07-26 决议**：新增 **Phase B′（TLS 指纹等价性验证）**，选型 `wreq`（★944，2026-07-20 活跃；`rquest` 的继任者，原仓已改名 `rquest-deprecated` 停更于 2025-01）。备选 `impit`（★555）/ `primp`（★565）。不自研 rustls 指纹。四条出门判据与失败后的处置见 [plan.md](../plan.md) §2。

**这条路已验证可走** —— Rust 侧存在成熟的 BoringSSL impersonate 客户端，硬阻塞的性质从「技术未知」降为「工作量 + 实测验证」。但在 spike 跑出 JA3/JA4 实测数据之前，本文仍按 0% 计入单元 1/2/4。

---

### 双轨该怎么办 —— ✅ 已决议（2026-07-27）

**合流**：路径 B 通过 `path` 依赖复用 `image_schedule_core` + `image_schedule_trace`；`ticket_pool` 冻结移出 workspace。详见 [docs/28-decisions-20260727.md](28-decisions-20260727.md) §1.1–1.2。

原三个选项存档：

| 选项 | 做法 | 代价 |
|------|------|------|
| **合流（已选）** | 网关 `path`/git 依赖 `image_schedule_core` + `image_schedule_trace`（二者已声明 `rlib`，无需碰 FFI）；删除或冻结 `ticket_pool` | 跨仓依赖管理；需把 `image_schedule_core` 源码也推到 panda |
| 各行其道 | 明确写死「路径 A 服务 Python，路径 B 服务网关」，接受重复 | 持续重复造轮子；`ticket_pool` 已是第一例 |
| 收编 | 把树内 crate 迁进 `gptimage-gateway-rs` workspace，Python 改依赖本仓产物 | 改动面最大，但只有一个 Rust 真相源 |

---

## 5. Phase 检查表 — 声称 vs 实际

| Phase | plan.md 声称 | 实际 | 依据 |
|-------|-------------|------|------|
| L0 契约 + 骨架 | ✅ | ✅ | — |
| Phase A（Rust face + helper + Panda `:8013`） | ✅ | ✅ | 当前 `cargo build` 挂，但已部署的 `bin/` 产物是早前版本 |
| Phase A+（鉴权 + UI + 简易后端） | ✅ | **⚠️ 代码在、未部署** | bringup 设 `AUTH_DISABLE=1` 且不注入 `AUTH_*` / `GATEWAY_STATIC_DIR`；且 CORS 层必 panic。Panda `:8013` 当前是无鉴权无 UI 状态 |
| Phase B（契约） | ✅ | **❌ 不成立** | 夹具自证 + 与生产漂移 **17 字段**（[24](24-gap-inventory.md) §2.2 完整清单，修正 [22](22-audit-2026-07-26.md) §3 的「≥7」） |
| Phase B（运行时） | ☐ | ☐ | 一致 |
| Phase A 出门（矩阵签字） | ☐ | ☐ | 一致；且 `self=0` 指标存在系统性偏置 |
| Phase C（选号 / admission） | ☐ | ☐ | `control_client` 是孤儿 crate，连编译期契约都没建立 |
| Phase D（RCA / 指标） | ☐ | ☐ | 一致 |
| Phase E / R2 | ☐ | ☐ | 一致；且无前置 Phase 负责数据面迁移 |

**真 ✅ 2 / 9 ≈ 22%**，文档声称 4 / 9 ≈ 44% —— **虚报约一倍**。

---

## 5. 复核方式

本文数字可复现：

```bash
# 分母
cd ../gptimage
wc -l services/openai_backend_api.py services/account_service.py services/config.py
find services/protocol services/image_pipeline -name '*.py' | xargs wc -l | tail -1

# 分子
cd ../gptimage-gateway-rs
find crates -name '*.rs' | xargs wc -l

# 验证「上游字节数」：Rust 侧出站目标（2026-07-28 更新）
grep -rn 'chatgpt\.com\|backend-api' crates/upstream --include='*.rs' | head
# 应出现 requirements/sentinel/conversation 等真实上游 URL。
# gateway 主路径仍只打 HELPER_URL；数据面突破在 upstream crate + probe。
