# 39 — grok2api 完整 Rust 移植进 TNexus

最后更新：**2026-08-03**

## 状态

| 项 | 状态 |
|----|------|
| 文档 | ✅ 规划档（未开工） |
| 代码 | ❌ 无 `grok-*` crate |
| 生产 | grok2api Go 容器在 Panda **已停止**（2026-07-28）；TNexus worker 仅 HTTP 调外部 `GROK2API_BASE` |

## 关联文档

| 文档 | 关系 |
|------|------|
| [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) | ChatGPT / gptimage 替代进度（加权 ≈85%） |
| [38-tnexus-production-cutover.md](38-tnexus-production-cutover.md) | gptimage 生产切流 |
| `AutoRegister/grokImage`（grok2api Go 仓） | **移植源** |
| [SOURCE.md](SOURCE.md) | gptimage Python 对照（Grok 不在此列） |

---

## 1. 结论（一句话）

**可行，但体量是「在 TNexus 内新建 Grok 子系统」**，不是给现有 `gateway` crate 加路由。

grok2api = Go 单体网关 + 三 Provider 号池 + 17 个后台任务 + 25+ 表 + Admin API + React 管理端。  
TNexus 当前仅有 worker 侧 `GROK2API_BASE` → `/v1/images/generations` 客户端，与完整 grok2api 能力差距 **约 90%+**。

完整 Rust 移植预估：**75–110 人周**（1 人约 18–24 个月；2–3 人约 7–10 个月）。

---

## 2. 现状对照

### 2.1 grok2api（源系统）

| 维度 | 说明 |
|------|------|
| 语言 | Go 1.26 + Gin；管理端 React 19 |
| 入口 | `backend/cmd/grok2api/main.go` → `internal/app/application.go` |
| Provider | `grok_build`（OAuth）、`grok_web`（SSO）、`grok_console`（SSO） |
| 号池 | Build 四池；Web 双 lane + 六池（dispatch/normal/verification/delete/dead/recovery） |
| 对外 API | `/v1/*`（OpenAI/Anthropic 兼容）+ `/api/admin/v1/*` |
| 后台任务 | 17 个（见 §5） |
| 侧车 | `browser-bridge`、`signer`（Statsig） |
| 数据 | SQLite/Postgres + Redis/Memory runtime |
| Panda | `/opt/grok2api/`，镜像 GHCR；**当前 Exited** |

### 2.2 TNexus（目标宿主）

| 维度 | 说明 |
|------|------|
| Rust workspace | 15 crate（`tnexus-api`、`gateway`、`worker`、`upstream`…） |
| Gateway `:8014` | **仅 ChatGPT** 数据面（PoW/TLS/arkose） |
| Grok 集成 | `crates/tnexus-worker/src/upstream.rs`：`GROK2API_BASE` + `grok-imagine-image` |
| 号池 | ChatGPT `accounts.db`（与 gptimage 共享），**无 Grok 账号** |
| UI | Studio/accounts/ops 面向 ChatGPT；**无 Grok 号池/审计/Chrome 票页** |
| 存储 | PostgreSQL（业务）+ Redis + R2 |

### 2.3 能力差距摘要

| 能力 | grok2api | TNexus |
|------|----------|--------|
| Grok 对话 + 识图/OCR | ✅ `grok-chat-*` + 图片上传 | ❌ |
| Grok 生图全链路 | ✅ Lite/Imagine + pipeline | ⚠️ worker 仅 generations |
| Grok 号池/调度 | ✅ 完整 | ❌ |
| Grok Admin API | ✅ 30+ 账号端点 | ❌ |
| 请求审计 | ✅ `request_audits` | ❌（仅有 ChatGPT usage NDJSON） |
| Chrome 票池 | ✅ | ❌ |
| Build/Console Provider | ✅ | ❌ |

---

## 3. 移植范围清单

### 3.1 对外推理 API（`/v1`）

| 端点 | 移植优先级 |
|------|------------|
| `GET /v1/models` | P1 |
| `POST /v1/chat/completions`（含 OCR） | P1 |
| `POST /v1/responses` + compact + GET/DELETE | P1 |
| `POST /v1/images/generations` | P1 |
| `GET/HEAD /v1/media/images/:id` | P2 |
| `POST /v1/images/edits` | P2 |
| `POST /v1/messages`（Anthropic） | P2 |
| `POST /v1/videos/generations` + GET 轮询 | P3 |
| `GET /healthz` / `/readyz` | P1 |

### 3.2 管理 API（`/api/admin/v1`）

