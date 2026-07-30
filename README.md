# TNexus

AI 生图工作台 — Rust 控制面 + Next.js 墨绿 UI。内网调用 gptimage / grok2api，资产存储 Cloudflare R2。

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
# 本地/WSL 禁止在 Panda 上 cargo build
docker compose -f deploy/panda/docker-compose.yml pull
docker compose -f deploy/panda/docker-compose.yml up -d
```

- 路径：`/opt/tnexus`
- 域名：`https://tnexus.closeapi.top`
- 内网：`GPTIMAGE_BASE=http://127.0.0.1:8012`，`GROK2API_BASE=http://127.0.0.1:18000`

## 仓库结构

```
crates/tnexus-api      # axum HTTP API
crates/tnexus-worker   # 异步任务消费者
crates/tnexus-domain   # 导演/因子/任务模型
crates/tnexus-auth     # JWT + argon2
crates/tnexus-storage  # R2 + 图片变体
web/                   # Next.js 前端
migrations/            # PostgreSQL
deploy/panda/          # 生产 compose
```
