# 39g — Admin Swagger 缺口清单（G4-A1 对照）

对照源：Go `backend/internal/transport/http/server.go`（`/api/admin/v1` group，11 个 handler 域）
Rust 侧：`crates/grok-admin`（AdminRouter 7 路由 + auth 5 端点 + domains 19）
日期：2026-08-05（2026-08-06 更新计数）

## 0. 结论

- Go 端点总数：**68**（11 域；含 adminauth 5 个公开/受保护认证端点）
- Rust 已实现：**~31**（auth 5 + 账号域 7 + domains 19：dashboard/models/keys/audits/settings/chrome-tickets/media/timeline/system 均已实现核心集）
- 缺失：**~37**（低优先级扩展端点：批量操作、导出、明细级查询等）

G4-A1（Swagger diff=0）**不达标**——核心域已覆盖（含 2026-08-06 补的 media get/size-summary、system config/logs、models aliases/sync-state），剩余为低优先级扩展，按 §4 优先级续补。

---

## 1. 对照表

### 1.1 auth（Go `adminauth/handler.go` → Rust `service.rs`）— ✅ 全实现

| Go 端点 | 方法/要点 | Rust 状态 | 缺口 |
|---|---|---|---|
| `/auth/login` | POST 用户名密码 → access+refresh token，登录限流 | ✅ `AdminAuthService::login` | 无 |
| `/auth/refresh` | POST refresh token 换新对 | ✅ `refresh` | 无 |
| `/auth/logout` | POST 吊销 refresh token | ✅ `logout` | 无 |
| `/me` | GET 当前管理员信息 | ✅ `authenticate_access` | 无 |
| `/me/password` | PUT 改密 | ✅ `change_password` | 无 |

### 1.2 账号域（Go `account/handler.go` 34 端点 → Rust `admin_router.rs` 7 路由）— ⚠️ 子集

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/accounts` | GET 列表（分页/筛选） | ✅ `list` | 无 |
| `/accounts/:id` | GET 详情 | ✅ `get` | Rust 详情含额度窗口+模型状态，超出 Go（超集，可接受） |
| `/accounts/:id` | PATCH 更新 | ✅ `patch` | 无 |
| `/accounts/:id` | DELETE 删除 | ✅ `delete` | 无 |
| `/accounts/:id/quota` | GET（Rust 特有） | ✅ `quota_list` | Go 无此子路径；对齐 Go 时应并入详情或移除 |
| `/accounts/:id/quota` | PUT（Rust 特有） | ✅ `quota_put` | 同上 |
| `/accounts/:id/model-states` | GET（Rust 特有） | ✅ `model_states` | 同上 |
| `/accounts/summary` | GET 池规模汇总 | ❌ 缺失 | G6 UI 首页需要 |
| `/accounts/analytics` | GET 账号分析（额度分布等） | ❌ 缺失 | G6 UI 图表需要 |
| `/accounts/export` | GET CSV 导出 | ❌ 缺失 | 可延后 |
| `/accounts/import` / `/import-json` | POST 导入 | ❌ 缺失 | G4-P5 accountsync 导入链依赖 |
| `/accounts/batch` | PATCH 批量更新 | ❌ 缺失 | 可延后 |
| `/accounts/:id/refresh-token` | POST 单账号刷新 token | ❌ 缺失 | 运维必需（G6 页面按钮） |
| `/accounts/:id/reauth` | POST 触发重登 | ❌ 缺失 | 运维必需 |
| `/accounts/:id/refresh-billing` | POST 单账号 billing 探测 | ❌ 缺失 | 运维必需 |
| `/accounts/:id/refresh-quota` | POST 单账号 quota 刷新 | ❌ 缺失 | 运维必需 |
| `/accounts/refresh-billing` | POST 批量 billing 探测 | ❌ 缺失 | 可延后 |
| `/accounts/refresh-tokens` | POST 批量刷新 token | ❌ 缺失 | 可延后 |
| `/accounts/batch/refresh-billing` | POST 批量 billing（指定 ids） | ❌ 缺失 | 可延后 |
| `/accounts/build-probe` | GET/PATCH 探针配置与状态 | ❌ 缺失 | 需 grok-ops BuildFourPool.status |
| `/accounts/web-probe` | GET/PATCH web 探针 | ❌ 缺失 | 需 grok-ops WebDispatchProbe |
| `/accounts/web-lane-quota` | GET web 双轨额度 | ❌ 缺失 | 可延后 |
| `/accounts/web-pools` | GET web 池视图 | ❌ 缺失 | G6 号池页需要 |
| `/accounts/web-pools/reconcile` | POST 对账 | ❌ 缺失 | 可延后 |
| `/accounts/web-pools/sync-dispatch-pins` | POST 同步 pin | ❌ 缺失 | 需 grok-ops PinSyncTask |
| `/accounts/web/import` | POST web 导入 | ❌ 缺失 | G4-P5 依赖 |
| `/accounts/console/import` | POST console 导入 | ❌ 缺失 | 可延后 |
| `/accounts/web/convert-to-build` | POST 转换 | ❌ 缺失 | 可延后 |
| `/accounts/web/sync-to-console` | POST 同步 | ❌ 缺失 | 可延后 |
| `/accounts/web/refresh-quotas` / `/console/refresh-quotas` | POST 批量 quota | ❌ 缺失 | 可延后 |
| `/accounts/device/start` / `/:sessionId/poll` | POST 养号设备会话 | ❌ 缺失 | G6 养号日历需要 |

### 1.3 模型域（Go `model/handler.go` 8 端点）— ❌ 全缺

| Go 端点 | 方法/要点 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/models` | GET/POST/DELETE 模型路由 CRUD | ❌ | 数据源：`grok_model_routes`（012 migration） |
| `/models/:id` | GET/PATCH/DELETE | ❌ | 同上 |
| `/models/accounts` | GET 模型↔账号绑定 | ❌ | `grok_model_route_accounts` |
| `/models/batch` | PATCH 批量 | ❌ | 可延后 |
| `/models/sync` | POST 触发模型同步 | ❌ | accountsync model 路 |

