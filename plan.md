# TNexus 合并施工总控 · gateway-rs + gptimage 管理台

最后更新：**2026-07-30**  
状态：**规划阶段** — 仓库合并与 UI 迁移尚未开工  
决策记录：见 [HANDOFF.md](HANDOFF.md) · UI 对照源见 [docs/SOURCE.md](docs/SOURCE.md)

---

## 0. 目标与非目标

### 目标

| # | 目标 | 验收 |
|---|------|------|
| G1 | 将 `gptimage-gateway-rs` **物理并入**本仓，**只保留 TNexus 一个 Git 仓库** | `cargo build --workspace` 含 gateway + tnexus 全部 crate；`gptimage-gateway-rs` 归档 |
| G2 | 管理台 UI 复刻 Python 生产仓 `gptimage/web`（chatgpt2api + 号池 + 账号流水图） | 与 `gptimage.relai.asia/accounts` 功能对齐（第一阶段） |
| G3 | **不接** Panda 生产 Python `:8012`；号池与管理 API **在 Rust gateway 内实现** | 无对 `chatgpt2api-local` 的运行时依赖 |
| G4 | 鉴权统一为 **TNexus + gateway 独立体系**（`admin/user` 123456 + 注册） | 单登录进入工作台与管理台 |
| G5 | 顶栏 **TNexus** = 同一壳内 **嵌入导演工作台**（`/studio`），非外跳 | 点 TNexus 不换布局壳、不新开标签 |
| G6 | 单域部署 **`https://tnexus.relai.asia`** | nginx：`/v1/` → gateway；TNexus API + 合并后 Web → `:9000` |
| G7 | 视觉：**shadcn 灰白**、字体层级、卡片 **阴影立体感** | 管理台与 studio 同设计语言 |

### 非目标（本波不做）

| 项 | 说明 |
|----|------|
| 生产 `:8012` 切流 / 替换 | Panda `chatgpt2api-local` **不动** |
| 注册机 | 顶栏与路由 **删除**；号源仍外置 |
| CPA 池 / sub2api / Outlook OTP / 维护环 UI | 归入永久非目标（与 [gap 清单](docs/24-gap-inventory.md) 一致） |
| 无限画布三方跳转 | 改为 TNexus 内嵌 studio，**不**带 apiKey 外跳 |
| 用户生图额度 | 暂不做 |
| 管理员 vs 普通用户权限差 | 暂一致 |

### 红线

- **禁止**在 Panda 上 `cargo build` / `docker build` / `npm run build`  
- 发布链路：**本地改测 → git commit → push → GHCR → Panda pull + compose up**  
- **禁止** `scp` / `docker cp` 绕过 git 部署生产代码  
- 生产 `:8012` 只读诊断不受限  

---

## 1. 已确认的产品决策（2026-07-30）

| 问题 | 决议 |
|------|------|
| 管理台后端接哪？ | **Rust gateway 逐步补齐**（不接 Python 生产 API） |
| 登录体系 | **TNexus + gateway 独立**；不动生产 chatgpt2api API Key 体系 |
| 仓库结构 | **彻底合并** monorepo（耦合业务、crate 边界解耦） |
| TNexus 顶栏入口 | **嵌入 studio**（同壳，非链接跳转） |
| 第一阶段范围 | **号池管理页完整可用**（列表/调度/流水图/导入导出等） |
| Git | **仅 TNexus 一仓**；`gptimage-gateway-rs` 归档 |

---

## 2. 目标架构（合并后）

```text
tnexus.relai.asia
├── /studio, /history, …          → TNexus Web（Next，单 app）
├── /accounts, /image, /logs, …     → 同上（gptimage 管理页迁入）
├── /api/auth, /api/jobs, …         → tnexus-api :9000
├── /api/accounts, /api/settings…   → gateway :8014（新增/迁移）
└── /v1/*                           → gateway :8014（生图/对话）
```

### 2.1 目标目录结构

