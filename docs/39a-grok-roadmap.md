# 39a — Grok 移植路线图与验收门禁

最后更新：**2026-08-04**  
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md)

## 0. 晋级规则

| 从 | 到 | 条件 |
|----|-----|------|
| 未开工 | G0 通过 | `grok_migration_gate.sh g0` 全绿 |
| G0 | G1 | OCR 或 chat 最小闭环 + ETL 抽样 |
| G1 | G2 | 生图 `generations` parity + worker 可切换 |
| G2 | G3 | 双轨号池 + selector 快照 diff <5% |
| G3 | G4 | Admin API + 22 后台任务 + accountsync |
| G4 | G5 | Build + Console + responses/messages |
| G5 | G6 | Admin UI + Panda 切流 + shadow |
| G6 | G7 | 统一入口 + Studio OCR（可选） |

**禁止**：未过门禁合入下一 Phase；禁止放宽断言删测例过门。

与 gptimage 主线（plan.md P0–P5）：**无硬依赖**，可并行。

---

## Phase G0 — 地基

### Checklist

- [ ] **G0-1** 根 `Cargo.toml` 增加 `grok-domain`、`grok-storage`、`grok2api-rs`（最小可编译）
- [ ] **G0-2** `migrations/010_grok_schema.sql` … `015_grok_pipeline.sql`（见 [39b](39b-grok-schema.md)）
- [ ] **G0-3** `grok-storage`：PG repository trait + 账号/凭据只读
- [ ] **G0-4** `grok2api-rs`：`GET /healthz`、`GET /readyz`、配置加载（对齐 `config.yaml` 语义）
- [ ] **G0-5** ETL：`scripts/grok_etl_sqlite_to_pg.py`（`backend.db` → PG，保留 `identity_key`、加密 token）
- [ ] **G0-6** CI：GHCR 镜像 `grok2api-rs`（与 tnexus 同 workflow 或 matrix job）
- [ ] **G0-7** `deploy/panda/grok-compose.yml` 草案（grok2api-rs + bridge；signer 外置注释）

### 验收门禁（G0）

| ID | 检查项 | 命令 / 证据 |
|----|--------|-------------|
| G0-A1 | workspace 编译 | `cargo build -p grok2api-rs` |
| G0-A2 | 单元测试 | `cargo test -p grok-domain -p grok-storage` |
| G0-A3 | PG schema 表数 | `010`–`015` 应用后 31 逻辑表族 |
| G0-A4 | ETL 抽样 | ≥10 账号 `identity_key` + 解密 smoke；行数与 SQLite 一致 |
| G0-A5 | 配置校验 | 无效 `config` 拒绝启动（对齐 Go `config.Validate`） |

```bash
./scripts/grok_migration_gate.sh g0
```

---

## Phase G1 — 推理最小闭环（含 OCR）

### Checklist

- [ ] **G1-1** `grok-egress`：lease 基础 + `grok_web` scope
- [ ] **G1-2** `grok-provider-web`：chat SSE、`prepareChatAttachments`、`upload-file`
- [ ] **G1-3** `grok-conversation`：`normalizeOpenAIInput`、`contentTextAndImages`
- [ ] **G1-4** `grok-gateway`：`POST /v1/chat/completions`、`GET /v1/models`
- [ ] **G1-5** 模型别名 `grok-vision-ocr` → fast + `enableImageGeneration=false`
- [ ] **G1-6** `grok-pool`：简化单池 dispatch（可 pin 测试账号）
- [ ] **G1-7** `grok-audit`：异步写入 `grok_request_audits`
- [ ] **G1-8** browser-bridge 集成测试（mock 或 staging bridge）

### 验收门禁（G1）

| ID | 检查项 | 严格 |
|----|--------|------|
| G1-A1 | OCR 单图 E2E | 中英文样图 → 非空文本；payload golden `enableImageGeneration=false` |
| G1-A2 | 多图上限 | 9 张 → 400；8 张 → 200 |
| G1-A3 | fast 额度 | 请求前后 quota remaining 减 1 |
| G1-A4 | 流式 SSE | `stream:true` 事件完整结束 |
| G1-A5 | Go golden（可选） | 同 payload upstream body diff 白名单 |

```bash
./scripts/grok_migration_gate.sh g1
cargo test -p grok-provider-web -p grok-gateway
```

---

## Phase G2 — Web 生图

### Checklist

- [ ] **G2-1** `grok-provider-web`：imagine / imagine-lite 全链路
- [ ] **G2-2** `grok-image-pipeline`：槽位 + trace/segment 写 PG
- [ ] **G2-3** `POST /v1/images/generations` + `GET /v1/media/images/:id`
- [ ] **G2-4** `grok-audit` + `tnexus-storage` R2 归档
- [ ] **G2-5** `tnexus-worker`：`GROK2API_BASE` → `grok2api-rs`
- [ ] **G2-6** Chrome ticket 基础取票（高并发前可 pin 单账号）

### 验收门禁（G2）

| ID | 检查项 | 严格 |
|----|--------|------|
| G2-A1 | `grok-imagine-image` | 10 并发 ≥8/10 成功（剔 upstream） |
| G2-A2 | pipeline 元数据 | 阶段耗时字段存在 |
| G2-A3 | worker E2E | Studio `imageEngine=grok` job 完成 |
| G2-A4 | media URL | `GET /v1/media/images/:id` 200 |

```bash
./scripts/grok_migration_gate.sh g2
```

---

## Phase G3 — 完整号池与选号

### Checklist

