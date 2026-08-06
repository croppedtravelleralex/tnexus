# HANDOFF — TNexus（含 gateway-rs 合并）

最后更新：**2026-08-02（`1ab5d25`：出图元数据 + 全量同步额度 + gateway edits 已部署 Panda）**

## 读什么（按顺序）

1. **[plan.md](plan.md)** — 合并施工总控与详细待办
2. **[docs/39f-grok-progress.md](docs/39f-grok-progress.md)** — **grok2api 完整 Rust 移植进 TNexus**（号池/Admin/运维/OCR；**G2–G7 已完成并合 main**，剩余上线执行 + 运行验收）
3. **[docs/38-tnexus-production-cutover.md](docs/38-tnexus-production-cutover.md)** — **1:1 替代 gptimage 生产切流路线图**
3. **[docs/36-image-delivery-bandwidth-strategy.md](docs/36-image-delivery-bandwidth-strategy.md)** — AVIF/WebP 显示、R2/Edge 302、带宽分层
4. **[docs/37-gptimage-tnexus-comparison.md](docs/37-gptimage-tnexus-comparison.md)** — 与 Python `:8012` 横向对比
5. **[docs/34-tnexus-rollout-oauth-panda.md](docs/34-tnexus-rollout-oauth-panda.md)** — 号池/OAuth 三阶段上线
6. **[docs/SOURCE.md](docs/SOURCE.md)** — UI/API 对照源（gptimage Python 仓）
7. **[README.md](README.md)** — 本地开发与仓库结构

---

## 项目定位（合并后）

**TNexus** = 导演工作台（studio）+ **chatgpt2api 管理台**（号池/生图/运维）+ **Rust gateway**（`/v1/` 生图/对话）。

- **唯一公网入口**：`https://tnexus.relai.asia`
- **号池页**：`https://tnexus.relai.asia/accounts`（需最新 GHCR 镜像）
- **不接** Panda 生产 Python `chatgpt2api-local :8012` 的管理 HTTP API（对照实现，不运行时依赖）
- **禁止** `GPTIMAGE_ADMIN_TOKEN` 代理生产 `:8012`；运维走 **account-ops `:9011` + `GPTIMAGE_ROOT`**
- **刷新/重登/OAuth/养号/Outlook/窗口预热** 均走 account-ops（本地 Python 库），不走 gptimage HTTP

---

## 当前状态（2026-08-02）

**生产镜像**：`main` @ **`1ab5d25`**

| 批次 | commit | 内容 |
|------|--------|------|
| Studio UX + 对话 | `b8d6fa8` … `9e8105b` | size/风格、热力图、对话 SSE 代理、`GATEWAY_AUTH_KEY` |
| 出图元数据 + 号池 | `0bc7463` … `9248d34` | `job_results` 宽高/字节数；无选中时「同步全部额度」 |
| gateway edits | `1ab5d25` | `POST /v1/images/edits` upstream；URL 模式 worker 元数据 |

### 已完成（2026-08-01 晚 — 已部署 Panda）

| 项 | 状态 |
|----|------|
| 工作台出图角标：分辨率 + 文件大小 | ✅ migration `008` + `output-panel`；无 DB 字段时前端探测 |
| 号池无选中 → 工具栏「同步全部额度」（`refresh-all`） | ✅ `accounts/page.tsx` |
| Worker URL 模式写入 `width/height/size_bytes` | ✅ 无 R2 时仍下载 `source_url` 落库 |
| Gateway `POST /v1/images/edits`（upstream 上传 + multimodal） | ✅ `1ab5d25`；需 `IMAGE_ENABLED=1` + `DATA_PLANE=upstream` |
| `capabilities.image_edits` | ✅ gateway `:8014` 返回 `true` |

### 已完成（2026-08-01 昼 — 已部署 Panda）

| 项 | 状态 |
|----|------|
| 工作台 size/quality/风格 hint 全链路 | ✅ `b8d6fa8` |
| 风格预设分类 + 子项 + 一键展开/收起 | ✅ |
| 演员张数 1–9 + ≥10 输入；润色因子滑条 | ✅ |
| 号池额度：调度中+正常 → 绿色 badge | ✅ API `image_quota_state` + 前端 |
| IP 热力图 binding 对齐 | ✅ `usage_metrics` + worker 空 binding |
| 日志阶段含 `ps_ms` +「其他」补差 | ✅ |
| 对话 `POST /api/chat/completions`（SSE 代理 gateway） | ✅ |
| `GATEWAY_AUTH_KEY` 与 `UPSTREAM_API_KEY` 同步刷新 | ✅ `9e8105b` `refresh_upstream_jwt.sh` |
| 生产冒烟（生图 / 号池 / 对话流式 / 热力图） | ✅ 2026-08-01 |

