# 39d — Go → Rust 模块映射

最后更新：**2026-08-04**  
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md)

## 1. 顶层入口

| Go | Rust |
|----|------|
| `cmd/grok2api/main.go` | `crates/grok2api-rs/src/main.rs` |
| `internal/cli/run.go` | `grok2api-rs::cli` |
| `internal/app/application.go` | `grok2api-rs::app` |
| `internal/app/startup.go` | `grok-ops::startup` + `grok2api-rs::app` |
| `internal/transport/http/server.go` | `grok2api-rs::http::router` |

---

## 2. HTTP Handlers

| Go 包 | 路由前缀 | Rust |
|-------|----------|------|
| `transport/http/inference` | `/v1/*` | `grok-gateway::handlers` |
| `transport/http/account` | `/api/admin/v1/accounts*` | `grok-admin::account` |
| `transport/http/adminauth` | `/api/admin/v1/auth*` | `grok-admin::auth` |
| `transport/http/model` | `/api/admin/v1/models*` | `grok-admin::model` |
| `transport/http/clientkey` | `/api/admin/v1/client-keys*` | `grok-admin::client_key` |
| `transport/http/audit` | `/api/admin/v1/request-audits*` | `grok-admin::audit` |
| `transport/http/dashboard` | `/api/admin/v1/dashboard` | `grok-admin::dashboard` |
| `transport/http/settings` | `/api/admin/v1/settings` | `grok-admin::settings` |
| `transport/http/egress` | `/api/admin/v1/egress-*` | `grok-admin::egress` |
| `transport/http/media` | `/api/admin/v1/media*`、`/v1/media/images/:id` | `grok-admin::media` + `grok-gateway::media` |
| `transport/http/imagepipeline` | `/api/admin/v1/image-timeline` | `grok-admin::pipeline` |
| `transport/http/chrometicket` | `/api/admin/v1/chrome-tickets*` | `grok-admin::chrome_ticket` |
| `transport/http/system` | `/api/admin/v1/system` | `grok-admin::system` |

### 2.1 `/v1` 推理端点

| 方法 | 路径 | Go | Rust |
|------|------|-----|------|
| GET | `/v1/models` | `inference/handler.go` | `grok-gateway` |
| POST | `/v1/chat/completions` | `inference` + `provider/web/chat.go` | `grok-gateway` + `grok-provider-web` |
| POST | `/v1/responses` | `gateway/service.go` | `grok-gateway` |
| POST | `/v1/responses/compact` | Build provider | `grok-provider-build` |
| GET/DELETE | `/v1/responses/:id` | ownership | `grok-gateway` |
| POST | `/v1/messages` | `conversation` | `grok-conversation` |
| POST | `/v1/images/generations` | `provider/web/image.go` | `grok-provider-web` + `grok-image-pipeline` |
| POST | `/v1/images/edits` | `image.go` | G2+ |
| POST | `/v1/videos/generations` | `gateway/video.go` | G5 |
| GET | `/v1/videos/:id` | `gateway/video.go` | G5 |
| GET/HEAD | `/v1/media/images/:id` | `media/handler.go` | `grok-gateway::media` |

### 2.2 Admin 账号端点（节选）

| 能力 | Go handler 方法区 | Rust |
|------|-------------------|------|
| List/Get/Patch/Delete | `account/handler.go` | `grok-admin::account` |
| Import Web SSO | `importWeb` | `grok-admin::import::web` |
| Import Console | `importConsole` | `grok-admin::import::console` |
| Import Build OAuth | `importBuild` | `grok-admin::import::build` |
| Web pools / reconcile | `webPools` | `grok-pool::admin` |
| sync-dispatch-pins | `syncDispatchPins` | `grok-pool::pins` |
| web-probe / build-probe | `handler.go` | `grok-ops::probe` |
| convert-to-build | `runWebToBuildConversion` | G5 |
| sync-to-console | `runWebToConsoleSync` | G5 |

完整清单：对 `account/handler.go` 中 `Register` 路由表逐项映射（G4 门禁 Swagger diff）。

---

## 3. Application 层

| Go `internal/application/*` | Rust |
|-------------------------------|------|
| `gateway` | `grok-gateway`（service、selector、video） |
| `account` | `grok-pool` + `grok-admin::account` |
| `accountsync` | `grok-ops::accountsync` |
| `audit` | `grok-audit` |
| `chrometicket` | `grok-chrome-ticket` |
| `clientkey` | `grok-admin::client_key` |
| `dashboard` | `grok-admin::dashboard` |
| `egress` | `grok-egress` + `grok-admin::egress` |
| `imagepipeline` | `grok-image-pipeline` |
| `media` | `grok-admin::media` |
| `model` | `grok-admin::model` |
| `settings` | `grok-admin::settings` + `grok-ops::settings` |
| `adminauth` | `grok-admin::auth` |
| `account/poolindex` | `grok-pool-index` |

---

## 4. Provider 上游