```text
TNexus/
├── plan.md                    # 本文件
├── HANDOFF.md
├── Cargo.toml                 # workspace：tnexus-* + gateway-*
├── crates/
│   ├── tnexus-api/
│   ├── tnexus-worker/
│   ├── tnexus-auth/
│   ├── tnexus-domain/
│   ├── tnexus-storage/
│   ├── gateway/               # ← 自 gptimage-gateway-rs
│   ├── auth/                  # gateway JWT/SQLite（与 tnexus-auth 后续可收敛）
│   ├── protocol/
│   ├── upstream/
│   ├── helper_client/
│   └── …
├── web/                       # 单一 Next.js：TNexus studio + chatgpt2api 管理台
├── helper/                    # protocol_bridge（可选，DATA_PLANE=upstream 时可不跑）
├── deploy/
│   ├── panda/                 # 合并后 compose + nginx 样例
│   └── nginx/
├── docs/                      # 自 gateway-rs 同步 + TNexus 专属
├── migrations/                # Postgres（TNexus）
└── scripts/
```

### 2.2 UI 顶栏（目标）

| 顺序 | 标签 | 路由 | 说明 |
|------|------|------|------|
| 1 | **TNexus** | 壳内嵌入 `/studio` | 替代原「无限画布」 |
| 2 | 生图 | `/image` | gptimage 生图工作台 |
| 3 | 号池管理 | `/accounts` | 完整号池页 |
| — | ~~注册机~~ | — | **删除** |
| 4 | 图片管理 | `/image-manager` | |
| 5 | 日志管理 | `/logs` | |
| 6 | 运维 | `/ops` | |
| 7 | 对话 | `/chat` | |
| 8 | 设置 | `/settings` | |

---

## 3. 阶段划分

| 阶段 | 名称 | 出口判据 |
|------|------|----------|
| **P0** | 文档与仓库合并 | 本仓可编译；gateway crate 在 TNexus 内；CI 绿 |
| **P1** | Web 壳 + 设计系统 | 顶栏/布局/shadcn 灰白；TNexus 嵌入 studio；无注册机 |
| **P2** | 号池 API（Rust） | 号池页依赖的 `/api/accounts/*` 端点可用 |
| **P3** | 号池 UI 迁完 | 列表/统计卡/流水图/调度/导入导出与 gptimage 对齐 |
| **P4** | 其余管理页 | image / logs / ops / chat / settings |
| **P5** | 部署与验收 | Panda 单域；`prod_url_chain_test.py` + 号池冒烟 |

---

## 4. 详细待办（Checklist）

### P0 — 文档与仓库合并

- [ ] **P0-1** 将 `D:\SelfMadeTool\AutoRegister\gptimage-gateway-rs` 迁入 `TNexus/crates/*`、`helper/`、`deploy/`、`scripts/`（保留 git 历史可选：subtree/filter-repo）
- [ ] **P0-2** 合并根 `Cargo.toml` workspace members（gateway + tnexus 全部 crate）
- [ ] **P0-3** 解决 crate 命名冲突（`auth` vs `tnexus-auth` → 重命名为 `gateway-auth` 或 feature 门控）
- [ ] **P0-4** 同步 gateway 文档到 `TNexus/docs/`（见 [docs/README.md](docs/README.md) 索引）
- [ ] **P0-5** 更新 `README.md`、`HANDOFF.md`、`.env.example`
- [ ] **P0-6** 合并 CI：`.github/workflows/`（fmt/clippy/test + GHCR 单镜像或双镜像决策）
- [ ] **P0-7** 合并 `Dockerfile`：tnexus-api + tnexus-worker + gateway 二进制（或 sidecar 容器方案文档化）
- [ ] **P0-8** 归档远程 `gptimage-gateway-rs` 仓库（README 指向 TNexus）
- [ ] **P0-9** `cargo build --workspace` / `cargo test --workspace` 全绿

### P1 — Web 壳与设计系统

