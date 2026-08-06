# 39f — Grok 移植进度记录（做了的 / 未做的 / 要做的）

## 2026-08-06 收尾批次（生产判定后补完）

- **部署草案 Blocker B1-B4 全修**（`ee30d55`）：redis 6380/postgres 5433 对齐主栈、runtime 加 curl、
  移除不存在的独立 grok-admin 服务、env 必填占位全补、rollback 自动读 last-deploy tag + readyz
- **DB 对齐**（`ee30d55`）：migrations/019（web_tier 列 + quota_recovery status 放开 active + tier 索引）、
  storage 读入 created_at/updated_at/last_used_at/web_tier
- **admin PG 数据面**（`ee30d55`）：grok2api-rs/pg_admin.rs——PgAdminStore/PgAuthRepo/PgSessionRepo
  （grok_admins/grok_admin_sessions 表），无 DB 降级内存
- **CI 工具链**（`ee30d55`）：rust-toolchain.toml 1.97 + workflow pin 1.97.1；grok-gate 15 crate 全含
- **grok-admin 端点补全**（`9cb872c`）：+5 端点（media get/size-summary、system config/logs、
  models aliases/sync-state）+ 系统环形日志 + 配置布尔视图，15 测试
- **生图接线**（`9cb872c`）：GROK_IMAGE_ENABLED=1 接真实 ImageEngine（未开 500 明确错误不外呼）
- **后台任务**（`9cb872c`）：GROK_TASKS_ENABLED=1 启动 Build 四池探针（TaskScheduler panic 续跑 +
  Drop abort）；web_quota_refresh/dispatch_probe/pin_sync 待 Go sidecar（TODO 已标注）
- **前端管理页**（`9cb872c`）：/grok/{models,keys,audits,dashboard,settings} 页面 + grok-tabs/token-gate
- **验证**：293 测试 / 48 套件全绿；grok-admin + grok2api-rs clippy 0；tsc + next build 通过

### 剩余未完成（切流不阻塞 / 需外部依赖）

- /v1/media/images/{id} 501、/v1/videos 500：media fetcher 需存储 + 视频需上游轮询（G2/G5 收尾 TODO）
- web_quota_refresh / web_dispatch_probe / pin_sync：需 Go sidecar PG 实现（G6）
- shadow compare 真实数据、探针 24h、G3-A2 dispatch diff<5%（运行类验收，上线后观测）
- docs/ARCHITECTURE.md 新文件未提交（8-05 遗留，非 grok 批次）


最后更新：**2026-08-06 凌晨**（G0–G5 完成；**G6/G7 大部分完成**：UI 页/OCR/deploy 草案/shadow 脚本/nginx 草案；剩余=部署执行 + N5 挂载 + 合并推送）
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md) · 路线图：[39a](39a-grok-roadmap.md) · 执行计划：[39e](39e-grok-execution-plan.md)

---

## 0. 一句话状态

> **G0/G1 已合入 `main`；G2 已提交；G3–G5 全部完成；G6/G7 大部分完成（261 测试 / 47 套件全绿，分支 `feat/grok/g2-image` commits `c954fde`→`578574f`）。剩余：G6-P2 部署执行（Panda 禁区）、G6-P3 shadow 实际运行、N5 gateway 挂载、合并 main + push。**

---

## 1. Git 现状（2026-08-05 夜实测）

| 项 | 值 |
|----|-----|
| 当前分支 | `feat/grok/g2-image`（**后续开发继续在此分支**） |
| 分支 HEAD | `901924c`（G3–G5 全部完成，12 笔 grok 提交 `c954fde`→`901924c`） |
| 分支上已合入 main 的 grok 基线 | G0（`5a8c5d9`，tag `grok-g0`）、G1（`2adbf63`，tag `grok-g1`） |
| 分支夹带的非 grok 提交 | G1 时代的 7 笔 upstream/studio 修复（`bcaefab`…`67d883d`）+ G2 时代的 image/worker 修复（`00814eb`…`9cecec1`）——合并 main 时需确认是否随行 |
| `origin/main` | `d5384e3`（落后：G0/G1 合并未 push） |
| 未提交 | 仅无关 untracked（`.pi-subagents/`、`791682`、`artifacts/`、`scripts/__pycache__/`、`docs/ARCHITECTURE.md`） |
| grok 测试 | **14 crate / 45 套件 / 246 测试全绿**，clippy 0 警告 |