| 模块 | 规模 | 优先级 |
|------|------|--------|
| Admin 认证（JWT + refresh） | 4 端点 | P1 |
| 账号 CRUD + import（Web/Console/Build/Device OAuth） | 30+ | P1 |
| Web 池（web-pools、reconcile、dispatch pin） | 5 | P1 |
| Build/Web 探针状态与控制 | 4 | P2 |
| 模型路由 CRUD + sync | ~8 | P1 |
| 客户端密钥 `g2a_*` + 计费预留 | ~6 | P2 |
| 请求审计 + summary | 2 | P1 |
| Dashboard / Settings / Egress / Media | 各若干 | P2 |
| 生图时间线 / Chrome 票池 | 各若干 | P2 |
| System info | 1 | P3 |

### 3.3 号池与调度（核心）

| 能力 | grok2api 源文件 | 复杂度 |
|------|-----------------|--------|
| Build 四池 | `four_pool_probe.go`、`poolindex/` | 高 |
| Web 双 lane + 六池 | `web_pool.go` | 极高 |
| 选号器（粘滞、冷却、lease、额度隔离） | `gateway/selector.go` | 极高 |
| Imagine slot / dispatch pin | `imagine_slots.go`、startup | 高 |
| Chrome ticket 偏好 | `chrometicket/pool.go` | 中 |
| 额度窗口（fast/auto/imagine/weekly） | `web/quota.go` | 高 |
| Imagine 0/0 + L2 Lite 探针 | `imagine_quota.go`、`web_pool_probe.go` | 高 |
| Quota recovery | `quotarecovery/` | 中 |

### 3.4 三 Provider 上游

| Provider | 源路径 | 复杂度 |
|----------|--------|--------|
| Web（SSO、Statsig、bridge、chat/SSE、imagine、video） | `infra/provider/web/`（15+ 文件） | **极高** |
| Build（OAuth、billing、Codex tools） | `infra/provider/cli/` | 高 |
| Console（无状态 Responses） | `infra/provider/console/` | 中 |
| 协议翻译 | `infra/provider/conversation/` | 高 |

### 3.5 OCR / 识图

- **无独立 OCR 模块**；通过 `grok-chat-fast` 等 + 图片上传实现。
- 上游：`/rest/app-chat/upload-file` → `fileAttachments` → 文本回复。
- 扣 **fast 对话额度**，非 imagine。
- 移植时需：`TextOnly: true`（关闭 `enableImageGeneration`），建议公开别名 `grok-vision-ocr`。
- 限制：单次最多 8 张图、总 64 MiB；多模态理解 ≠ 专用 OCR。

### 3.6 管理前端页面

grok2api React（需迁入 TNexus Next.js 或独立托管）：

`/dashboard` · `/accounts` · `/models` · `/client-keys` · `/request-audits` · `/media/images` · `/image-timeline` · `/docs/*` · `/settings`

---

## 4. 目标架构

**禁止**将 Grok 塞进现有 `gateway`（ChatGPT PoW/TLS）或 `upstream` crate。应 **平行新建 `grok-*` 栈**。

```text
TNexus workspace
├── tnexus-api :9000          # 工作台 + 可代理 Grok Admin
├── gptimage-gateway :8014    # ChatGPT（保持独立）
├── grok2api-rs :8000         # 【新建】Grok 全功能网关
├── tnexus-worker             # 调 grok2api-rs 而非外部 Go
└── grok-* crates（见下表）

侧车（不 Rust 化）
├── browser-bridge            # Chrome / CF
└── grok-signer               # Statsig POST /sign
```

### 4.1 建议新增 Crate

| Crate | 职责 |
|-------|------|
| `grok-domain` | Provider、QuotaWindow、PoolLane、Audit、ModelRoute |
| `grok-pool` | 四池 / Web 六池、候选列表 |
| `grok-pool-index` | heap、drr、timing_wheel、dispatch BTree |
| `grok-conversation` | Chat / Messages / Responses 协议翻译 |
| `grok-provider-core` | Provider trait、Registry |
| `grok-provider-web` | chat、image、video、quota、statsig、attachments |
| `grok-provider-build` | OAuth、billing、Codex tools |
| `grok-provider-console` | 无状态 Console |
| `grok-egress` | 出口节点、lease、CF cookie、scope |
| `grok-gateway` | `/v1` 推理、流式、失败分类 |
| `grok-audit` | 异步审计、计费预留 |
| `grok-image-pipeline` | 生图并发槽 + timeline |
| `grok-chrome-ticket` | 票池 CRUD + sweep |
| `grok-admin` | Admin API handlers |
| `grok-ops` | 17 个后台任务 runner |
| `grok-storage` | PG repositories |
| `grok2api-rs` | 主二进制 `main.rs` |