- [ ] **P1-1** 以 `AutoRegister/gptimage/web` 为 UI **对照源**（只读复制，不运行时依赖 Python）
- [ ] **P1-2** 合并为 **单一** `web/`：保留 TNexus `studio`/`history`，迁入 gptimage 管理路由
- [ ] **P1-3** 统一 `top-nav.tsx`：删除注册机；**TNexus** 槽位 → 壳内渲染 `studio`（iframe 或 layout slot，禁止 `window.open`）
- [ ] **P1-4** 建立 shadcn 灰白主题：`globals.css`、CSS 变量、圆角/阴影 token（立体卡片 `shadow-md` + 浅灰边框）
- [ ] **P1-5** 字体：与 TNexus 现用 sans 对齐（Geist/Inter），统一 `text-sm` / `font-medium` 层级
- [ ] **P1-6** 删除 gateway-rs 旧精简侧栏（`app-sidebar` 号池只读页）或改为重定向 `/accounts`
- [ ] **P1-7** 删除「无限画布」三方配置 UI（`third-party-apps-card` 中 infinite_canvas 段）或改为「TNexus 工作台」说明
- [ ] **P1-8** 鉴权：全站走 TNexus JWT；管理 API 请求带同一 Bearer（gateway 校验 tnexus 签发或共享 secret）
- [ ] **P1-9** `web` 本地 dev：proxy `/v1` → gateway、`/api/jobs` → tnexus-api、`/api/accounts` → gateway

### P2 — 号池 API（Rust gateway，第一阶段必达）

对照 `gptimage/api/accounts.py`（约 **26** 个端点，[gap §1.4](docs/24-gap-inventory.md)）。按号池页调用顺序实现：

#### P2-A 读列表与统计

- [ ] **P2-A1** `GET /api/accounts` — 分页列表 + `stats`（总数/正常/受限/禁用/报错/今日额度）
- [ ] **P2-A2** `GET /api/accounts/activity/daily?days=N` — 账号流水（注册/入库/读取/删除 + 生图/对话）
- [ ] **P2-A3** `GET /api/accounts/schedulable-breakdown` — 不可调度归因
- [ ] **P2-A4** `GET /api/accounts/usage/recent` / `usage/binding-slots` — 热力图数据

#### P2-B 调度与状态

- [ ] **P2-B1** `POST /api/accounts/scheduling` — 单账号进/出调度
- [ ] **P2-B2** `POST /api/accounts/scheduling/bulk` — 全选/全停调度
- [ ] **P2-B3** `POST /api/accounts/soft-band` — 软封/解封
- [ ] **P2-B4** `POST /api/accounts/update` — 编辑代理/备注等

#### P2-C 导入导出与刷新

- [ ] **P2-C1** `POST /api/accounts` — 单条写入
- [ ] **P2-C2** `POST /api/accounts/import-batch` — 批量导入
- [ ] **P2-C3** `GET` 导出（对齐前端导出已载入 Token 流程）
- [ ] **P2-C4** `POST /api/accounts/refresh` + `GET .../progress/{id}` — 刷新额度
- [ ] **P2-C5** `POST /api/accounts/refresh-all/*` — 全量刷新启停
- [ ] **P2-C6** `POST /api/accounts/re-login` + progress — OAuth 重登（可 P2 末或 P3 初）

#### P2-D 持久化与号池数据面

- [ ] **P2-D1** 账号存储：SQLite/Postgres 方案选型（现 gateway `HashMap` 重启丢失 — 必须改）
- [ ] **P2-D2** 与 `pin_account.json` + 共享 `accounts.db` 加载兼容
- [ ] **P2-D3** 代理绑定、fp、token 字段与 gptimage `Account` 模型对齐
- [ ] **P2-D4** 调度门 / slot 记账：评估复用 `gptimage/crates/image_schedule_core`（path 依赖）vs 自研子集

#### P2-E 明确不做（号池页可隐藏或灰显）

- [ ] **P2-E1** Panda sync UI / `POST /api/accounts/sync/panda`
- [ ] **P2-E2** Outlook 自动恢复 / maintenance-loop 全套
- [ ] **P2-E3** CPA 池 / sub2api 连接

### P3 — 号池 UI 迁完（第一阶段验收核心）

- [ ] **P3-1** 迁入 `gptimage/web/src/app/accounts/**` + `components/accounts/**`
- [ ] **P3-2** 统计卡 8 项与流水 **双折线图** 接 P2 API
- [ ] **P3-3** 账户表格：搜索/筛选/按 IP 分组/调度开关/额度列/操作列
- [ ] **P3-4** 批量操作：全选调度、全部出调度、模糊额度、导入、导出
- [ ] **P3-5** 账号导入对话框 `account-import-dialog.tsx`
- [ ] **P3-6** 绑定热力图 / CF 状态灯 /  egress 漂移（依赖 API 可分期 mock→实）
- [ ] **P3-7** 号池页 E2E 冒烟脚本 `scripts/accounts_smoke_test.py`
- [ ] **P3-8** 与 Python 生产页 **对照截图** 验收（关键交互 20 条用例，见 [docs/18-test-matrix.md](docs/18-test-matrix.md) 待增 §TNexus-admin）