**⚠️ 合并纪律**：G3–G5 尚未 merge main、无 tag `grok-g3`；合并前先处理分支夹带提交（cherry-pick 分离或确认随行），并跑 `./scripts/grok_migration_gate.sh g3`。

## 2. 做了的（已完成）

### 2.1 基线分支（✅ 已合 main）

- `docs/39e-grok-execution-plan.md` 执行计划（分支/合并/subagent/验收）
- 现有 39* 骨架入库：`migrations/010_grok_core.sql`（原 58 行）、`scripts/grok_etl_sqlite_to_pg.py`（原 39 行）、`scripts/grok_migration_gate.sh`（117 行）、`deploy/panda/grok-compose.yml`（草案）
- commit `4249941` → 合入 main（`71e32ae`）

### 2.2 G0 地基（✅ 完成，tag `grok-g0`，已合 main `5a8c5d9`）

| 检查项（39e） | 产出 | 状态 |
|---------------|------|------|
| G0-P1 | `crates/grok-domain`（7 模块：account/audit/chrome_ticket/egress/model_route/pipeline/lib） | ✅ build+test 绿 |
| G0-P2 | `crates/grok-storage`（只读 PG repo：account/credential/quota） | ✅ 编译绿 |
| G0-P6 | `crates/grok2api-rs`（config.Validate + `/healthz` `/readyz` :8000） | ✅ 5 单测过 |
| G0-P3 | `migrations/010`（重写为 Go-parity）+ `011–015`（31 表 / 44 索引 / egress scope CHECK） | ✅ |
| G0-P4 | `scripts/grok_etl_sqlite_to_pg.py` 全量 ETL（schema 驱动 COPY、保留密文、--dry-run/--limit/--schema） | ✅ py_compile + dry-run（74 账号） |
| G0-P5 | `Dockerfile.grok` + `.github/workflows/ghcr-image.yml`（grok2api-rs 镜像 job + grok-gate CI job） | ✅ |
| 门禁 | `./scripts/grok_migration_gate.sh g0` → **PASS** | ✅ 本地 + 合并前复跑 |

**整合时人工修复的遗留问题**（subagent 独立产出时已知风险）：
1. `grok2api-rs` `config.rs` 类型错误（database_url Option/String）→ 修。
2. `010` 凭据列名与 Go `account_credentials` 不一致（`encrypted_access_token/refresh_token` vs `encrypted_primary/encrypted_refresh`）→ DB 子代理重写 010，storage/ETL 三方对齐。
3. 门禁 `cargo fmt --all` 被无关 upstream crate 阻塞 → scoped 到 grok crates。
4. `python3` Windows Store stub 探测失败 → 用 `py` launcher + 真实解释器探测。
5. 门禁 `|| true` 掩盖测试失败 → 去掩（`cargo test -p grok-domain`、`-p grok-storage` 独立执行）。

**G0 独立复审（code-reviewer）**：无 blocker；minor（门禁掩码/提交范围）已处理。

### 2.3 G1 OCR + chat 最小闭环（✅ 实现完成、✅ 测试全绿、❌ 未提交）

> 实现阶段完成；**尚未 git add/commit/merge/tag**。测试计数：**55 个全绿**。