### 4.2 与现有 TNexus 整合

| 现有组件 | 整合方式 |
|----------|----------|
| `gateway`（ChatGPT） | 保持独立；可选 nginx 按 model 分流 |
| `tnexus-accounts-db` | **不动**；Grok 独立 `grok_*` 表 |
| `tnexus-worker` | `GROK2API_BASE` → `http://grok2api-rs:8000` |
| `tnexus-storage` | Grok 生图归档走 R2 |
| Redis / PostgreSQL | 共享实例，key 前缀 `grok:` |
| `tnexus-account-ops` | 并行；Grok SSO/OAuth 在 `grok-ops` 内 |

---

## 5. 后台运维任务（17 个，须全部移植）

来源：`grokImage/backend/internal/app/application.go` → `startBackground`。

| 任务名 | 周期/触发 | 职责 |
|--------|-----------|------|
| `settings_reconcile` | 30s | 运行时设置热加载 |
| `billing_reservation_cleanup` | 10min | 客户端密钥计费预留清理 |
| `model_cooldown_cleanup` | 10min | 模型级冷却块 prune |
| `response_ownership_cleanup` | 24h | Response 归属 TTL（30 天） |
| `quota_recovery` | 持续 | 免费/付费额度恢复探测 |
| `web_quota_refresh` | 持续 | Web 额度刷新队列 |
| `account_analytics` | 15min | 号池快照（保留 90 天） |
| `credential_refresh` | 持续 | Build token 自动续期 |
| `statsig_warmup` | 15min | Statsig 签名预热 |
| `web_quota_startup_catchup` | 启动 + 周期 | 陈旧 Web 额度补齐 |
| `model_catalog_startup_catchup` | 启动 + 6h | 模型能力目录同步 |
| `build_chat_capability_probe` | 可配 | Build 调度探针 |
| `build_dispatch_probe` | 可配 | Build 维护探针 |
| `web_dispatch_probe` | 可配 | Web 调度探针 |
| `web_maintenance_probe` | 可配 | Web 维护探针 |
| `image_dispatch_pin_sync` | 45s 首跑 + 5min | Imagine dispatch pin 对齐 |
| `video_recovery` + `video_workers` | 持续 | 异步视频任务 |
| `media_cleanup` | 周期 | 本地媒体磁盘清理 |
| `chrome_ticket_pool_sweep` | 15min | Chrome 票过期清扫 |
| `image_pipeline_cleanup` | 1h | 流水线事件清理（默认 12h） |
| `settings_change_listener` | Redis 广播 | 多实例设置同步 |

另：**启动阶段** `reconcileStartup`（凭据恢复、冷却恢复、due Web quota 排队等）。

---

## 6. 数据层

grok2api 约 **25+ 实体**，与 ChatGPT `accounts.db` **不可共用**。

| grok2api 表族 | TNexus 建议（PostgreSQL） |
|---------------|---------------------------|
| `provider_accounts` + `account_credentials` | `grok_accounts` + `grok_credentials` |
| `account_quota_windows` | `grok_quota_windows` |
| `account_model_states` / capabilities / quota_blocks | `grok_model_*` |
| `model_routes` + aliases + route_accounts | `grok_model_routes` |
| `client_keys` + billing_reservations | `grok_client_keys` |
| `request_audits` | `grok_request_audits` |
| `response_ownership` + web_response_states | `grok_response_*` |
| `media_jobs` + media_assets | 或复用 TNexus jobs + R2 |
| `egress_nodes` + egress_traffic | `grok_egress_*` |
| `chrome_tickets` | `grok_chrome_tickets` |
| `image_pipeline_*` | `grok_pipeline_*` |
| `runtime_settings` | `grok_runtime_settings` |
| `account_pool_snapshots` | `grok_pool_snapshots` |

- 迁移：`grok2api/data/backend.db` → PG 一次性 ETL（保留 `identity_key`、加密 token）。
- 迁移文件建议：`migrations/010_grok_schema.sql` … `015_grok_pipeline.sql`。
- 凭据加密：AES-GCM，密钥与 grok2api `config.yaml` 对齐以便迁移。

---

## 7. 分阶段路线图

### Phase 0 — 地基（3–4 周）

- [ ] `grok-domain` + `grok-storage` + PG migrations
- [ ] `grok2api-rs` 骨架：healthz、配置（对齐 `config.yaml` 语义）
- [ ] ETL：SQLite → PG
- [ ] CI：镜像 `ghcr.io/.../grok2api-rs`
- [ ] 测试 harness：对齐 Go `*_test.go` 用例清单

