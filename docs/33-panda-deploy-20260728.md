# Panda 独立部署记录（:8014）

最后更新：2026-07-28

> **红线**：生产 `:8012`（`chatgpt2api-local`）未改动。

## 本次部署结果

| 项 | 状态 |
|----|------|
| Git | `origin/main` @ `2ed60e0+` |
| 端口 | **8014**（host 进程，非 docker — GHCR 未授权） |
| gptimage DB 快照 | `data/gptimage/accounts.db`（28 账号） |
| 号池接入 | `secrets/accounts_pool.json`（20 账号）+ `pin_account.json` |
| health / capabilities | ✅ `data_plane=upstream`，`accounts=20` |
| 流式 chat | ✅ HTTP 200 |
| 生图 | ✅ HTTP 200（~46s，提示词需完整场景描述） |
| upstream-probe 生图 | ✅ `IMAGE_READY` |
| Web UI | `http://<panda>:8014/showcase` |

## 数据库复制（只读）

```bash
cd /root/gptimage-gateway-rs
bash scripts/panda_sync_gptimage_db.sh   # docker cp from chatgpt2api-local
ACCOUNTS_DB=data/gptimage/accounts.db OUT_PATH=secrets/pin_account.json \
  python3 scripts/export_pin_account.py
ACCOUNTS_DB=data/gptimage/accounts.db OUT_PATH=secrets/accounts_pool.json LIMIT=20 \
  python3 scripts/export_accounts_pool.py
```

备份目录：`data/gptimage-backup/<timestamp>/`

## 部署命令（标准链路）

```bash
# 1. GHCR 登录（一次性）
echo '<PAT read:packages>' > secrets/ghcr_token && chmod 600 secrets/ghcr_token
bash scripts/panda_deploy_independent.sh   # docker compose :8014
```

**本次例外**：GHCR `unauthorized`，使用 WSL 编译二进制 + `web/out` 经 tar 传到 Panda，host 启动：

```bash
bash scripts/panda_start_gateway_host.sh
```

## 验收

```bash
GATEWAY_LISTEN=127.0.0.1:8014 IMAGE_ENABLED=1 STREAM_ENABLED=1 \
  bash scripts/independent_acceptance.sh
```

生图 API 示例（需先 `/api/auth/login`）：

```json
{"prompt":"a red cube on a white background, product photo","model":"gpt-image-2","n":1,"size":"1024x1024","response_format":"b64_json"}
```

## 待办

- [ ] **P2.1** 子域名 `rs.gptimage.relai.asia` → Nginx → `:8014`（见 [32-independent-deploy.md](32-independent-deploy.md) §5）
- [ ] **P2.2** 生图 API 支持 `url` 响应，减轻 Panda 30Mbps 上行
- [ ] 可选：GHCR 包长期私有 + `secrets/ghcr_token`（已配置则可忽略）
- [ ] R2 canary 切流 `:8012` —— **另立项**
