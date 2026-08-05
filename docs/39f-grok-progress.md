# 39f — Grok 移植进度记录（做了的 / 未做的 / 要做的）

最后更新：**2026-08-04**（G0 已完成并合并；G1 实现完成、**未提交**；G2–G7 未开工）
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md) · 路线图：[39a](39a-grok-roadmap.md) · 执行计划：[39e](39e-grok-execution-plan.md)

---

## 0. 一句话状态

> **G0 完成并已合入 `main`（tag `grok-g0`）**；**G1（OCR + chat 最小闭环）代码全部写完、55 个测试全绿、门禁 `g1` 本地 PASS，但尚未 git 提交、未合并、未打 tag**；G2–G7 未开工。

---

## 1. Git 现状（2026-08-04 实测）

| 项 | 值 |
|----|-----|
| 本地 `main` HEAD | `5a8c5d9`（G0 合并提交） |
| `origin/main` | `d5384e3`（**落后本地**：基线+G0 两笔合并未 push） |
| tag | `grok-g0` ✅（G0 门禁通过标记） |
| 分支 `chore/grok-plan-baseline` | 已合 main（`4249941`） |
| 分支 `feat/grok/g0-foundation` | 已合 main（`2e00cb2`） |
| 分支 `feat/grok/g1-ocr-chat` | **当前分支**；G1 代码全部未跟踪（untracked） |
| 未提交的 grok 文件 | 6 个 crate（31 个文件）+ `tests/grok_golden/` 2 个 + `Cargo.toml`/`Cargo.lock` 修改 |

### ⚠️ 分支漂移（重要）

`feat/grok/g1-ocr-chat` 上存在 **7 笔非 grok 提交**（`bcaefab`…`67d883d`，内容为 upstream/studio/worker 修复），
不在 `main` 上（`git log main..HEAD` 可见）。来源：切分支时工作区带入了这些已提交改动。
**合并 G1 到 main 前必须先处理**（cherry-pick 分离或确认是否应随 G1 一起合）。

---

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

## 3. 未做的（已知缺口，按严重度）

### 3.1 G1 收尾（阻塞性，未做）

| # | 缺口 | 影响 |
|---|------|------|
| N1 | **G1 全部代码未 git 提交**（6 crate + golden + Cargo.toml/lock 均 untracked） | 若丢失工作区=全丢；无法合并 |
| N2 | **G1 未合并 `main`、无 `grok-g1` tag** | 门禁已过但无正式里程碑 |
| N3 | **分支夹带 7 笔非 grok 提交**（`bcaefab`…`67d883d`） | 直接 merge 会把无关改动带入 main |
| N4 | **G1 独立复审未完成**（reviewer 流中断） | 违反「每 phase 独立复审」纪律 |
| N5 | **`grok2api-rs` 二进制未挂载 grok-gateway 路由**（grok2api-rs 只依赖 domain/storage，无 gateway） | 跑起来的 `:8000` 只有 healthz/readyz，**没有 /v1/chat/completions**；gate_g1 只测 crate 不测二进制，掩盖此缺口 |
| N6 | `origin/main` 落后本地 2 笔合并（未 push） | 远端没有 G0 |
| N7 | G1-A3（fast 额度扣减）**未真实验证**（quota 读取属 G3；G1 仅 audit 记录事件） | 门禁 G1-A3 是「请求前后 quota −1」，当前无真 quota |
| N8 | OCR 真实 bridge 联调未做（仅 mock bridge；L3 `GROK_INTEGRATION=1` 需 staging bridge） | 39c L3 未达标；真实 grok.com 时序未验证 |

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

### 4.1 立即（G1 收尾，半天内）

```bash
# 1. 处理分支漂移：把 G1 grok 文件与 7 笔无关提交分离
git switch -c feat/grok/g1-ocr-chat-clean   # 或 rebase 策略
# 建议：cherry-pick 或仅提交 grok 文件，不夹带 upstream/studio 提交

# 2. 提交 G1
git add crates/grok-egress crates/grok-conversation crates/grok-pool \
        crates/grok-audit crates/grok-provider-web crates/grok-gateway \
        tests/grok_golden Cargo.toml Cargo.lock
git commit -m "feat(grok): G1 OCR+chat minimal loop — egress/conversation/pool/audit/provider-web/gateway + golden"

# 3. 门禁复跑（须 PASS）
./scripts/grok_migration_gate.sh g1

# 4. 重跑独立 code-reviewer 复审 G1 diff（上次流中断）

# 5. 挂载 gateway 到 grok2api-rs 二进制（N5）：grok2api-rs 加 grok-gateway 依赖，
#    router 合并 build_app，使 :8000 真实可服务 /v1/chat/completions + /v1/models
#    （可选：把 N5 作为 G1 门禁补充项）

# 6. 合并 + tag + push
git switch main && git merge --no-ff feat/grok/g1-ocr-chat-clean
git tag grok-g1 && git push origin main --tags
```

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

# 测试
cargo test -p grok-egress          # 4 ✅
cargo test -p grok-conversation    # 12 ✅
cargo test -p grok-pool            # 8 ✅
cargo test -p grok-audit           # 4 ✅
cargo test -p grok-provider-web    # 18 ✅
cargo test -p grok-gateway         # 9（ocr_e2e）✅

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
