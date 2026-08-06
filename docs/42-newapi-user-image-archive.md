# 42 — NewAPI 用户生图归档（Phase 0）

最后更新：**2026-08-05**

将 Gateway OpenAPI 生图写入 TNexus 图片管理，并按 NewAPI 用户归属。

---

## 1. 数据流

```
sub2api / NewAPI → gateway :8014
  → 生图成功
  → user_image_records（Postgres）+ /data/images（磁盘）
  → TNexus 图片管理（按 TNexus 用户 / NewAPI 绑定过滤）
```

工作台 Worker 生图仍走 `jobs` + `job_results`（`source=studio`）。

---

## 2. 表结构

| 表/列 | 说明 |
|--------|------|
| `users.newapi_user_id` | TNexus 用户 ↔ NewAPI 用户 ID（唯一） |
| `user_image_records` | Gateway OpenAPI 生图元数据 + 存储 key |

`backup_status`：`pending`（Phase 0 仅服务端 staging，默认保留 7 天）。

---

## 3. NewAPI 归因

Gateway 读取请求头（NewAPI 转发时应带上）：

| 头 | 含义 |
|----|------|
| `New-Api-User` / `new-api-user` | NewAPI 用户 ID |
| `X-Newapi-Token-Name`（可选） | token 名称，如 `tnexus-test-key` |

绑定：`POST /api/auth/newapi/bind` `{ "newapi_user_id": 1 }`  
管理员：`POST /api/auth/users/{id}/newapi`

在 **设置 → 账户** 可自助绑定。

---

## 4. 图片管理权限

| 角色 | 可见范围 |
|------|----------|
| **admin** | 全部 `job_results` + `user_image_records` |
| **member** | 自己的 Worker 图 + 已绑定 NewAPI 用户的 API 图 |

列表项 `source`：`studio` | `api`。

---

## 5. 环境变量

| 变量 | 默认 | 说明 |
|------|------|------|
| `IMAGE_STORE_PATH` | `/data/images` | Gateway + API + Worker 共用 |
| `GATEWAY_IMAGE_STAGING_RETENTION_DAYS` | `7` | `staging_expires_at`（Phase 1 清理任务） |
| `DATABASE_URL` | — | Gateway 需 `ACCOUNTS_BACKEND=postgres` 才归档 |

Gateway compose 需挂载：`/opt/tnexus/data/images:/data/images:rw`。

---

## 6. 运维检查

```bash
# Panda：查最近 API 归档
docker exec new-api-postgres psql -U tnexus -d tnexus -c \
  "SELECT id, owner_user_id, newapi_user_id, prompt, created_at FROM user_image_records ORDER BY created_at DESC LIMIT 5;"

# 绑定 root（NewAPI user id 多为 1，以库为准）
curl -sS -b cookies.txt -X POST https://tnexus.relai.asia/api/auth/newapi/bind \
  -H 'Content-Type: application/json' -d '{"newapi_user_id":1}'
```

NewAPI 用户 ID：`docker exec new-api-postgres psql -U newapi -d new-api -c "SELECT id, username FROM users;"`

---

## 7. Phase 1+（未实现）

- 浏览器 / 客户端备份目录 + `backup_status=backed_up` + 服务端 purge
- NewAPI OAuth 登录
- `staging_expires_at` 定时清理任务
