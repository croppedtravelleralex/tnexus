# TNexus 三阶段上线：独立 OAuth/刷新 → Panda 导号 → URL 生图验收

最后更新：**2026-07-31**

## 阶段 1：本地独立 OAuth / 刷新

| 进程 | 端口 | 说明 |
|------|------|------|
| `account_ops_face.py` | 9011 | OAuth PKCE + refresh/relogin |
| `tnexus-api` | 9000 | 号池 API |
| `tnexus-worker` | — | Studio 生图 |

```bash
# WSL @ TNexus
bash scripts/restart-local-api.sh   # 拉起 account_ops + api + worker
```

`.env` 见根目录 `.env.example`（`ACCOUNTS_DB`、`ACCOUNT_OPS_*`、`GPTIMAGE_ROOT`）。

---

## 阶段 2：Panda 部署 + 全量号池

> 遵守部署铁律：不在 Panda 编译；号池与 `:8012` 共享 live sqlite。

### 2.1 一次性准备

```bash
ssh panda
# 克隆/更新部署仓（仅脚本与 compose，不编译）
test -d /root/TNexus/.git || git clone https://github.com/croppedtravelleralex/tnexus.git /root/TNexus
cd /root/TNexus && git pull

# 补全 /opt/tnexus/.env（对照 deploy/panda/.env.example）
# 必加：
#   ACCOUNTS_DB=/gptimage/data/accounts.db
#   SCHEDULING_STATE_FILE=/data/pool/scheduling_state.json
#   USAGE_EVENTS_FILE=/data/pool/usage_events.ndjson
#   ACCOUNT_OPS_BASE=http://127.0.0.1:9011
#   ACCOUNT_OPS_TOKEN=<随机串>
#   TNEXUS_ACCOUNT_OPS_IMAGE=ghcr.io/croppedtravelleralex/tnexus-account-ops:latest
```

### 2.2 部署

```bash
export TNEXUS_ROOT=/root/TNexus
bash /root/TNexus/deploy/panda/deploy.sh        # GHCR pull + up
# 删除旧快照（若存在）
rm -f /opt/tnexus/data/pool/accounts_pool.json
```

api/gateway 容器挂载 `/root/gptimage/data` → `/gptimage/data`，与 `chatgpt2api-local :8012` 读写同一 `accounts.db`（WAL + 事务）。

### 2.3 验收

- 浏览器：`https://tnexus.relai.asia/accounts`（应 200，非 404）
- API：登录后 `GET /api/accounts?offset=0&limit=5`
- `curl http://127.0.0.1:9011/health` on Panda

---

## 阶段 3：URL 生图 + 号池回归

### 3.1 生图链路

| 环境 | `GPTIMAGE_BASE` |
|------|-----------------|
| Panda worker | `http://127.0.0.1:8014` |
| 本地 WSL | `https://tnexus.relai.asia` 或 SSH 隧道 `127.0.0.1:18014` |

```bash
python3 scripts/prod_url_chain_test.py    # 生产全链路
# 本地经 Panda gateway：
GW_BASE=https://tnexus.relai.asia python3 scripts/run_url_chain_test.py
```

### 3.2 号池回归清单

- [ ] 分页列表、CF 灯、IP 热力图
- [ ] OAuth 新号导入
- [ ] 批量刷新（需 `refresh_token`）
- [ ] 密码重登（需导出 JSON 含 `password`）
- [ ] `POST /api/accounts/reload-from-storage`

### 3.3 失败排查

| 现象 | 检查 |
|------|------|
| `/accounts` 404 | GHCR 镜像是否最新；`GATEWAY_STATIC_DIR` |
| OAuth 502 | `account-ops` 日志；`ACCOUNT_OPS_TOKEN` 一致 |
| 刷新无额度 | `GPTIMAGE_ROOT` 挂载；JSON 含 `refresh_token` |
| 公网 8014 不通 | 云安全组 + `ufw allow 8014/tcp` |