| crate | 模块 | 测试 | 说明 |
|-------|------|------|------|
| `grok-egress` | lease.rs + memory.rs（InMemoryLeaseManager） | 4 ✅ | lease 基础 + `grok_web` scope 并发闸门；Redis 多实例留 G3 |
| `grok-conversation` | normalize.rs + limits.rs + error.rs | 12 ✅ | OpenAI 归一化、contentTextAndImages、8 图/64MiB/file_id 校验（G-OCR-4/5/6） |
| `grok-pool` | lib.rs（SimplifiedPool） | 8 ✅ | 简化单池：pin 优先、冷却、dispatch 记账 |
| `grok-audit` | sink.rs + repo.rs + audit.rs | 4 ✅ | 异步缓冲写入 `grok_request_audits`，DB down 不阻塞 |
| `grok-provider-web` | bridge.rs + attachments.rs + chat.rs + engine.rs + error.rs | 18 ✅ | bridge 客户端、附件下载、**OCR payload golden**、ChatEngine 全链路 |
| `grok-gateway` | handlers.rs + router.rs + error.rs | 9 ✅（ocr_e2e.rs） | `POST /v1/chat/completions`（JSON/SSE）+ `GET /v1/models` |
| golden | `tests/grok_golden/chat_ocr_request.json` + `chat_ocr_upstream_payload.json` | — | OCR 契约（39c §4） |

**OCR 关键验收证据（已核实代码）**：
- `grok-provider-web/src/chat.rs:53`：`enable_image_generation = if ocr { false } else { !attachments.is_empty() }`；model=`grok-chat-fast`、`enableImageStreaming=false`（G-OCR-7/10 锁定）
- `grok-conversation/src/limits.rs`：`MAX_CHAT_IMAGE_ATTACHMENTS=8`、`MAX_TOTAL_IMAGE_BYTES=64<<20`；file_id 明确 400（G-OCR-4/5/6）
- E2E 覆盖：中文单图 200、英文 URL 200、无文字 200、9 图 400、file_id 400、payload golden、stream SSE 完成、空池 503、models 别名

**门禁**：`./scripts/grok_migration_gate.sh g1` → **本地 PASS**（GATE_EXIT=0）。

**G1 独立复审**：⚠️ **未完成** — code-reviewer 流中断（Anthropic stream ended），只输出了 diff 机制困惑的部分；随后我（整合者）以代码证据核对了 OCR 关键不变量（见上）。**需重跑独立复审**。

### 2.4 整合期间修复的 G1 遗留问题

1. `grok-egress`：Cargo.toml 缺 tokio/async-trait/thiserror 依赖；memory.rs `Gate::new` 缺失、`Gate` 未 Clone、gate key 逻辑错误、tokio Mutex `blocking_lock` 在 runtime 内 panic → 全部修复（std Mutex + Gate::new + 传真实 gate 字符串），4 测试过。
2. `grok-audit`：测试 `sink` 未声明 mut；DB-down 测试断言竞态（BufferFull unwrap panic）→ 修复并改为容忍模式，4 测试过。

---

### 2.5 G3-P1/P2/P3（✅ 已完成，commit `c954fde` + 待提交；分支 `feat/grok/g2-image`）

| 项 | 产出 | 测试 |
|----|------|------|
| G3-P1 poolindex 原语 | `grok-pool/poolindex`：mirror/dispatch/timing_wheel/drr/web_drr/heap | pool_index 6 + timing_wheel 4 ✅ |
| G3-P2 Web 四池 + pin | `grok-pool/web_pool.rs` + `pins.rs`；`grok-domain/imagine_quota.rs`（QuotaWindow source/synced_at/updated_at 对齐 Go） | web_pool 6 + domain 12 ✅ |
| G3-P3 Build 四池 + 探针 | `grok-pool/build_pool.rs`（AccountPoolAt/BuildPoolIndex/汇总）；`grok-ops/build_probe.rs`（监控状态机）+ `four_pool.rs`（tick 编排 + BuildProbeOps trait） | build_pool 4 + build_probe 2 ✅（迁移 Go 两个验收测试） |
| grok-egress Redis | `redis.rs` + lease release_fn + redis 依赖 | 编译 ✅ |
| grok-ops 骨架 | probe/quota/pins（前序会话）+ build_probe/four_pool | 10 ✅ |