### 已完成（较早迭代）

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
| GHCR 拉取最新镜像（api + worker + account-ops + gateway + 静态 UI） | ✅ 2026-08-02 `1ab5d25` |
| gateway `:8014` `tnexus-gateway` | ✅ 与 TNexus `crates/gateway` 同步 |
| `UPSTREAM_API_KEY` 定期刷新（cron） | ⚠️ 需运维；TTL ≈24h，过期报 `invalid session` |
| 号池与 8012 共享 live `accounts.db`（WAL + 事务写入） | ✅ `tnexus-accounts-db` |
| Studio 生图轮询改轻量 `/api/jobs/{id}/status`（不再每 2s 拉全量 b64） | ✅ |
| 生成记录/结果预览改 thumb API（左侧列表不再内联 MB 级 data URL） | ✅ |
| 生图进行中计时器实时刷新（不再固定显示「1秒」） | ✅ |
| `pin_account.json` 与 pool/sqlite 同步 | ⚠️ 曾过期；刷新后需与 pool 对齐 |
| Outlook 恢复 UI、养号结果 merge 回 JSON | 📋 下一迭代 |
| 对话多轮持久化 / 对话生图 | 📋 Phase 1（见 [doc 38](docs/38-tnexus-production-cutover.md)） |
| gateway edits 生产 E2E 脚本 | 📋 待补（capabilities 已 true；`prod_url_chain_test` 仅文生图） |
| `:8012` vs `:8014` 同 prompt 像素级对比 | 📋 Phase 0 剩余 |
| 10 并发压测 | 📋 Phase 0 剩余 |
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

**不再需要** `export_pool.sh`、`panda_setup_tnexus_env.py` 或手动 `patch_env.sh`。JWT 由 `deploy/panda/refresh_upstream_jwt.sh` 在部署时自动刷新（只改 `UPSTREAM_API_KEY` + **`GATEWAY_AUTH_KEY`**（对话代理用），不覆盖整个 `.env`）。

`.env` 必含：`ACCOUNTS_DB`、`SCHEDULING_STATE_FILE`、`ACCOUNT_OPS_*`、`TNEXUS_ACCOUNT_OPS_IMAGE`、`GATEWAY_BASE=http://127.0.0.1:8014`；`deploy.sh` 会通过 `patch_env.sh` 自动补齐缺失项。

### 刷新 worker → gateway JWT

**已并入 `deploy.sh`**，无需单独操作。若仅 JWT 过期、不想全量重建：

```bash
bash /root/TNexus/deploy/panda/refresh_upstream_jwt.sh
cd /root/TNexus && docker compose --env-file /opt/tnexus/.env -f deploy/panda/docker-compose.yml up -d --force-recreate worker
```

**禁止**再跑 `panda_setup_tnexus_env.py`（会整文件覆盖 `.env`）。

### 号池并发写入

8012（Python）与 TNexus（Rust `tnexus-accounts-db`）共享同一 sqlite 文件。双方均启用 **WAL** + **busy_timeout**；Rust 侧单行 upsert / delete / inflight 更新在 **事务** 内完成，避免整表覆盖。

Panda 上线后删除旧快照：`rm -f /opt/tnexus/data/pool/accounts_pool.json`，并在 `/opt/tnexus/.env` 将 `ACCOUNTS_FILE` 改为 `ACCOUNTS_DB=/gptimage/data/accounts.db`（可用 `deploy/panda/patch_env.sh`）。

### 验收

```bash
# 生图 E2E（导演模式，需可出网）
python3 /root/TNexus/scripts/prod_url_chain_test.py

# Studio UX 全项（尺寸/竞演/排队/额度/对话 SSE/日志阶段；建议在 Panda 上跑，避免本地 SSL EOF）
python3 /root/TNexus/scripts/test_ux_coverage.py

# 导演 vs 竞演模式
python3 /root/TNexus/scripts/test_studio_modes.py

curl -fsS https://tnexus.relai.asia/health
curl -fsS http://127.0.0.1:9000/health
curl -fsS http://127.0.0.1:8014/health
curl -fsS -o /dev/null -w 'accounts_page=%{http_code}\n' https://tnexus.relai.asia/accounts/
```

