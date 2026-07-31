# HANDOFF — TNexus（含 gateway-rs 合并）

最后更新：**2026-07-31**

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
- **刷新/重登** 走 `account-ops`（`:9011`）+ `GPTIMAGE_ROOT` Python 库，不走 gptimage HTTP

---

## 当前状态（2026-07-31）

### 已完成

| 项 | 状态 |
|----|------|
| TNexus API + worker + Postgres + Redis | ✅ Panda `/opt/tnexus` |
| Gateway `:8014` + UFW/安全组放行 | ✅ 公网与回环均可 |
| nginx `tnexus.relai.asia` | ✅ `/v1/`、`/api/backend/` → 8014；其余 → 9000 |
| URL 生图（域名） | ✅ `GPTIMAGE_BASE=https://tnexus.relai.asia` 或回环 `127.0.0.1:8014` |
| 号池 API `/api/accounts` | ✅ 代码已合入；**待 GHCR 部署** |
| 号池 UI `/accounts` | ✅ 静态导出；**待 GHCR 部署** |
| OAuth / 刷新 / 重登 | ✅ `account-ops` 镜像 + `:9011` |
| 本地验收 | ✅ 38 账号导入、`TNEXUS_URL_CHAIN_OK` |

### 进行中

| 项 | 状态 |
|----|------|
| gateway-rs 物理归档 | 已迁入 `crates/gateway`，待删独立仓 |
| settings 七卡片 | 明确不做 |
| 完整 ops-dashboard | 明确不做（仅双槽/养号/埋点子集） |

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
  ├─ /v1/*              → 127.0.0.1:8014  (gptimage-gateway-rs)
  ├─ /api/backend/*     → 127.0.0.1:8014
  └─ /*                 → 127.0.0.1:9000  (tnexus-api + 静态 UI)

/opt/tnexus/.env              — secrets + compose env
/opt/tnexus/data/pool/        — accounts_pool.json（api 挂载 /data/pool）
/root/TNexus/                 — git 仓（deploy 脚本，不在此编译）
/root/gptimage/               — refresh/relogin Python 库（account-ops 只读挂载）

account-ops :9011             — OAuth / refresh / relogin
```

### 部署（只读导号 + GHCR）

```bash
# Panda
export TNEXUS_ROOT=/root/TNexus
cd "$TNEXUS_ROOT" && git pull
bash deploy/panda/export_pool.sh          # sqlite → /opt/tnexus/data/pool/
bash deploy/panda/deploy.sh               # pull + up api worker account-ops
```

`.env` 必含：`ACCOUNTS_FILE`、`ACCOUNT_OPS_*`、`TNEXUS_ACCOUNT_OPS_IMAGE`（见 `deploy/panda/.env.example`）。

### 刷新 worker 上游 token

```bash
python3 /root/gptimage-gateway-rs/scripts/panda_setup_tnexus_env.py
cd /root/TNexus && bash deploy/panda/deploy.sh
```

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
| worker 401 生图 | `docker restart` 不刷新 env | `deploy.sh` force-recreate |
| 公网 `:8014` 超时 | 仅云安全组、未开 UFW | `ufw allow 8014/tcp` |
| `helper_ok: false` | helper 未跑 | `DATA_PLANE=upstream` 时可忽略 |
| `phase_timings_ms` 空 | job 详情 API 未透出 | 见 `routes/media.rs` logs API |

---

## 产品决策摘要

- **TNexus 顶栏**：嵌入 studio，不外跳  
- **注册机**：删除  
- **settings 全卡片 / ops-dashboard 壳**：不做  
- **号池**：OAuth、批量刷新/重登、CF 灯、分页多选、养号、IP 热力图  
- **权限**：admin/user 暂一致；额度暂不做