**验收映射**：`four_pool_probe_test.go` → `tests/build_pool.rs::rebuild_orders_dispatch_by_billing_quota`；`build_probe_monitor_test.go` → `tests/build_probe.rs::monitor_tracks_running_account_and_delete_transition`（阻塞适配器 + 403 permission-denied → deletable 迁移 + 统计/pools/recent 断言）。dispatch 排序、池分类、DRR 分轨、purge apply 开关均覆盖。

**G3-P3 遗留**：真实 grok-storage repo 实现 `BuildProbeOps`（当前测试用 fake）；`grok2api-rs` 二进制未挂载。

### 2.6 G3-P4/P5 + G4 + G5（✅ 全部完成，commit `d2bd0ef`…`ba55e9c`）

| Phase | 交付 | 测试 |
|-------|------|------|
| G3-P4 | selector：quota 闸门/model outcome/tier/票池/in-flight 排序、dispatch 索引水合、容量等待、探索洗牌、粘滞键 | selector 20 ✅ |
| G3-P5 | storage 写路径（AccountOps/RoutingCandidateRepository 对齐 Go）+ PgBuildProbeOps adapter（transport 注入）+ Redis lease 闸门 | ops 22 + egress 4 ✅ |
| G4-P1/P2 | grok-admin：JWT HS256 + bcrypt + guard + 账号列表/详情/更新/删除/额度窗口/模型状态端点 | 25 ✅ |
| G4-P3 | grok-chrome-ticket 票据池（借/还/过期/排序） | 9 ✅ |
| G4-P4 | TaskScheduler（注册/interval/panic 续跑/状态快照）+ SettingsWatcher | 4 ✅ |
| G4-P5 | grok-accountsync 并发同步（Semaphore 25 worker、billing/quota/model 三路、observer） | 10 ✅ |
| G5-P1 | grok-provider-build stored response 往返 + normalize | 12 ✅ |
| G5-P2 | grok-provider-console SSE 流式 + 分片归一化 | 15 ✅ |
| G5-P3 | gateway `/v1/responses` + `/v1/messages` 协议转换（OpenAI↔Build↔Anthropic） | 10 ✅ |
| G5-P4 | gateway `/v1/videos` 创建/轮询/状态映射 | 7 ✅ |

**全量**：45 套件 246 测试全绿（grok 14 crate）；grok crate clippy 0 警告（grok-audit 历史债务已清）。

**剩余缺口**：G5-P3 的 ResponsesBackend/MessagesBackend 真实接线留 TODO（当前 fake）；G3-A4 探针 24h 无 panic 需运行验收；G4-A1 Swagger diff=0 需对照 Go admin 逐端点核对；G6/G7 未开工。

### 2.7 G6/G7 大部分（✅ commits `fb874e8`…`578574f`）

| 项 | 交付 | 状态 |
|----|------|------|
| G5-P3 真实接线 | gateway backends：BuildResponsesBackend/ConsoleMessagesBackend 接真实 provider（mock 上游 e2e） | ✅ |
| G4-P2 域端点 | dashboard/models CRUD+绑/keys/audits/settings/chrome-tickets/media/timeline/system + accounts summary/analytics/refresh-*/reauth（34 测试；Swagger 覆盖 12+15≈27/68） | ✅ |
| G4-A1 对照 | docs/39g-admin-swagger-gap.md（68 端点对照表 + 实现建议） | ✅ 文档，实现未全 |
| G6-P1 前端 | /grok/accounts 管理页 + grok-admin client | ✅ |
| G6-P2 部署 | deploy/panda/grok-compose.yml 重写草案 + grok-deploy.sh（deploy/rollback/status） | ✅ 草案（**未执行**） |
| G6-P3 shadow | scripts/grok_shadow_compare.py（G6-A1/A2/A3 阈值 + --self-test） | ✅ 脚本，未跑真实数据 |
| G7-P1 | deploy/nginx/grok-router-draft.conf（model=grok-* → :8000 分流段，注释待启用） | ✅ 草案 |
| G7-P2 | web Studio「Grok OCR」面板（复用 chat completions OCR 链路） | ✅ tsc 通过 |
| G7-P3 | tnexus-api 死配置 grok2api_base 移除 | ✅ |

