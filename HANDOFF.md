# HANDOFF — TNexus（含 gateway-rs 合并）

最后更新：**2026-07-31（共享 accounts.db + 删除 JSON 快照）**

## 读什么（按顺序）

1. **[plan.md](plan.md)** — 合并施工总控与详细待办
2. **[docs/34-tnexus-rollout-oauth-panda.md](docs/34-tnexus-rollout-oauth-panda.md)** — 号池/OAuth 三阶段上线
3. **[docs/SOURCE.md](docs/SOURCE.md)** — UI/API 对照源（gptimage Python 仓）
4. **[README.md](README.md)** — 本地开发与仓库结构

---

## 项目定位（合并后）

**TNexus** = 导演工作台（studio）+ **chatgpt2api 管理台**（号池/生图/运维）+ **Rust gateway**（`/v1/` 生图/对话）。

- **唯一公网入口**：`https://tnexus.relai.asia`
- **号池页**：`https://tnexus.relai.asia/accounts`（需最新 GHCR 镜像）
- **不接** Panda 生产 Python `chatgpt2api-local :8012` 的管理 HTTP API（对照实现，不运行时依赖）
- **禁止** `GPTIMAGE_ADMIN_TOKEN` 代理生产 `:8012`；运维走 **account-ops `:9011` + `GPTIMAGE_ROOT`**
- **刷新/重登/OAuth/养号/Outlook/窗口预热** 均走 account-ops（本地 Python 库），不走 gptimage HTTP

---

## 当前状态（2026-07-31）

### 已完成（本迭代）

| 项 | 状态 |
|----|------|
| 去生产 HTTP 代理（`gptimage_proxy` 删除） | ✅ |
| 号池全列排序、慢刷/软封/编辑、流水图、养号日历 UI | ✅ |
| 图片管理：小图/WebP thumb/灯箱缩放拖拽 | ✅ |
| 生图结果持久化 `inline_preview_b64`（非 gateway 内存 URL） | ✅ `a54d057` |
| 顶栏原生 `<a>` 导航（静态导出） | ✅ `863fe28` |
| gateway `:8014` → `tnexus-gateway` 镜像 | ✅ `panda-gateway-1` |
| account-ops：养号/Outlook/窗口预热执行面 | ✅ 需 `GPTIMAGE_ROOT` |
| tnexus-api 委托 account-ops + gateway 预热回退 | ✅ |
| gateway `scheduling_gate`（需单独更新 :8014 进程） | ✅ 代码已合入 `crates/gateway` |
| 差距文档 | ✅ [docs/35-tnexus-gptimage-gap.md](docs/35-tnexus-gptimage-gap.md) |

### 已完成（基线）

| 项 | 状态 |
|----|------|
| TNexus API + worker + Postgres + Redis | ✅ Panda `/opt/tnexus` |
| Gateway `:8014` + UFW/安全组放行 | ✅ 公网与回环均可 |
| nginx `tnexus.relai.asia` | ✅ `/v1/`、`/api/backend/` → 8014；其余 → 9000 |
| URL 生图（域名） | ✅ `GPTIMAGE_BASE=https://tnexus.relai.asia` 或回环 `127.0.0.1:8014` |
| OAuth / 刷新 / 重登 | ✅ `account-ops` 镜像 + `:9011` |
| 本地验收 | ✅ 38 账号导入、`TNEXUS_URL_CHAIN_OK` |

### 待验收 / 进行中

| 项 | 状态 |
|----|------|
| GHCR 拉取最新镜像（api + worker + account-ops + 静态 UI） | ✅ 2026-07-31 18:03 `9dcb82a` |
| gateway `:8014` `tnexus-gateway` | ✅ 与 TNexus `crates/gateway` 同步 |
| `UPSTREAM_API_KEY` 定期刷新（cron） | ⚠️ 需运维；TTL ≈24h，过期报 `invalid session` |
| 号池与 8012 共享 live `accounts.db`（WAL + 事务写入） | ✅ `tnexus-accounts-db` |
| `pin_account.json` 与 pool/sqlite 同步 | ⚠️ 曾过期；刷新后需与 pool 对齐 |
| Outlook 恢复 UI、养号结果 merge 回 JSON | 📋 下一迭代 |
| gateway-rs 物理归档 | 已迁入 `crates/gateway`，待删独立仓 |

---

## 域名 vs IP

| 场景 | 推荐 `GPTIMAGE_BASE` |
|------|----------------------|
| Panda 同机 worker | `http://127.0.0.1:8014`（回环，最快） |
| 本地 WSL 开发 | `https://tnexus.relai.asia`（走 Nginx `/v1/`，无需开 8014 隧道） |
| 裸 IP `:8014` | 可用但无 TLS；需云安全组 **+** 主机 `ufw allow 8014/tcp` |

Gateway 登录拿 JWT 仍在 Panda 本机：`http://127.0.0.1:8014/api/auth/login`（Nginx 未反代 `/api/auth` 到 gateway，避免与 TNexus 登录冲突）。

---

## Panda 拓扑（现网）