| Go | Rust crate |
|----|------------|
| `infra/provider/web/*.go` | `grok-provider-web` |
| `infra/provider/cli/*.go` | `grok-provider-build` |
| `infra/provider/console/*.go` | `grok-provider-console` |
| `infra/provider/conversation/*.go` | `grok-conversation` |
| `infra/provider/registry` | `grok-provider-core` |

### 4.1 `grok-provider-web` 文件级

| Go 文件 | Rust 模块 | 说明 |
|---------|-----------|------|
| `chat.go` | `chat` | SSE、normalize、OCR 路径 |
| `attachments.go` | `attachments` | upload-file |
| `image.go` | `image` | imagine/lite/edits |
| `quota.go` | `quota` | 刷额度 |
| `catalog.go` | `catalog` | 静态模型表 |
| `statsig.go` | `statsig` | x-statsig-id |
| `video.go` | `video` | G5 |

### 4.2 OCR 相关函数链

```
inference.Handler.ChatCompletions
  → gateway.Service.Handle
    → provider/web.Adapter (chat)
      → normalizeOpenAIInput          → grok-conversation
      → contentTextAndImages          → grok-conversation
      → prepareChatAttachments        → grok-provider-web::attachments
      → openChat / openChatWithScope  → grok-provider-web::chat
      → buildWebChatPayload           → grok-provider-web::payload
        enableImage = !TextOnly       → grok-vision-ocr 强制 false
```

扩写（**非 OCR**）：

```
image.go expandPrompt
  → openChatWithScope(TextOnly:true, ScopeWebExpand)
```

---

## 5. 号池与探针

| Go | Rust |
|----|------|
| `account/web_pool.go` | `grok-pool::web` |
| `account/web_pool_probe.go` | `grok-pool::web_probe` |
| `account/web_pool_pins.go` | `grok-pool::pins` |
| `account/web_pools_cache.go` | `grok-pool::cache` |
| `account/web_lane_quota.go` | `grok-pool::lane_quota` |
| `account/four_pool_probe.go` | `grok-pool::build_probe` |
| `account/imagine_slots.go` | `grok-image-pipeline::slots` |
| `account/poolindex/*.go` | `grok-pool-index` |

---

## 6. 基础设施

| Go | Rust |
|----|------|
| `infra/persistence/relational/*` | `grok-storage` |
| `infra/egress/manager.go` | `grok-egress` |
| `infra/runtime/redis/store.go` | `grok-runtime` 或 `grok-ops::redis` |
| `infra/runtime/memory/*` | 单实例内存后端 |
| `infra/config/config.go` | `grok2api-rs::config` |
| `repository/*` | `grok-storage::repo` |

---

## 7. Domain

| Go `domain/*` | Rust `grok-domain` |
|---------------|---------------------|
| `account` | `account`, `provider`, `quota` |
| `egress` | `egress::Scope` |
| `audit` | `audit` |
| `imagepipeline` | `pipeline::Stage` |
| `chrometicket` | `chrome_ticket` |
| `model` | `model_route` |

---

## 8. 侧车（不移植，HTTP 客户端）

| 组件 | 路径 | Rust 封装 |
|------|------|-----------|
| browser-bridge | `browser-bridge/app.py` | `grok-egress::bridge_client` |
| signer | `signer/app.py` | `grok-provider-web::signer_client` |

环境变量：

- `GROK2API_BROWSER_BRIDGE_URL`（默认 `http://browser-bridge:8192`）
- `Provider.Web.StatsigSignerURL`

---

## 9. 前端（迁入 Next.js）

| React 路由 | Next 目标 |
|------------|-----------|
| `frontend/src/app/router.tsx` | `web/src/app/(console)/grok/*` |
| `/accounts` + WebProbePanel | `grok/accounts/page.tsx` |
| `/request-audits` | `grok/audits/page.tsx` |
| `/image-timeline` | `grok/timeline/page.tsx` |
| Settings EgressNodes | `grok/settings/page.tsx` |

数据源：`GROK_ADMIN_API` → `grok2api-rs` `/api/admin/v1`（非 tnexus-api）。

---

## 10. TNexus 整合点

| TNexus 文件 | 整合 |
|-------------|------|
| `crates/tnexus-worker/src/upstream.rs` | `GROK2API_BASE` → grok2api-rs |
| `crates/tnexus-worker/src/main.rs` | env 回退链 |
| `deploy/panda/.env.example` | `GROK2API_BASE=http://grok2api-rs:8000` |
| `deploy/panda/grok-compose.yml` | 新服务 |
| `web/src/components/studio/*` | G7 OCR UI |
| `migrations/010-015` | Grok schema |

---

## 11. Crate 首期合并建议（降低 Agent 上下文切换）

| 首期 crate | 合并自规划 |
|------------|------------|
| `grok-domain` | domain 全部 |
| `grok-storage` | persistence + repo |
| `grok-provider-web` | web + conversation（chat 路径） |
| `grok-pool` | pool + pool-index + selector 子集 |
| `grok-gateway` | gateway HTTP + audit 写入 |
| `grok-ops` | 后台任务 + accountsync |
| `grok2api-rs` | main + config + router |

G4 后再拆 `grok-admin`、`grok-chrome-ticket`、`grok-image-pipeline` 等。
