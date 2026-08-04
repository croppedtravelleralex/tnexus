# 39b — Grok PostgreSQL Schema 与 Redis 布局

最后更新：**2026-08-04**  
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md)

## 1. 原则

- Grok 账号与 ChatGPT `tnexus-accounts` / `accounts.db` **完全隔离**。
- TNexus 现有 migration：`001`–`009`；Grok 从 **`010`** 起（勿与 `009_tnexus_accounts.sql` 混用前缀）。
- 源定义：`grokImage/backend/internal/infra/persistence/relational/schema.go`（**31** 张表）。
- 凭据：AES-GCM，密钥与 Go `config.yaml` → `security.credentialEncryptionKey` 对齐，便于 ETL 后原地解密。

---

## 2. Migration 拆分建议

| 文件 | 内容 |
|------|------|
| `migrations/010_grok_core.sql` | admins、sessions、accounts、credentials、links、web_profiles |
| `migrations/011_grok_quota_models.sql` | quota_windows、billing、pool_snapshots、quota_recovery、model_* 五表族 |
| `migrations/012_grok_routing_keys.sql` | model_routes、aliases、route_accounts、client_keys、permissions、billing_reservations |
| `migrations/013_grok_inference.sql` | request_audits、response_ownership、web_response_states |
| `migrations/014_grok_media_egress.sql` | media_jobs、media_assets、egress_nodes、egress_traffic_hops |
| `migrations/015_grok_pipeline_ops.sql` | image_pipeline_*、chrome_tickets、runtime_settings |

应用顺序：`010` → `015`；`grok-storage` 集成测试在空库上 `sqlx migrate run`。

---

## 3. 表清单（31）与 TNexus 命名

Go 表名 → 建议 PG 表名（可加 `grok_` 前缀统一，或保留原表名放 `grok` schema）：

| # | Go model / 表 | 建议 PG | 说明 |
|---|---------------|---------|------|
| 1 | `admins` | `grok_admins` | Admin 用户 |
| 2 | `admin_sessions` | `grok_admin_sessions` | JWT refresh |
| 3 | `provider_accounts` | `grok_accounts` | 账号主表 |
| 4 | `account_credentials` | `grok_credentials` | 加密 token + refresh 调度 |
| 5 | `account_provider_links` | `grok_account_provider_links` | 跨 provider 关联 |
| 6 | `web_account_profiles` | `grok_web_profiles` | Web 扩展字段 |
| 7 | `account_quota_windows` | `grok_quota_windows` | fast/auto/imagine |
| 8 | `account_billing_snapshots` | `grok_billing_snapshots` | Build 计费 |
| 9 | `account_pool_snapshots` | `grok_pool_snapshots` | 15min 分析快照 |
| 10 | `account_quota_recovery` | `grok_quota_recovery` | 恢复队列（DB 部分） |
| 11 | `model_routes` | `grok_model_routes` | 对外模型路由 |
| 12 | `model_route_aliases` | `grok_model_route_aliases` | 含 `grok-vision-ocr` |
| 13 | `model_route_accounts` | `grok_model_route_accounts` | pin 绑定 |
| 14 | `account_model_capabilities` | `grok_model_capabilities` | |
| 15 | `account_model_sync_states` | `grok_model_sync_states` | accountsync |
| 16 | `account_model_quota_blocks` | `grok_model_quota_blocks` | 冷却块 |
| 17 | `account_model_states` | `grok_model_states` | 探针状态 |
| 18 | `client_keys` | `grok_client_keys` | `g2a_*` |
| 19 | `client_key_models` | `grok_client_key_models` | |
| 20 | `billing_reservations` | `grok_billing_reservations` | |
| 21 | `request_audits` | `grok_request_audits` | |
| 22 | `response_ownership` | `grok_response_ownership` | Build 会话 |
| 23 | `web_response_states` | `grok_web_response_states` | Web 粘滞 |
| 24 | `media_jobs` | `grok_media_jobs` | 视频异步 |
| 25 | `media_assets` | `grok_media_assets` | 图片归档 |
| 26 | `runtime_settings` | `grok_runtime_settings` | 热加载 revision |
| 27 | `egress_nodes` | `grok_egress_nodes` | scope CHECK |
| 28 | `image_pipeline_traces` | `grok_pipeline_traces` | |
| 29 | `image_pipeline_segments` | `grok_pipeline_segments` | stage 枚举见 Go |
| 30 | `chrome_tickets` | `grok_chrome_tickets` | |
| 31 | `egress_traffic_hops` | `grok_egress_traffic_hops` | 每 hop 流量 |

### 3.1 关键索引（从 Go `schemaIndexes` 移植）

必须保留（性能 + 完整性）：

- `UNIQUE (identity_key)` on `grok_accounts`
- `idx_accounts_routing (provider, enabled, auth_status, priority DESC, id)`
- `idx_quota_windows_due (remaining, reset_at, account_id)`
- `idx_audits_created_id`、`idx_audits_event_id`（partial unique）
- `idx_image_pipeline_segments_trace (trace_id, sequence)`
- `idx_chrome_tickets_avail (status, expires_at, account_id)`