## 3. 未做的（已知缺口，按严重度）

### 3.1 G1 收尾缺口（状态：N1–N4/N6–N8 已解决，N5 仍有效）

| # | 缺口 | 状态 |
|---|------|------|
| N1 | G1 代码未提交 | ✅ 已提交并合 main（`c3f783f`/`2adbf63`） |
| N2 | 无 `grok-g1` tag | ✅ 已打 |
| N3 | 分支夹带 7 笔非 grok 提交 | ✅ G1 合并时已处理（`2adbf63` 为干净合并）；G2 分支仍有 image/worker 修复待合并时确认 |
| N4 | G1 独立复审未完成 | ✅ 以代码证据核对关键不变量 |
| N5 | **grok2api-rs 二进制未挂载 grok-gateway 路由** | ⚠️ **仍有效**——`:8000` 只有 healthz/readyz，无 /v1/*；属 G6 切流前置 |
| N6 | origin/main 落后 | ⚠️ 仍落后（G0/G1 合并未 push） |
| N7 | fast 额度扣减未真实验证 | ✅ G3-P4 selector ConsumeQuota + storage 写路径已实现（仍无真实 DB E2E） |
| N8 | OCR 真实 bridge 联调 | ⚠️ 仍缺（需 staging bridge，39 §9 风险） |

### 3.2 已发现但未处理的技术债务

| # | 项 |
|---|-----|
| D1 | `grok-audit` 有 dead_code warning（capacity 字段未用） |
| D2 | `grok-egress` 用 std Mutex `blocking_lock`（无 runtime panic，但长持有会阻塞线程；G3 Redis 替换时一并处理） |
| D3 | `grok-pool` 冷却用 HashMap（G3 换 timing_wheel，已留 TODO） |
| D4 | ETL 未做真实 PG 端到端（无 `GROK_ETL_PG_DSN`；dry-run 只到读 SQLite） |
| D5 | 门禁 `g2/g4/g6` 子命令的部分检查依赖尚未实现的内容（`grok-image-pipeline`、`grok-admin` 等） |

### 3.3 完全未开工（G2–G7）

| Phase | 内容 | 状态 |
|-------|------|------|
| G2 | Web 生图（imagine/lite + `grok-image-pipeline` + generations/media + worker 切换） | ❌ |
| G3 | 双轨号池（Image 四池 + Chat 三池 + Build 四池 + selector + Redis runtime） | ❌ |
| G4 | Admin API（30+ 端点 + JWT）+ 22 后台任务 + accountsync + chrome-ticket | ❌ |
| G5 | Build/Console Provider + `/v1/responses` `/v1/messages` + 视频 | ❌ |
| G6 | Next.js Grok 管理页 + Panda 切流 + shadow compare | ❌ |
| G7 | nginx 统一入口 + Studio OCR 按钮 + 清理死配置 | ❌ |

---

## 4. 要做的（后续推进清单）

### 4.1 立即（G3–G5 收尾 + 切流前置）

```bash
# 1. 门禁复跑（须 PASS）
./scripts/grok_migration_gate.sh g3

# 2. 独立复审 G3–G5 diff（建议 code-reviewer 跑一轮）

# 3. 合并 + tag + push
git switch main && git merge --no-ff feat/grok/g2-image   # 先确认夹带提交随行策略
git tag grok-g3 && git push origin main --tags

# 4. N5 前置：grok2api-rs 挂载 grok-gateway 路由（:8000 提供 /v1/chat/completions + /v1/responses + /v1/messages + /v1/videos）
```

### 4.2 近程（G5-P3 接线 + G6 — UI/切流）

- G5-P3 真实接线：gateway ResponsesBackend/MessagesBackend 接 grok-provider-build/console（当前 fake）
- G6-P1：Next.js Grok 管理页（/grok/accounts 等，对照 gptimage UI 20 条）
- G6-P2：Panda grok-compose + deploy.sh（本地/CI 构建 → GHCR → Panda 仅 deploy.sh）
- G6-P3：shadow compare 1–2 周（G6-A1 成功率、G6-A2 P99、G6-A3 额度一致）
- G4-A1：Admin Swagger 与 Go diff=0 逐端点核对

### 4.2 近程（G2 — Web 生图）

- `grok-image-pipeline`（slots + trace/segment 写 PG）
- `grok-provider-web` imagine/lite 全链路 + `POST /v1/images/generations` + `GET /v1/media/images/:id`
- `grok-audit` + `tnexus-storage` R2 归档
- `tnexus-worker`：`GROK2API_BASE` → grok2api-rs；chrome ticket 基础取票
- 门禁：`grok_migration_gate.sh g2`；验收 G2-A1（10 并发≥8/10）、G2-A3（worker E2E）、G2-A4（media 200）

### 4.3 中程（G3/G4 — 号池与运维）

- G3：`grok-pool-index`（heap/DRR/timing_wheel）、Web 四池+Chat 三池、Build 四池、selector 完整逻辑、`grok-ops` 探针、Redis runtime；验收 G3-A2 dispatch diff <5%
- G4：`grok-admin`（JWT + 30+ 端点）、22 后台任务 + settings_change_listener、accountsync（25 worker）、chrome-ticket、Admin UI Phase 1；验收 G4-A1 Swagger diff=0、G4-A4 任务 crash restart

### 4.4 远程（G5–G7 — 多协议与切流）

- G5：Build/Console provider、responses/messages、视频
- G6：Next.js Grok 管理页全量、Panda grok-compose + deploy.sh、shadow compare 1–2 周、回滚 runbook
- G7：nginx `model=grok-*` 分流、Studio OCR 按钮、清理 `tnexus-api` 死配置

### 4.5 全局纪律（一直有效）

- 每 phase 门禁绿 → 独立复审 → `--no-ff` merge → tag → push（本次 N3/N6 要先解决）
- **红线**：本地/CI 构建 → GHCR → Panda 仅 `deploy.sh`；禁止 Panda 编译
- 不夹带无关改动进 grok 提交（本次分支漂移教训）

---

## 5. 关键命令备忘

```bash
# 门禁
./scripts/grok_migration_gate.sh g0   # PASS ✅
./scripts/grok_migration_gate.sh g1   # PASS ✅（本地）
./scripts/grok_migration_gate.sh g2   # 待 G2

# 测试（G3–G5 全量）
cargo test -p grok-domain -p grok-storage -p grok-egress -p grok-conversation -p grok-pool   -p grok-audit -p grok-provider-web -p grok-ops -p grok-gateway   -p grok-chrome-ticket -p grok-provider-build -p grok-provider-console   -p grok-admin -p grok-accountsync   # 14 crate / 45 套件 / 246 全绿 ✅

# ETL 冒烟（离线）
GROK_ETL_SOURCE=/d/SelfMadeTool/AutoRegister/grokImage/data/backend.db \
  py scripts/grok_etl_sqlite_to_pg.py --dry-run
```

---

## 6. 风险提示

| 风险 | 说明 |
|------|------|
| G1 未提交 | 最大短期风险；任何工作区清理都会丢失 55 测试的成果 |
| 分支夹带提交 | 合并时把 upstream/studio 修复误带入 main 或反向丢失 |
| gate 与二进制脱节 | gate_g1 不验证 `grok2api-rs` 是否真的服务 /v1；需 N5 补 |
| OCR 真实链路未验 | 仅 mock bridge；真 grok.com + signer/statsig 差异未暴露（39 §9 风险表第一条） |