```text
tnexus.relai.asia (nginx)
  ├─ /v1/*              → 127.0.0.1:8014  (tnexus-gateway, DATA_PLANE=upstream)
  ├─ /api/backend/*     → 127.0.0.1:8014
  └─ /*                 → 127.0.0.1:9000  (tnexus-api + 静态 UI)

chatgpt2api-local :8012       — 生产 gpt-image（live accounts.db，禁止替换）
/opt/tnexus/.env              — secrets + UPSTREAM_API_KEY（gateway JWT）
/opt/tnexus/data/pool/        — scheduling_state.json、usage_events.ndjson（调度/用量）
/root/gptimage/data/accounts.db — 号池真源（8012 + TNexus api/gateway 共享读写，WAL）

account-ops :9011             — OAuth / refresh / relogin / nurture / outlook / quota-prime
```

**进度详见** [docs/35-tnexus-gptimage-gap.md](docs/35-tnexus-gptimage-gap.md)（含完成度百分比）。

### 部署（一条命令）

```bash
# Panda — 只需这一条（patch env + gateway + 刷新 JWT + pull + 重启）
export TNEXUS_ROOT=/root/TNexus
cd "$TNEXUS_ROOT" && git pull && bash deploy/panda/deploy.sh
```

**不再需要** `export_pool.sh`、`panda_setup_tnexus_env.py` 或手动 `patch_env.sh`。JWT 由 `deploy/panda/refresh_upstream_jwt.sh` 在部署时自动刷新（只改 `UPSTREAM_API_KEY`，不覆盖整个 `.env`）。

`.env` 必含：`ACCOUNTS_DB`、`SCHEDULING_STATE_FILE`、`ACCOUNT_OPS_*`、`TNEXUS_ACCOUNT_OPS_IMAGE`；`deploy.sh` 会通过 `patch_env.sh` 自动补齐缺失项。

### 刷新 worker → gateway JWT（`UPSTREAM_API_KEY`）

Worker 调 `:8014/v1/*` 需 Bearer JWT（`AUTH_MODE=jwt`）。**不是号池 ChatGPT token**；约 24h 过期，过期时报 `401 invalid session`。

```bash
python3 /root/gptimage-gateway-rs/scripts/panda_setup_tnexus_env.py
cd /root/TNexus && bash deploy/panda/deploy.sh   # 必须 force-recreate worker
```

建议 cron 每日执行上述脚本（或改 gateway `AUTH_MODE=apikey` + 固定 `GATEWAY_AUTH_KEY`）。

### 号池并发写入

8012（Python）与 TNexus（Rust `tnexus-accounts-db`）共享同一 sqlite 文件。双方均启用 **WAL** + **busy_timeout**；Rust 侧单行 upsert / delete / inflight 更新在 **事务** 内完成，避免整表覆盖。

Panda 上线后删除旧快照：`rm -f /opt/tnexus/data/pool/accounts_pool.json`，并在 `/opt/tnexus/.env` 将 `ACCOUNTS_FILE` 改为 `ACCOUNTS_DB=/gptimage/data/accounts.db`（可用 `deploy/panda/patch_env.sh`）。

### 验收

```bash
python3 /root/TNexus/scripts/prod_url_chain_test.py
curl -fsS -o /dev/null -w '%{http_code}\n' https://tnexus.relai.asia/accounts
# 登录后
curl -fsS -b /tmp/cj https://tnexus.relai.asia/api/accounts?offset=0&limit=1
```

---

## 部署铁律

1. **禁止在 Panda 上编译**任何项目  
2. **只走 Git**：本地 → commit → push → GHCR → Panda `git pull` + `deploy.sh`  
3. **禁止 scp/docker cp** 部署生产代码（**导号脚本读 sqlite 除外**）  
4. 生产 `:8012` **禁止**替换或重启（除非用户另立项）

---

## 已知问题

| 现象 | 原因 | 处理 |
|------|------|------|
| `/accounts` 404 | 旧 GHCR 镜像无静态页 | `deploy.sh` 拉最新 |
| `/api/accounts` 404 | 同上 | 同上 |
| worker 401 `invalid session` | **`UPSTREAM_API_KEY`（gateway JWT）过期**，非号池 OAuth | `panda_setup_tnexus_env.py` + `deploy.sh` |
| 8012 通、8014/TNexus 不通 | 多为 **Gateway JWT 过期** 或 pin 未对齐 | `panda_setup_tnexus_env.py` + `deploy.sh`；确认 `ACCOUNTS_DB` 指向 live db |
| 图片管理灰块（旧图） | 历史记录仅存 gateway 内存 URL | 新图已写 `inline_preview_b64`；旧图不可恢复 |
| worker env 未生效 | `docker restart` 不刷新 env | `deploy.sh` force-recreate |
| 公网 `:8014` 超时 | 仅云安全组、未开 UFW | `ufw allow 8014/tcp` |
| `helper_ok: false` | helper 未跑 | `DATA_PLANE=upstream` 时可忽略 |
| `phase_timings_ms` 空 | job 详情 API 未透出 | 见 `routes/media.rs` logs API |
| 养号/Outlook 503 | account-ops 未配或 GPTIMAGE_ROOT 不可用 | 查 `9011/health` 与容器日志 |
| 窗口预热仅 queued | account-ops 未启用 prime 服务 | 配 GATEWAY_BASE 回退或修 GPTIMAGE_ROOT |
| 调度门不生效 | gateway :8014 未更新 | 单独发布 `crates/gateway` 并重启 |

---

## 产品决策摘要

- **TNexus 顶栏**：嵌入 studio，不外跳  
- **注册机**：删除  
- **settings 全卡片 / ops-dashboard 壳**：不做  
- **号池**：OAuth、批量刷新/重登、CF 灯、分页多选、养号、IP 热力图  
- **权限**：admin/user 暂一致；额度暂不做