完整列表见 Go `schema.go` `schemaIndexes` 数组（43 条语句）。

### 3.2 `egress_nodes.scope` CHECK

允许值：`grok_build`、`grok_web`、`grok_web_asset`、`grok_console`。  
**不含** `grok_web_expand`（仅 runtime 闸门，见主文档 §2.2）。

### 3.3 `grok-vision-ocr` 种子数据

```sql
-- 示例：插入 model_route + alias（具体列名以 011 为准）
-- public_id / upstream 对齐 web/catalog.go grok-chat-fast
INSERT INTO grok_model_route_aliases (alias, ...) VALUES ('grok-vision-ocr', ...);
```

---

## 4. ETL（SQLite → PG）

脚本：`scripts/grok_etl_sqlite_to_pg.py`（G0 交付）

| 步骤 | 说明 |
|------|------|
| 1 | 只读打开 `grok2api/data/backend.db` |
| 2 | 按依赖顺序 COPY：accounts → credentials → quota → routes → … |
| 3 | 保留 `provider_accounts.identity_key`、`account_credentials` 密文 **原样** |
| 4 | 序列 / ID：保持 `id` 一致（便于 shadow diff） |
| 5 | 校验：`COUNT(*)` 每表；抽样 10 账号解密 + chat smoke |

环境变量：

- `GROK_ETL_SOURCE` — SQLite 路径
- `GROK_ETL_PG_DSN` — Postgres DSN
- `GROK_CREDENTIAL_KEY` — 与 Go config 相同（仅 ETL 验证用，勿入 git）

---

## 5. Redis Key 布局

Go 默认 prefix：`grok2api:`（`config.yaml` `runtimeStore.redis.keyPrefix`）。  
TNexus 建议：`grok:`（迁移期 ETL 不动 Redis；双实例时 **禁止** 共用 prefix）。

命名函数：`prefix + namespace + ":" + key`（`runtime/redis/store.go`）

| Namespace | Key 模式 | 用途 | TTL |
|-----------|----------|------|-----|
| `events` | `settings` | pub/sub 设置重载 | — |
| `sticky` | `{clientKey}:{model}:...` | 粘滞选号 | 请求配置 |
| `sticky-account` | `{accountId}` | 反向索引 | 同 sticky |
| `concurrency` | `{scope}:{gate}` | ZSET lease | lease + grace |
| `rate` | `{key}` | 分钟限流 | 1min window |
| `quota-recovery` | `events` / `attempts` / `claims` | 恢复队列 | — |
| (mirror) | `{prefix}:build:dispatch-index` | Build dispatch ZSET 镜像 | — |

多实例部署 **必须** Redis；单实例可用 `memory` driver（Go 默认）。

---

## 6. 与 TNexus Job 表关系

| TNexus 表 | Grok 关系 |
|-----------|-----------|
| `jobs.provider` | `grok` / `both` — 仅业务标记 |
| `job_results.provider` | 生图结果来源 |
| `job_results` pipeline JSON | 可存 grok2api-rs 返回的阶段耗时 |

**不**外键关联 `grok_accounts`；worker 不持有 Grok 账号 ID。

---

## 7. DDL 草案入口

完整 DDL 在 G0 由 `sqlx migrate` 落地；生成方式（Agent 执行一次）：

```bash
# 参考 Go models，手工维护 010-015；或用 pg_dump 自 staging Go+PG 实例
```

`010_grok_core.sql` 骨架（节选，实施时补全列）：

```sql
-- migrations/010_grok_core.sql
CREATE TABLE IF NOT EXISTS grok_accounts (
    id              BIGSERIAL PRIMARY KEY,
    identity_key    TEXT NOT NULL UNIQUE,
    provider        TEXT NOT NULL CHECK (provider IN ('grok_build','grok_web','grok_console')),
    enabled         BOOLEAN NOT NULL DEFAULT true,
    auth_status     TEXT NOT NULL DEFAULT 'unknown',
    priority        INTEGER NOT NULL DEFAULT 0,
    observed_model  TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS grok_credentials (
    account_id              BIGINT PRIMARY KEY REFERENCES grok_accounts(id) ON DELETE CASCADE,
    encrypted_access_token  BYTEA NOT NULL,
    encrypted_refresh_token BYTEA,
    refresh_due_at          TIMESTAMPTZ,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- ... 其余表见 011-015
```

---

## 8. Schema 验收

| ID | 检查 |
|----|------|
| S-1 | 31 表（或等价表族）存在 |
| S-2 | `egress_nodes.scope` CHECK 四类 |
| S-3 | 索引与 Go `schemaIndexes` 一一对应 |
| S-4 | ETL 后 `grok_accounts` 行数 = SQLite `provider_accounts` |
| S-5 | `grok_model_route_aliases` 含 `grok-vision-ocr`（G1 后） |

```bash
./scripts/grok_migration_gate.sh g0  # 含 schema 检查
```