**门禁**：PG schema 与 grok2api 只读对比；ETL 671 账号抽样校验。

### Phase 1 — 推理最小闭环（6–8 周）

- [ ] `grok-egress`（基础 lease）
- [ ] `grok-provider-web`：chat + 附件 + OCR（`TextOnly`）
- [ ] `grok-conversation`：Chat Completions
- [ ] `grok-gateway`：`/v1/chat/completions`、`/v1/models`
- [ ] `grok-pool`：简化 dispatch（单池）
- [ ] Worker 切换内置 `grok2api-rs`

**门禁**：OCR 单图 E2E；fast 额度扣减正确。

### Phase 2 — Web 生图（6–8 周）

- [ ] `grok-provider-web`：imagine/lite 全链路
- [ ] `grok-image-pipeline` + timeline
- [ ] `/v1/images/generations` + media URL
- [ ] `grok-audit` + `tnexus-storage` 归档

**门禁**：`grok-imagine-image` 10 并发 ≥ 8/10。

### Phase 3 — 完整号池 + 选号（8–10 周）

- [ ] `grok-pool-index` + Web 六池 + Build 四池
- [ ] `grok-gateway/selector` 完整逻辑
- [ ] Imagine slot、dispatch pin
- [ ] `grok-ops`：web/build 探针、pin sync、quota refresh

**门禁**：671 Web 账号 reconcile；与 Go 快照 dispatch 误差 < 5%。

### Phase 4 — Admin + 运维 API（6–8 周）

- [ ] `grok-admin`：账号 30+ 端点、模型、审计、设置
- [ ] `grok-chrome-ticket`
- [ ] `grok-ops` 17 任务全部上线
- [ ] Admin UI Phase 1（accounts + audits + settings）

### Phase 5 — Build + Console + 多协议（8–10 周）

- [ ] `grok-provider-build` / `grok-provider-console`
- [ ] `/v1/responses`、`/v1/messages`
- [ ] Video 异步任务

### Phase 6 — Admin UI 完整 + 切流（4–6 周）

- [ ] Next.js 全部 Grok 管理页
- [ ] Panda：`grok2api-rs` 替代 Go 容器
- [ ] Shadow compare 1–2 周
- [ ] 下线 Go grok2api

### Phase 7 — 统一入口（可选，2–3 周）

- [ ] nginx / gateway 按 `grok-*` model 分流
- [ ] Studio OCR 按钮 → `grok-vision-ocr`

---

## 8. 工时估算

| 模块 | 人周（1 资深 Rust） |
|------|-------------------|
| grok-provider-web | 18–24 |
| grok-pool + selector | 12–16 |
| grok-provider-build | 8–12 |
| grok-gateway + stream | 6–8 |
| grok-ops（17 任务） | 6–8 |
| grok-admin API | 6–8 |
| grok-image-pipeline | 4–6 |
| grok-egress + chrome-ticket | 4–6 |
| grok-storage + ETL | 3–4 |
| Admin UI（Next.js） | 8–12 |
| 测试 / 对齐 / 切流 | 8–12 |
| **合计** | **75–110** |

| 团队 | 日历时间 |
|------|----------|
| 1 人全职 | 18–24 个月 |
| 2 人（Rust + 前端） | 10–14 个月 |
| 3 人（2 Rust + 1 前端/QA） | 7–10 个月 |

---

## 9. Panda 部署拓扑（终态）

```text
tnexus.relai.asia
  ├─ /v1/grok/* 或 model=grok-*  → grok2api-rs:8000
  ├─ /v1/* (chatgpt)            → gateway:8014
  └─ /*                         → tnexus-api:9000

grok2api-rs:8000
  ├─ PostgreSQL（grok_* 表，可与 tnexus 同库）
  ├─ Redis（grok: 前缀）
  ├─ browser-bridge（侧车）
  ├─ grok-signer（侧车）
  └─ R2（tnexus-storage）

下线：grok2api Go 容器
```

**部署铁律**：本地/CI 构建 → GHCR → Panda `pull && up`；**禁止 Panda 编译**。

---

## 10. 测试策略

1. **契约测试**：Go `*_test.go` → Rust integration tests
2. **Golden 对比**：同请求 Go vs Rust 上游 payload diff
3. **Shadow 生产**：双跑对比 `request_audits` 成功率、P99、额度
4. **探针回归**：`AutoRegister/grokImage/tools/panda_*` 改指向 Rust `:8000`

优先移植测试文件：

- `gateway/selector_test.go`、`gateway/service_test.go`
- `account/web_pool_test.go`、`account/four_pool_probe_test.go`
- `web/protocol_test.go`、`web/quota_test.go`
- `imagepipeline/scheduler_test.go`

