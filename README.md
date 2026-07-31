# TNexus

AI 生图工作台 + **chatgpt2api 管理台**（合并中）— Rust 控制面（tnexus-api + gateway）+ Next.js UI。内网生图走 gateway `/v1/`，资产可存 Cloudflare R2。

> **施工总控**：[plan.md](plan.md) · **当前状态**：[HANDOFF.md](HANDOFF.md) · **UI 对照源**：[docs/SOURCE.md](docs/SOURCE.md)

## 功能

- 注册 / 登录 / 用户管理（admin 可禁用用户）
- 导演模式 / 竞演模式
- A类全 Agent / B类关键词 PS 工作流
- 导演因子 + PS 因子二维可视化
- PS 开关（B类强制开启）
- 异步生图 + SSE 进度
- R2 三级存储（原图 / 预览 / 缩略图）+ 签名 URL 下载

## 本地开发

### 依赖

- Docker（PostgreSQL + Redis）
- Rust（WSL 或本机）
- Node.js 22+

### 启动

```bash
cp .env.example .env
docker compose up -d postgres redis

# WSL
cd /mnt/d/SelfMadeTool/TNexus
cargo run -p tnexus-api
cargo run -p tnexus-worker   # 另一个终端

cd web && npm run dev
```

- 前端：http://localhost:3000
- API：http://localhost:9000/health

### R2 配置

1. Cloudflare → R2 → 创建 `tnexus-assets`
2. 创建 API Token（Object Read & Write）
3. 写入 `.env`：`R2_ACCOUNT_ID`、`R2_ACCESS_KEY_ID`、`R2_SECRET_ACCESS_KEY`、`R2_BUCKET`

未配置 R2 时任务可完成但无预览/下载 URL。

## Panda 部署

```bash
# 本地禁止在 Panda 上 cargo/npm build
# Panda:
export TNEXUS_ROOT=/root/TNexus
cd $TNEXUS_ROOT && git pull
bash deploy/panda/deploy.sh
```

- 数据：`/opt/tnexus/.env`、`/opt/tnexus/data/pool/`（调度状态/用量事件）、`/root/gptimage/data/accounts.db`（号池真源，8012 与 TNexus 共享）
- 域名：`https://tnexus.relai.asia`（号池 `/accounts`，生图 `/v1/` 反代 gateway）
- 同机回环：`GPTIMAGE_BASE=http://127.0.0.1:8014`
- 详见 [HANDOFF.md](HANDOFF.md)、[docs/34-tnexus-rollout-oauth-panda.md](docs/34-tnexus-rollout-oauth-panda.md)

## 仓库结构（目标，见 plan.md P0）

```
crates/tnexus-api      # axum HTTP API（/api/jobs、/api/auth）
crates/tnexus-worker   # 异步任务消费者
crates/tnexus-domain   # 导演/因子/任务模型
crates/tnexus-auth     # JWT + argon2
crates/tnexus-storage  # R2 + 图片变体
crates/gateway         # ← 自 gptimage-gateway-rs 迁入（/v1/ + 号池 API）
crates/auth            # gateway 鉴权（待与 tnexus-auth 收敛）
crates/protocol        # OpenAI 契约
crates/upstream        # TLS/SSE 数据面
web/                   # 单一 Next.js：studio + chatgpt2api 管理台
migrations/            # PostgreSQL
deploy/panda/          # 生产 compose
deploy/nginx/          # tnexus.relai.asia 反代样例
docs/                  # 含自 gateway-rs 同步的 gap/部署文档
plan.md                # 合并详细待办
```