### P4 — 其余管理页（第二阶段）

- [ ] **P4-1** `/image` — 生图工作台（gptimage `image-workbench`）
- [ ] **P4-2** `/image-manager` — 图片管理
- [ ] **P4-3** `/logs` — 日志管理
- [ ] **P4-4** `/ops` — 运维仪表盘（替换 gateway-rs JSON 占位）
- [ ] **P4-5** `/chat` — 对话
- [ ] **P4-6** `/settings` — 系统设置（剔除注册机/无限画布相关卡片）
- [ ] **P4-7** `/debug` — 可选，默认管理员可见

### P5 — 部署与生产验收

- [ ] **P5-1** 合并 `deploy/nginx/tnexus.relai.asia.conf`（`/v1/`、`/api/backend/` → gateway）
- [ ] **P5-2** `scripts/panda_setup_tnexus_env.py` 迁入并更新（`UPSTREAM_API_KEY`、Postgres 密码保留逻辑）
- [ ] **P5-3** Panda compose：gateway 容器 + tnexus api/worker（或单镜像多 command）
- [ ] **P5-4** `prod_url_chain_test.py` — URL 生图全链路 OK
- [ ] **P5-5** 号池页生产冒烟（只读列表 + 一次调度切换 + 流水图有数据）
- [ ] **P5-6** `gptimage.relai.asia` → `tnexus.relai.asia` 301（管理台路径迁移完成后）
- [ ] **P5-7** 更新 [docs/33-panda-deploy-20260728.md](docs/33-panda-deploy-20260728.md) 合并版

---

## 5. 风险与依赖

| 风险 | 影响 | 缓解 |
|------|------|------|
| 号池 API 26 端点一次补齐周期长 | P3 延期 | P2 按页面调用链排序；热力图可后补 |
| 账号持久化未做 | 重启丢池 | P2-D 为 P3 硬前置 |
| 双 `auth` crate | 编译/登录混乱 | P0-3 重命名 + 统一 JWT issuer |
| 两套 Next 合并 `_next` 冲突 | 已单 app，无此问题 | — |
| `helper_ok: false` 不影响生图 | upstream 模式可忽略 | 文档注明 `DATA_PLANE=upstream` |
| 生产 `:8012` 与合并后行为不一致 | 运维困惑 | HANDOFF 标明「对照源只读」 |

---

## 6. 验收命令（合并后）

```bash
# 本地
cargo build --workspace
cargo test --workspace
cd web && npm run build

# Panda（只 pull，不 build）
ssh panda "python3 /root/prod_url_chain_test.py"
ssh panda "python3 /root/scripts/accounts_smoke_test.py"   # P3 后新增
```

---

## 7. 文档索引

| 文档 | 说明 |
|------|------|
| [HANDOFF.md](HANDOFF.md) | 当前状态、部署、已知问题 |
| [docs/SOURCE.md](docs/SOURCE.md) | UI/API 对照源（gptimage） |
| [docs/README.md](docs/README.md) | docs 目录索引 |
| [docs/24-gap-inventory.md](docs/24-gap-inventory.md) | 生产 vs Rust 能力 gap（自 gateway-rs 同步） |
| [TNexus.md](TNexus.md) | 产品愿景（保留） |

---

## 8. 进度跟踪

| 阶段 | 状态 | 完成日 |
|------|------|--------|
| P0 仓库合并 | ☐ 未开始 | — |
| P1 Web 壳 | ☐ 未开始 | — |
| P2 号池 API | ☐ 未开始 | — |
| P3 号池 UI | ☐ 未开始 | — |
| P4 其余管理页 | ☐ 未开始 | — |
| P5 部署验收 | ☐ 部分（仅 URL 生图链路过） | 2026-07-30 |