### 1.4 keys（Go `clientkey/handler.go` 7 端点）— ❌ 全缺

| Go 端点 | 方法/要点 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/client-keys` | GET/POST CRUD | ❌ | 数据源：grok 或 tnexus client key 表 |
| `/client-keys/:id` | PATCH/DELETE | ❌ | 同上 |
| `/client-keys/:id/secret` | GET 重置密钥 | ❌ | 可延后 |
| `/client-keys/batch` | PATCH 批量 | ❌ | 可延后 |

### 1.5 audits（Go `audit/handler.go` 2 端点）— ❌ 全缺

| Go 端点 | 方法/要点 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/request-audits` | GET 审计列表 | ❌ | 数据源：`grok_request_audits`（grok-audit 已写，缺读） |
| `/request-audits/summary` | GET 汇总 | ❌ | 同上；G6 页面需要 |

### 1.6 dashboard（1 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/dashboard` | GET 首页看板聚合 | ❌ | 聚合账号/额度/流量/模型；G6 首页必需 |

### 1.7 settings（2 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/settings` | GET/PUT 全局配置 | ❌ | 含 settings_change_listener 触发（grok-ops SettingsWatcher 已备）；G6 设置页需要 |

### 1.8 egress（5 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/egress-nodes` | GET/POST | ❌ | 数据源：`grok_egress_nodes`（013 migration） |
| `/egress-nodes/:id` | PUT/DELETE | ❌ | 同上 |
| `/egress-traffic` | GET 流量统计 | ❌ | 可延后 |

### 1.9 media（5 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/media/images` | GET 列表 / DELETE | ❌ | 数据源：tnexus image archive 或 `grok_media` |
| `/media/images/:assetId` | GET/DELETE | ❌ | 同上 |
| `/media/images/stats` | GET | ❌ | G6 图片管理需要 |
| `/v1/media/images/:assetId` | GET 公开下载 | ❌ | 公开路由（非 admin）；媒体网关 |