- [ ] **G3-1** `grok-pool-index`：heap、DRR、timing_wheel
- [ ] **G3-2** Web Image 四池 + Chat 三池
- [ ] **G3-3** Build 四池 + `four_pool_probe`
- [ ] **G3-4** `grok-gateway/selector` 完整逻辑
- [ ] **G3-5** Imagine slot、`image_dispatch_pin_sync`
- [ ] **G3-6** `grok-ops`：探针、quota refresh、pin sync
- [ ] **G3-7** Redis runtime（多实例必选）

### 验收门禁（G3）

| ID | 检查项 | 严格 |
|----|--------|------|
| G3-A1 | Web reconcile | reconcile 后快照与 Go 一致 |
| G3-A2 | dispatch diff | 全量 dispatch 集合 vs Go <5% |
| G3-A3 | selector 单测 | Go `selector_test.go` 行为对齐 |
| G3-A4 | 探针 24h | 无 panic |

```bash
./scripts/grok_migration_gate.sh g3
```

---

## Phase G4 — Admin + 运维 API

### Checklist

- [ ] **G4-1** `grok-admin`：JWT auth
- [ ] **G4-2** 账号 30+ 端点
- [ ] **G4-3** models、keys、audits、dashboard、settings、media、timeline
- [ ] **G4-4** `grok-chrome-ticket`
- [ ] **G4-5** 22 后台任务 + `settings_change_listener`
- [ ] **G4-6** `accountsync`（25 worker）
- [ ] **G4-7** Admin UI Phase 1（`/grok/accounts` 等）

### 验收门禁（G4）

| ID | 检查项 |
|----|--------|
| G4-A1 | Swagger 端点 diff = 0（允许 deprecated） |
| G4-A2 | Web import → accountsync → 可 chat |
| G4-A3 | 每次推理有 audit 记录 |
| G4-A4 | 22 任务 crash restart |
| G4-A5 | chrome sweep 与 stats 一致 |

---

## Phase G5 — Build + Console + 多协议

### Checklist

- [ ] **G5-1** `grok-provider-build`
- [ ] **G5-2** `grok-provider-console`
- [ ] **G5-3** `/v1/responses`、`/v1/messages`
- [ ] **G5-4** Web→Build / Web→Console
- [ ] **G5-5** Video workers + recovery

### 验收门禁（G5）

| ID | 检查项 |
|----|--------|
| G5-A1 | Build stored response 往返 |
| G5-A2 | Anthropic `protocol_test` 对齐 |
| G5-A3 | Console 流式 200 |
| G5-A4 | Video poll 成功 |

---

## Phase G6 — Admin UI + 生产切流

### Checklist

- [ ] **G6-1** Next.js 全部 Grok 管理页
- [ ] **G6-2** Panda grok-compose + deploy.sh
- [ ] **G6-3** Shadow compare 1–2 周
- [ ] **G6-4** 回滚 runbook 演练
- [ ] **G6-5** 下线 Go 容器

### 验收门禁（G6）

| ID | 检查项 |
|----|--------|
| G6-A1 | 成功率 Rust ≥ Go −1% |
| G6-A2 | P99 ≤ Go ×1.15 |
| G6-A3 | 额度抽样 50 账号一致 |
| G6-A4 | 15min 内回滚 Go digest |

---

## Phase G7 — 统一入口（可选）

- [ ] **G7-1** nginx `model=grok-*` 分流
- [ ] **G7-2** Studio OCR → `grok-vision-ocr`
- [ ] **G7-3** 清理 `tnexus-api` 死配置
- [ ] **G7-4** 可选 `GROK_DIRECTOR_BASE`

---

## 后台任务对照（G4 须全部上线）

| 任务名 | Go 源 |
|--------|-------|
| `settings_reconcile` | `application.go:412` |
| `billing_reservation_cleanup` | `application.go:418` |
| `model_cooldown_cleanup` | `application.go:425` |
| `response_ownership_cleanup` | `application.go:432` |
| `quota_recovery` | `quotarecovery/service.go` |
| `web_quota_refresh` | `application.go:443` |
| `account_analytics` | `application.go:447` |
| `credential_refresh` | `credential_scheduler.go` |
| `statsig_warmup` | `startup.go` |
| `web_quota_startup_catchup` | `startup.go:474` |
| `model_catalog_startup_catchup` | `startup.go:534` |
| `build_chat_capability_probe` | `startup.go:562` |
| `build_dispatch_probe` | `startup.go:608` |
| `web_dispatch_probe` | `startup.go:644` |
| `web_maintenance_probe` | `startup.go:690` |
| `image_dispatch_pin_sync` | `startup.go:508` |
| `video_recovery` | `gateway/video.go` |
| `video_workers` | `gateway/video.go` |
| `media_cleanup` | `media/service.go` |
| `chrome_ticket_pool_sweep` | `application.go:506` |
| `image_pipeline_cleanup` | `application.go:522` |
| `settings_change_listener` | `application.go:536`（Redis） |

另：`reconcileStartup`、`audits.Start()`、`accountsync`（25 worker）。

---

## Shadow Compare 指标

| 指标 | 通过线 |
|------|--------|
| 推理成功率 | Rust ≥ Go −1% |
| chat/image P99 | ≤ Go ×1.15 |
| fast/imagine 额度 | 抽样 0 偏差 |
| dispatch 集合 | diff <5% |

记录：`artifacts/grok-shadow/<date>/summary.json`

---

## 回滚 Runbook

1. Stop `grok2api-rs`
2. Start Go grok2api @ pinned digest
3. Worker `GROK2API_BASE` → Go 端口
4. PG `grok_*` 只读冻结
5. `healthz` + grok job 冒烟
