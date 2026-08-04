# 39e — Grok 移植执行计划（分支 / 合并 / Subagent 推进）

最后更新：**2026-08-04**（承接 [39a](39a-grok-roadmap.md) 路线图与门禁）
状态：**规划已就绪，待启动**

## 0. 当前基线（已核实，2026-08-04）

| 项 | 现状 |
|----|------|
| git 分支 | 仅 `main`；Docs 39a–39d 未跟踪（untracked） |
| 已有 grok 骨架 | `migrations/010_grok_core.sql`（58 行，仅 core）、`scripts/grok_etl_sqlite_to_pg.py`（39 行骨架）、`scripts/grok_migration_gate.sh`（117 行全）、`deploy/panda/grok-compose.yml`（草案）——**均 untracked** |
| crate | **无任何 `grok-*`**；根 `Cargo.toml` members 无 grok |
| 源码 | `D:\SelfMadeTool\AutoRegister\grokImage\`（Go，111 `*_test.go`；`backend.db` 679KB 存在） |
| Cargo workspace | 15 crate（含 gateway/upstream） |

## 1. 分支策略

**每 Phase 一条分支，门禁全绿才合并 `main`；禁止跨 Phase 合并、禁止放宽门禁删测例过门。**

合并顺序（严格线性，`--no-ff`）：

```
main
 ├─ feat/grok/g0-foundation ──▶ merge g0（G0 门禁绿）
 ├─ feat/grok/g1-ocr-chat   ──▶ merge g1
 ├─ feat/grok/g2-image      ──▶ merge g2
 ├─ feat/grok/g3-pool       ──▶ merge g3
 ├─ feat/grok/g4-admin      ──▶ merge g4
 ├─ feat/grok/g5-build-console ─ merge g5
 ├─ feat/grok/g6-ui-cutover ──▶ merge g6
 └─ feat/grok/g7-unified    ──▶ merge g7（可选）
