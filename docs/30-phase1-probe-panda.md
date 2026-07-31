# 探针验证记录（Panda 一次性；:8013 MVP 已退役）

最后更新：2026-07-28

> **策略**：后续以**本地 WSL 实现**为主；Panda `:8013` gateway/helper **已停服**。
> 完整后在**独立部署**上线，不替换生产 `:8012`。

## 已验证结果（2026-07-28 Panda 实网）

账号来源：Panda 号池导号（`verified_ready` + 绑定代理）。探针：`PROBE_STEPS=requirements,image`。

```
REQUIREMENTS_OK token_len=3000 proof_len=651 turnstile_len=2392
IMAGE_PREPARE_OK conduit_len=351
IMAGE_READY conversation_id=6a68726d-... file_ids=file_00000000f99c81f5... events=8 elapsed≈40s
```

| 判据 | 状态 |
|------|------|
| Sentinel 开票链 | ✅ |
| 生图 prepare/start body | ✅ |
| SSE → `file_id` / `sediment_id` | ✅ |
| 文本 SSE（`PROBE_STEPS=sse`） | ⏳ 待补跑 |

> GHCR 镜像已 publish；Panda `docker pull` 需配置 `ghcr.io` 登录。探针亦可经 WSL 编译二进制 pipe 至 Panda `/tmp/upstream-probe`（一次性诊断，非 gateway 部署）。

## 目标（第一期判据）

1. Sentinel 开票链：`prepare` → PoW → Turnstile → `finalize`
2. 文本 SSE：`POST /backend-api/f/conversation` 解析到 **text ready**（`conversation_id` 或 delta）
3. **不要求**完整出图（第二期 `PROBE_STEPS=image` 单独验证）

## 前置

- **仅 Panda 导号**：号池账号 + 对应代理导出为 `pin_account.json`（`scripts/export_pin_account.py`，禁止本地注册/手工拼 token）
- 镜像含 `upstream-probe`（`Dockerfile.gateway` 已构建）
- 部署链路：`git push` → GitHub Actions → GHCR → Panda `git pull` + `docker pull`

## Panda 操作步骤

### 1. 导出 pin 账号（Panda 只读，唯一来源）

```bash
# 在 panda 上；ACCOUNTS_DB 默认 chatgpt2api-local 容器内 /app/data/accounts.db
mkdir -p /root/gptimage-gateway-rs/secrets
ACCOUNTS_DB=/app/data/accounts.db \
PIN_EMAIL='<pin-email>' \
OUT_PATH=/root/gptimage-gateway-rs/secrets/pin_account.json \
  docker exec -i chatgpt2api-local python3 - < /root/gptimage-gateway-rs/scripts/export_pin_account.py
```

### 2. ~~拉取镜像并启动 gateway（:8013）~~ — 已取消

`:8013` MVP 已退役。GHCR 镜像仅作 CI 产物保留；不在 Panda 上跑 gateway。

本地开发：

```bash
bash scripts/local_bringup_wsl.sh
```

### 3. 跑第一期文本探针

```bash
docker compose -f deploy/test-compose.example.yml --profile probe run --rm upstream-probe
```

或进入已运行的 gateway 容器：

```bash
docker exec -e PIN_ACCOUNT_FILE=/secrets/pin_account.json \
  -e PROBE_STEPS=requirements,sse \
  -e PROBE_PROMPT='Reply with one short word.' \
  -e PROBE_SSE_TIMEOUT_SECS=120 \
  -e RUST_LOG=info \
  <gateway-container> \
  /usr/local/bin/upstream-probe
```

挂载 `pin_account.json` 到容器内 `/secrets/pin_account.json`（与 compose 中 gateway 一致）。

### 4. 期望输出

```
REQUIREMENTS_OK token_len=... proof_len=... turnstile_len=...
SSE_READY conversation_id=... saw_delta=true events=...
```

失败时检查：

| 症状 | 可能原因 |
|------|----------|
| `chat_requirements_turnstile_required_but_unsolved` | Turnstile VM 回归；CI `cargo test -p upstream turnstile` |
| `conversation HTTP 403` | CF / TLS 指纹 / 代理失效 |
| `sse ended before text ready` | 账号额度、模型限制或 SSE 解析缺口 |
| `bootstrap` 失败 | 代理或 chatgpt.com 不可达 |
| `NO_CANDIDATE`（导号） | 号池无 `verified_ready`/`verified` 或代理缺失 |

### 5. 生图探针（第二期）

```bash
docker run --rm --network host \
  -v /root/gptimage-gateway-rs/secrets/pin_account.json:/secrets/pin_account.json:ro \
  -e PIN_ACCOUNT_FILE=/secrets/pin_account.json \
  -e PROBE_STEPS=requirements,image \
  -e PROBE_IMAGE_PROMPT='a red cube on white background' \
  -e PROBE_IMAGE_TIMEOUT_SECS=300 \
  -e RUST_LOG=info \
  ghcr.io/croppedtravelleralex/gptimage-gateway-rs:latest \
  /usr/local/bin/upstream-probe
```

期望：`IMAGE_PREPARE_OK` → `IMAGE_READY ... file_ids=file_...`

### 6. 可选：全步骤（含 TLS）

```bash
PROBE_STEPS=tls,bootstrap,requirements,sse FP_ENDPOINT=https://tls.browserleaks.com/json \
  upstream-probe
```

## 相关文件

| 文件 | 说明 |
|------|------|
| `crates/upstream-probe/` | 探针二进制 |
| `crates/upstream/` | 数据面库（PoW/Turnstile/SSE/requirements） |
| `Dockerfile.gateway` | 同时打包 gateway + upstream-probe |
| `.github/workflows/publish-gateway.yml` | push `main` → GHCR `ghcr.io/croppedtravelleralex/gptimage-gateway-rs` |
| `scripts/export_pin_account.py` | Panda 导号 |
| `fixtures/protocol/image_*.json` | 生图 body 契约 |