**2026-08-02 已验**：`prod_url_chain_test.py` OK；`job_results` 最新行含元数据（例 `1254×1254 · 1.27MB`）；`curl :8014/api/backend/capabilities` → `image_edits: true`。

**2026-08-01 已验**：health 全绿；16:9(4k) 与 1:1 出图宽高比正确；竞演双 provider；排队 <30s 无黄字；40/40 调度账号绿 badge；热力图有 binding；对话 SSE + UI 流式正常；日志 `wall_clock_ms` 与阶段之和比 ≈1.01。

---

## 部署铁律

1. **禁止在 Panda 上编译/构建**（`cargo` / `docker build` / `npm build` / `buildx`）— **违反曾导致 CPU/内存爆满（重大事故）**
2. **只走 Git + GHCR**：本地或 CI 构建 → commit → push → Actions → Panda **仅** `deploy.sh`（pull + up）
3. **禁止 scp/docker cp** 部署生产代码（**导号脚本读 sqlite 除外**）
4. 生产 `:8012` **禁止**替换或重启（除非用户另立项）
5. Cursor 强制规则：`.cursor/rules/panda-no-remote-build.mdc`（`alwaysApply: true`）

---

## 已知问题

| 现象 | 原因 | 处理 |
|------|------|------|
| `/accounts` 404 | 旧 GHCR 镜像无静态页 | `deploy.sh` 拉最新 |
| `/api/accounts` 404 | 同上 | 同上 |
| worker 401 `invalid session` | **`UPSTREAM_API_KEY`（gateway JWT）过期** | `bash deploy/panda/deploy.sh`（自动刷新） |
| 对话 `/api/chat/completions` 401 `login required`（JSON） | API 容器缺 **`GATEWAY_AUTH_KEY`**，gateway 拒 Bearer | `refresh_upstream_jwt.sh`（`9e8105b` 起与 `UPSTREAM_API_KEY` 同步）；`docker compose … up -d --force-recreate api` |
| 8012 通、8014/TNexus 不通 | 多为 **Gateway JWT 过期** 或 pin 未对齐 | `panda_setup_tnexus_env.py` + `deploy.sh`；确认 `ACCOUNTS_DB` 指向 live db |
| 图片管理灰块（旧图） | 历史记录仅存 gateway 内存 URL | 新图已写 `inline_preview_b64`；旧图不可恢复 |
| worker env 未生效 | `docker restart` 不刷新 env | `deploy.sh` force-recreate |
| 公网 `:8014` 超时 | 仅云安全组、未开 UFW | `ufw allow 8014/tcp` |
| `helper_ok: false` | helper 未跑 | `DATA_PLANE=upstream` 时可忽略 |
| `phase_timings_ms` 空 | job 详情 API 未透出 | 已合入 `GET /api/jobs/{id}`；见 pipeline_events |
| thumb 展示流量大 | `/api/images/thumb` 302 到全 PNG | 见 [docs/36](docs/36-image-delivery-bandwidth-strategy.md) §2 |
| Panda 出口带宽 | 用户看图经 asset | R2 + WebP/AVIF；见 [docs/36](docs/36-image-delivery-bandwidth-strategy.md) |
| 养号/Outlook 503 | account-ops 未配或 GPTIMAGE_ROOT 不可用 | 查 `9011/health` 与容器日志 |
| 窗口预热仅 queued | account-ops 未启用 prime 服务 | 配 GATEWAY_BASE 回退或修 GPTIMAGE_ROOT |
| 调度门不生效 | gateway :8014 未更新 | 单独发布 `crates/gateway` 并重启 |
| Studio 一直「1秒」/ 55% 假死 | 轮询 `GET /api/jobs/{id}` 返回全量 inline b64，JSON 过大超时；计时器未 tick | 已改 status 轮询 + thumb API；2K 竞演仍可能需 2–5 分钟 |
| 页面/切换/列表加载慢 | 生成记录 API 内联 100 条 b64 缩略图；静态导出整页跳转 | 已改 thumb API；首屏仍受静态导出影响 |

---

## 产品决策摘要

- **TNexus 顶栏**：嵌入 studio，不外跳  
- **注册机**：删除  
- **settings 全卡片 / ops-dashboard 壳**：不做  
- **号池**：OAuth、批量刷新/重登、CF 灯、分页多选、养号、IP 热力图  
- **权限**：admin/user 暂一致；额度暂不做