### 1.10 timeline（1 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/image-timeline` | GET 生图时间线 | ❌ | 数据源：`grok_image_pipeline`/`job_results`；G6 流水图需要 |

### 1.11 chrome-tickets（3 端点）— ❌（底层 crate 已有）

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/chrome-tickets` | GET 列表 | ❌ | grok-chrome-ticket pool 已有，缺 admin 接线 |
| `/chrome-tickets/stats` | GET | ❌ | 同上 |
| `/chrome-tickets/sweep` | POST 清理 | ❌ | 同上 |

### 1.12 system（1 端点）— ❌

| Go 端点 | 方法 | Rust 状态 | 缺口说明 |
|---|---|---|---|
| `/system` | GET 版本/就绪信息 | ❌ | 简单；G6 页脚需要 |

---

## 2. G4-A1 优先级

### 2.1 必须补齐（G6 UI 所需优先，约 20 个）

| # | 端点 | 实现建议 |
|---|------|----------|
| 1 | `/accounts/summary` | grok-admin + grok-pool `summarize_build_probe_pools`/web_pool 统计；数据源 grok-storage `list_pool` |
| 2 | `/accounts/analytics` | grok-storage 聚合查询（额度分布/状态分布） |
| 3 | `/accounts/:id/refresh-billing` `/refresh-quota` `/refresh-token` `/reauth` | grok-ops `PgBuildProbeOps` + `BuildFourPool` 单账号方法；数据源 grok-storage AccountOps |
| 4 | `/accounts/web-pools` | grok-pool web_pool 状态 + storage |
| 5 | `/models` CRUD + `/models/accounts` | 新 `grok-admin/src/models.rs` + grok-storage model repo（012 migration） |
| 6 | `/client-keys` CRUD | 新 `grok-admin/src/client_keys.rs` + storage |
| 7 | `/request-audits` + `/summary` | grok-audit 加读接口 + storage SQL |
| 8 | `/dashboard` | 聚合上述 summary + models + audits；grok-admin 新 handler |
| 9 | `/settings` GET/PUT | grok-storage settings 表 + grok-ops SettingsWatcher 触发 |
| 10 | `/chrome-tickets` + `/stats` + `/sweep` | grok-chrome-ticket 暴露 pool 查询/清理，grok-admin 接线 |
| 11 | `/image-timeline` | grok-storage 查询 `job_results` |
| 12 | `/media/images` + `/stats` | grok-storage/tnexus-image-archive 复用 |
| 13 | `/system` | 常量 + 版本 |

### 2.2 可延后（非 UI 阻塞，约 36 个）

`/accounts/export`、`/accounts/import*`（4）、`/accounts/batch*`（3）、
`/accounts/refresh-billing`、`/accounts/refresh-tokens`、`/accounts/build-probe`、
`/accounts/web-probe`、`/accounts/web-lane-quota`、`/accounts/web-pools/reconcile`、
`/accounts/web-pools/sync-dispatch-pins`、`/accounts/device/*`（2）、
`/models/batch`、`/models/sync`、`/client-keys/:id/secret`、`/client-keys/batch`、
`/egress-*`（5）、`/media/images/:assetId`（2）、`/v1/media/images/:assetId`、
`/accounts/web/convert-to-build`、`/accounts/web/sync-to-console`、
`/accounts/web/refresh-quotas`、`/accounts/console/refresh-quotas`

---

## 3. Rust 特有/偏差注记

- Rust 有 Go 没有的 3 个子路径：`/accounts/:id/quota`（GET/PUT）、`/accounts/:id/model-states`（GET）——保留即可，G4-A1 diff 判定时按「超集」处理。
- Rust 响应形态为 `{items,page,pageSize,total}` 分页包裹；Go 各端点字段名需在接线时逐一核对（`user_id`/`team_id`/`source_key` 等 Grok 字段已进 domain）。
- JWT guard 已就绪（`guard.rs`），新端点直接挂 `AdminRouter::route` 即可，无需重做认证。
