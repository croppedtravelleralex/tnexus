# SOURCE — UI / API 对照基准

本仓管理台 UI 与号池 API，以 **Panda 正在运行的 gptimage 生产仓** 为对照实现（抄/借鉴/对比），**合并后不在运行时依赖** Panda `:8012`。

---

## 对照源路径（权威）

| 环境 | 路径 | 角色 |
|------|------|------|
| **生产源码（主对照）** | `D:\SelfMadeTool\AutoRegister\gptimage\` | Panda `chatgpt2api-local` 对应树；**前端与 API 有需要从此抄/借鉴/对比** |
| **Web UI** | `gptimage/web/src/` | 顶栏、号池、流水图、日志、运维、对话、设置 |
| **HTTP API** | `gptimage/api/` | `accounts.py`、`ai.py`、`ops.py`、`system.py`、`image_tasks.py` 等 |
| **生产进程** | Panda `:8012` → `gptimage.relai.asia` | 只读对照行为；TNexus 合并版走 Rust gateway |
| **gateway-rs（迁入 TNexus）** | `TNexus/crates/gateway*` | `/v1/` 生图 + 待补齐 `/api/accounts/*` |

> 本地路径与 Panda 挂载 `/app` 同源；改 UI 前先 diff `gptimage/web`，改 API 前先读 `gptimage/api` 对应路由。

---

## UI 关键文件（迁移清单）

| gptimage 路径 | 用途 |
|---------------|------|
| `web/src/components/top-nav.tsx` | 顶栏；**无限画布 → TNexus 内嵌 studio** |
| `web/src/app/accounts/page.tsx` | 号池管理主页面（3000+ 行） |
| `web/src/app/accounts/accounts-activity-panels.tsx` | 账号流水双图 |
| `web/src/app/accounts/components/*` | 导入对话框等 |
| `web/src/components/accounts/*` | 热力图、CF 灯、调度图标 |
| `web/src/lib/api.ts` | 管理 API 客户端（需改 baseURL 指向 gateway） |
| `web/src/app/image/*` | 生图工作台 |
| `web/src/app/logs/*` | 日志 |
| `web/src/app/ops/*` | 运维 |
| `web/src/app/settings/*` | 设置（剔除注册机/无限画布卡片） |

### 明确不迁移

| 项 | 原因 |
|----|------|
| `web/src/app/register/**` | 注册机 — 产品决策删除 |
| `third-party-apps-card` 无限画布段 | 改为 TNexus 内嵌 |
| CPA / sub2api / Panda sync UI | 永久非目标 |

---

## API 对照（号池第一阶段）

实现目标：**Rust gateway** `crates/gateway`（合并后路径），行为对齐 `gptimage/api/accounts.py`。

| 优先级 | 端点 | 号池页用途 |
|--------|------|------------|
| P0 | `GET /api/accounts` | 列表 + stats |
| P0 | `GET /api/accounts/activity/daily` | 流水折线图 |
| P0 | `POST /api/accounts/scheduling` | 调度开关 |
| P0 | `POST /api/accounts/scheduling/bulk` | 全选调度 |
| P1 | `POST /api/accounts/import-batch` | 导入 |
| P1 | `POST /api/accounts/refresh` | 刷新额度 |
| P1 | `POST /api/accounts/update` | 编辑 |
| P2 | `POST /api/accounts/re-login` | OAuth 重登 |

完整 gap 见 [24-gap-inventory.md](24-gap-inventory.md) §1.4（26 端点，22 缺失）。

---

## Grok / grok2api 对照源（独立于 gptimage）

| 环境 | 路径 | 角色 |
|------|------|------|
| **grok2api Go 仓** | `D:\SelfMadeTool\AutoRegister\grokImage\` | Grok 网关 + 号池 + Admin；**Rust 全量移植源** |
| **移植规划** | [39-grok2api-rust-migration.md](39-grok2api-rust-migration.md) | crate 拆分、Phase、schema、工时 |
| **生产（已停）** | Panda `/opt/grok2api/` | compose + `data/backend.db`；ETL 源 |

与 gptimage **不共用**号池 schema；TNexus worker 当前仅 `GROK2API_BASE` HTTP 客户端（生图）。

---

## 与 TNexus 原有模块关系

| 模块 | 合并后职责 |
|------|------------|
| `tnexus-api` | `/api/auth`、`/api/jobs`、用户注册登录 |
| `tnexus-worker` | 导演任务队列 → 调 gateway `/v1/images/generations` |
| `gateway` | `/v1/*` + **新增** `/api/accounts/*` 等管理 API |
| `web/` | 单一 Next：studio + chatgpt2api 管理台 |

鉴权：**统一 TNexus JWT**；gateway 校验同一 issuer/secret（见 plan.md P1-8）。

---

## ⚠️ 安全

`gptimage` 树内可能含 `config.json` 凭据、代理口令 — **勿提交、勿复制到 TNexus git**。对照时只读代码与路由形状。