```

执行纪律：
- 每条分支从 `main` 切出（后一分支在前一分支**已合并**后切，避免跨 Phase 依赖）。
- **红线**：本地/CI 构建 → GHCR Actions → Panda **仅 `deploy.sh`**；禁止 Panda 编译（生产机禁 `cargo/docker build`）。
- 合并前必须跑 `./scripts/grok_migration_gate.sh g<N>` 全绿；每次合并 `--no-ff` 留合并提交，便于回滚 digest。

## 2. Subagent 派发模型

- 每个独立 crate/模块 = 一个 `worker`（或 `rust-refactoring-specialist`）子代理，在**独立 git worktree** 隔离（`worktree:true`／`using-git-worktrees`），`context: fresh`。
- 每条 phase 分支内：**可并行**的 crate 组并行跑；**有依赖**的串行（顶层入口 `grok2api-rs` 依赖各 provider，放 phase 末）。
- 编码后追加**独立 reviewer**子代理做代码审查（走 `requesting-code-review` + `code-reviewer`），再跑门禁。
- 关键决策点（号池存储、drawbridge 顺序、Redis 切分）用 `oracle`/`planner` 子代理裁定，避免漂移。

## 3. 分支 × Subagent 任务与验收

> 验收 = 该 crate 编译 + 对应 Go 单测移植过（[39c](39c-grok-test-matrix.md)）+ 本 phase 门禁子段。每个任务列其**必产样图/golden 与验收命令**。

### Phase G0 — `feat/grok/g0-foundation`（地基）

**可并行（5 子代理）**：

| # | 子代理任务 | 产出 | 验收（门禁 G0-* + 39c L0/L1） |
|---|-----------|------|------|
| G0-P1 | `worker`：crate 骨架 `grok-domain` + 根 `Cargo.toml` members | `grok-domain` 可编译 | `cargo build --workspace` 绿；`cargo test -p grok-domain` |
| G0-P2 | `worker`：`grok-storage`（PG repository trait + 账号/凭据只读） | `grok-storage` | `sqlx migrate run` 空库可建 schema |
| G0-P3 | `database-optimizer`：补齐 `migrations/011–015`（基于 39b §2 + Go `schema.go` 31 表/43 索引） | 6 个 sql 文件 | G0-A3 31 逻辑表族；S-2 scope CHECK 四类；S-3 索引一一对应 |
| G0-P4 | `worker`：ETL `grok_etl_sqlite_to_pg.py` 全实现（31 表依赖序 COPY） | 脚本 | G0-A4 ≥10 账号 identity_key + 解密 smoke；行数与 SQLite 一致 |
| G0-P5 | `devops-automator`：`grok2api-rs` CI 镜像 + `grok_migration_gate.sh g0` 接 CI | GHCR job | G0-A1/A2/A5 在 CI 绿 |

**串行末**：

| # | 任务 | 验收 |
|---|------|------|
| G0-P6 | `worker`：`grok2api-rs` 顶层（config 加载、`/healthz` `/readyz`） | G0-A5 无效 config 拒启；`curl /healthz`/`/readyz` 200 |

**合并门禁**：`./scripts/grok_migration_gate.sh g0` 全绿 → `--no-ff` merge → git tag `grok-g0`。

### Phase G1 — `feat/grok/g1-ocr-chat`（OCR + chat 最小闭环）

**依赖拓扑**：`grok-egress` → `grok-provider-web`/`grok-conversation` → `grok-pool`(简化单池) → `grok-audit` → `grok-gateway`。

**串行链 + 并行内层（核心是 OCR）**：

| # | 任务 | 验收（G1-* / OCR 矩阵） |
|---|------|------|
| G1-P1 | `worker`：`grok-egress` lease 基础 + `grok_web` scope | `egress/manager_test.go` 移植过 |
| G1-P2 | `rust-refactoring-specialist`：`grok-provider-web`（chat SSE、`prepareChatAttachments`、`upload-file`）+ bridge 客户端 | `browser_bridge_test.go` 过 |
| G1-P3 | `worker`：`grok-conversation`（`normalizeOpenAIInput`、`contentTextAndImages`）+ **`grok-vision-ocr` 别名`enableImageGeneration=false`** | G-OCR-7 payload golden 锁 `enableImageGeneration=false`；`protocol_test.go` 过 |
| G1-P4 | `worker`：`grok-pool` 简化单池（可 pin 测试账号） | dispatch pin 测试账号可命中 |
| G1-P5 | `worker`：`grok-audit` 异步写 `grok_request_audits` | 每次推理有 audit 记录 |
| G1-P6 | `worker`（末）：`grok-gateway` `POST /v1/chat/completions` + `GET /v1/models` + OCR E2E `tests/ocr_e2e.rs` | G1-A1 单图 E2E 非空文本；G1-A2 9图→400/8图→200；G1-A3 fast 额度 −1；G1-A4 SSE 完整；G-OCR-1~6,9,10 |

**合并门禁**：`./scripts/grok_migration_gate.sh g1` + `cargo test -p grok-provider-web -p grok-gateway`。样图需中英文混排（L3 bridge staging）。tag `grok-g1`。

### Phase G2 — `feat/grok/g2-image`（Web 生图 + worker 切换）

| # | 任务 | 验收（G2-* / 生图矩阵） |
|---|------|------|
| G2-P1 | `worker`：`grok-image-pipeline`（slots + trace/segment 写 PG） | G-IMG-2 PS trace segment 存在；`scheduler_test.go` 过 |
| G2-P2 | `worker`：`grok-provider-web` imagine/lite 全链路 + `/v1/images/generations` + `/v1/media/images/:id` | G-IMG-1 200；G2-A4 media 200 |
| G2-P3 | `worker`：`grok-audit` + `tnexus-storage` R2 归档 | 生图结果落 R2 |
| G2-P4 | `tnexus-worker`：`GROK2API_BASE→grok2api-rs` + chrome ticket 基础取票 | G2-A3 worker E2E：Studio `imageEngine=grok` job 完成；G2-A1 10并发≥8/10 |

**门禁**：`grok_migration_gate.sh g2`（含 live generations smoke）。tag `grok-g2`。

### Phase G3 — `feat/grok/g3-pool`（双轨号池 + selector）

**并行**：

| # | 任务 | 验收（G3-* / P0 池测试） |
|---|------|------|
| G3-P1 | `worker`：`grok-pool-index`（heap、DRR、timing_wheel） | `poolindex_test.go`、`web_drr_test.go` 过 |
| G3-P2 | `worker`：Web Image 四池 + Chat 三池 | `web_pool_test.go`、`web_pool_pins_test.go` 过 |
| G3-P3 | `worker`：Build 四池 + `four_pool_probe` | `four_pool_probe_test.go`、`build_probe_monitor_test.go` 过 |
| G3-P4 | `worker`：`grok-gateway/selector` 完整 + imagine slot + pin sync | `selector_test.go` 过；G3-A3 对齐 |
| G3-P5 | `worker`：`grok-ops` 探针/quota refresh/pin sync + Redis runtime | G3-A4 探针 24h 无 panic |

**门禁**：`grok_migration_gate.sh g3`（dispatch diff <5%）。tag `grok-g3`。

### Phase G4 — `feat/grok/g4-admin`（Admin + 22 任务）

**并行**：

| # | 任务 | 验收（G4-*） |
|---|------|------|
| G4-P1 | `worker`：`grok-admin` JWT auth + 账号 30+ 端点 | G4-A1 Swagger diff=0 |
| G4-P2 | `worker`：models/keys/audits/dashboard/settings/media/timeline | 各 `*_test.go` 过 |
| G4-P3 | `worker`：`grok-chrome-ticket` | `chrometicket/pool_test.go` 过 |
| G4-P4 | `worker`：22 后台任务 + `settings_change_listener` | G4-A4 22 任务 crash restart |
| G4-P5 | `worker`：`accountsync`（25 worker） | G4-A2 Web import→accountsync→可 chat |
| G4-P6 | `frontend-developer`：Admin UI Phase 1（`/grok/accounts` 等） | 页面可连 `grok-admin` |

**门禁**：`grok_migration_gate.sh g4`。tag `grok-g4`。

### Phase G5 — `feat/grok/g5-build-console`（Build/Console/多协议）

| # | 任务 | 验收（G5-*） |
|---|------|------|
| G5-P1 | `worker`：`grok-provider-build` | G5-A1 stored response 往返；`adapter_test.go`、`normalize_test.go` 过 |
| G5-P2 | `worker`：`grok-provider-console` | G5-A3 Console 流式 200；`console_test.go` 过 |
| G5-P3 | `worker`：`/v1/responses`、`/v1/messages` + Web→Build/Console 转换 | G5-A2 Anthropic `protocol_test.go` 对齐 |
| G5-P4 | `worker`：video workers + recovery | G5-A4 video poll 成功 |

**门禁**：`grok_migration_gate.sh g5`（脚本需补 g5 子命令）。tag `grok-g5`。

### Phase G6 — `feat/grok/g6-ui-cutover`（Admin UI + 切流）

| # | 任务 | 验收（G6-* / Shadow） |
|---|------|------|
| G6-P1 | `frontend-developer`：Next.js 全部 Grok 管理页 | 与 gptimage UI 对照 20 条 |
| G6-P2 | `devops-automator`：Panda grok-compose + deploy.sh | G6-A4 15min 回滚 runbook 演练 |
| G6-P3 | `sre-site-reliability-engineer`：Shadow compare `grok_shadow_compare.py` + 1-2 周 | G6-A1 成功率≥Go−1%；G6-A2 P99≤Go×1.15；G6-A3 额度 50 账号一致 |

**门禁**：`grok_migration_gate.sh g6`（shadow summary 阈值）。tag `grok-g6`。

### Phase G7 — `feat/grok/g7-unified`（统一入口，可选）

| # | 任务 | 验收 |
|---|------|------|
| G7-P1 | `devops-automator`：nginx `model=grok-*` 分流 | 路由生效 |
| G7-P2 | `frontend-developer`：Studio OCR → `grok-vision-ocr` | Studio OCR 按钮出文本 |
| G7-P3 | `worker`：清理 `tnexus-api` 死配置 `grok2api_base` | 配置移除编译绿 |
| G7-P4 | `worker`：可选 `GROK_DIRECTOR_BASE` | 构思走真 grok |

---

## 4. 派发清单（总量）

- **~50 子代理任务**（50 执行 + 每 phase 独立 reviewer 复审 ≈ 60+ 调用）
- 每 phase 末：1× `reviewer`/`code-reviewer` 独立复审 diff → 1× 门禁跑子代理 → 人工 `--no-ff` merge `main`。

## 5. 我要如何跑

1. 明确**从哪个 Phase 开始**（建议 G0；若你只要 OCR 单图，切问题 A：方案 A 恢复 Go vs G1 最小 Rust，用 `oracle` 裁定）。
2. 用 `workflow`/`subagent` 并行派发：G0 五路 + reviewer，worktree 隔离。
3. 每 phase 等门禁 → 合并 → 下一 phase 分支。
4. Panda 走 deploy.sh（pull+up），禁止编译。

## 6. 风险

- 各 provider 依赖 Go 私有协议（signer/statsig/bridge）→ 保留 sidecar，不 Rust 化（39 §8）。
- phase 间依赖：G3 依赖 G1/G2 已完成；G4 依赖 G3。
- PHP/Go 源码引用 `backend.db` 仅 ETL 源，勿提交凭据。