---

## 11. 风险与缓解

| 风险 | 缓解 |
|------|------|
| Web Provider 边界遗漏 | Shadow + 保留 Go 镜像 digest 回滚 |
| 号池漂移（dispatch/pin/ticket） | 移植全套探针；每日快照 diff |
| Statsig/CF 行为差异 | signer + bridge 侧车不改 |
| 双系统运维混乱 | Phase 6 前禁止双写 |
| 人力不足半移植 | 按 Phase 门禁切流，不做「差不多就上线」 |

---

## 12. 决策记录

| 决策 | 选项 | 结论 |
|------|------|------|
| 是否塞进现有 `gateway` | 是 / 否 | **否** — ChatGPT 与 Grok 数据面分离 |
| Grok 号池存储 | 共用 accounts.db / 独立 | **独立 `grok_*` PG 表** |
| browser-bridge / signer | Rust 化 / 侧车 | **侧车** |
| Admin UI | 复用 grok2api React / 迁入 Next.js | **迁入 Next.js**（长期）；可过渡期双 UI |
| Build/Console | 与 Web 同期 / 延后 | Web 主路径优先；Build 可 Phase 5 |
| OCR 模型名 | 复用 fast / 专用别名 | **`grok-vision-ocr`** → 内部 fast + TextOnly |

---

## 13. 文档可优化方向

本文档为 **规划主档**；随实施应持续演进。可优化方向如下：

### 13.1 结构与导航

| 方向 | 说明 |
|------|------|
| 拆分子文档 | 将 §4 crate 设计、§6 schema、§7 Phase 各拆为 `39a`/`39b`/`39c`，主档只保留结论与索引 |
| 与 plan.md 对齐 | 在 `plan.md` 增加「Grok 移植」独立阶段，与 gptimage 替代并列，避免两条主线混淆 |
| 双向链接 | grokImage 仓 `docs/` 增加「TNexus 移植对照」指向本文 |
| 状态徽章 | 各 Phase 清单项改为「未开始 / 进行中 / 完成 + commit」可扫读 |

### 13.2 技术深度

| 方向 | 说明 |
|------|------|
| Go → Rust 函数级映射表 | `web/chat.go` 等逐文件 → crate 模块（附录或 `39b-go-rust-map.md`） |
| `010_grok_schema.sql` 草案 | 从 `grokImage/models.go` 导出完整 DDL + 索引说明 |
| API 契约快照 | 从 grok2api OpenAPI/Swagger 或 handler 导出端点清单 JSON，供契约测试 |
| 上游调用序列图 | Web 识图 / Lite 生图 / Imagine WS 各一张 mermaid 时序图 |
| Redis key 布局 | 粘滞、lease、settings 广播 key 命名与 TTL 表 |

### 13.3 验收与运维

| 方向 | 说明 |
|------|------|
| Phase 门禁脚本化 | 每 Phase 对应 `scripts/grok-migration-gate-N.sh` 可一键跑 |
| Shadow compare 指标模板 | 成功率、P99、额度偏差、dispatch 集合 diff 表格模板 |
| 回滚 Runbook | Go 镜像 digest 切回、`grok_*` 表只读冻结步骤 |
| Panda compose 草案 | `deploy/panda/grok-compose.yml` 与现有 gateway-compose 并列 |

### 13.4 产品与安全

| 方向 | 说明 |
|------|------|
| OCR 产品规格 | prompt 模板、输出格式、与专用 OCR 的能力边界说明 |
| 多租户 / 客户端密钥 | `g2a_*` 与 TNexus 用户体系是否合并的决策树 |
| 凭据迁移安全 | ETL 过程密钥处理、审计日志、最小权限 |

### 13.5 维护纪律

| 方向 | 说明 |
|------|------|
| 更新触发规则 | 任一 `grok-*` crate 合并或 grokImage 上游行为变更时更新 §2/§11 |
| 与 35/38 文档边界 | 35 = ChatGPT gptimage；39 = Grok grok2api；避免重复写 gateway 进度 |
| 版本化决策 | §12 决策表增加日期与「若推翻则新行追加」而非覆盖 |

---

## 14. 下一步（建议）

1. **若只要 OCR**：不启动全移植；恢复 Panda grok2api 或只做 Phase 1 子集。
2. **若确认全移植**：先交付 Phase 0（`grok-domain` + `010_grok_schema.sql` + ETL 脚本）。
3. **并行不阻塞 gptimage 切流**：Grok 移植与 [38](38-tnexus-production-cutover.md) 独立排期。
