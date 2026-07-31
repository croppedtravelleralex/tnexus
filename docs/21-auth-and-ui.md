# 21 — 鉴权与 Web UI

> **⚠️ 定位：超越 Panda 的增强功能，不是对齐 Panda 的前置条件。**
>
> 鉴权验收标准见 [docs/28-decisions-20260727.md](28-decisions-20260727.md) §1.4：
> - **对齐阶段**：`:8013` = `AUTH_DISABLE=1`（与 Panda 现网一致）；R2 替换 `:8012` = API key
> - **本文所述**（JWT / Web UI / 角色门禁）= **后续增强**，Panda 对齐完成后再立项启用
>
> **⚠️ 本文描述的能力尚未部署到 Panda。**
> panda `:8013` 实测 `/api/auth/*` 等 **全部 404**（二进制仍为 2026-07-21 旧版）。
> 代码已进 git（`80fc447`+），待走 git 链路发布。

最后更新：2026-07-27

## 分支状态

| 分支 | 实际状态 |
|------|----------|
| `main` | 可部署快照 |
| `develop` | 与 `main` **逐字节相同**；`git rev-list --count main..develop` = 0 |

仓库中**不存在任何 feature 分支**，git log 里也没有任何 auth / UI / 契约相关提交。
本文涉及的 `crates/auth/`、`auth_routes.rs`、`backend_routes.rs`、`state.rs`、
`error_class.rs`、`image_contract.rs`、`ticket_pool/`、`control_client/`、
`fixtures/protocol/*.json`、`docs/21`–`26`、以及整个 `web/` 均处于 **untracked** 状态，
尚未进入任何分支。

## 鉴权

- 存储：`data/auth.db`（SQLite，不进 git）
- 密码：argon2id
- 会话：JWT in `gws_session` httpOnly cookie（名称可由 `AUTH_COOKIE_NAME` 改）；可选 `Authorization: Bearer`
- 角色：`admin` | `member`
- Bootstrap：首次空库时读 `AUTH_BOOTSTRAP_ADMIN_USER` / `AUTH_BOOTSTRAP_ADMIN_PASSWORD`

> **CRITICAL —— Bootstrap 静默回退硬编码凭据。**
> `crates/auth/src/lib.rs:166-173`：两个 bootstrap 变量**任一缺失或为空**时，直接回退到
> `("admin", "admin-change-me")` 并建库，**无告警、无日志、无强制改密**。
> 也就是说：部署时忘记设这两个变量，系统会静默创建一个公开已知口令的 admin 账号。
> 详见 `22-audit-2026-07-26.md`。

### API

| Method | Path | 角色 |
|--------|------|------|
| POST | `/api/auth/login` | 公开 |
| POST | `/api/auth/register` | admin（或 `AUTH_ALLOW_PUBLIC_REGISTER=1`） |
| POST | `/api/auth/logout` | 已登录 |
| GET | `/api/auth/me` | 已登录 |
| GET | `/api/admin/users` | admin |
| POST | `/api/admin/users/{id}/disabled` | admin |
| GET | `/api/backend/capabilities` | 公开 |

### 路由门禁

- **成员 + 管理员**（`main.rs:108-114` 的 `member_api`，统一挂 `require_member` + `require_auth`）：
  `/v1/chat/completions`、`/v1/models`、`/v1/images/generations`、`/v1/images/edits`
  —— 生图**同样需要登录**，不是公开端点
- **仅管理员**：`/v1/accounts/candidates`、`/v1/quota*`、`/api/admin/*`
- 开发：`AUTH_DISABLE=1` 跳过 JWT（默认关）

### 生图两条路由行为不同

| 路由 | 受 `IMAGE_ENABLED` 控制 | 返回 | code |
|------|------------------------|------|------|
| `/v1/images/generations` | 是（`main.rs:400-407`） | 501 | `image_deferred` |
| `/v1/images/edits` | **否** | 501 | `image_edits_deferred` |

`image_edits`（`main.rs:532-542`）把 State 解构为 `_st`，**完全不读 `image_enabled`**，
无条件返回 501；即使 `IMAGE_ENABLED=1` 也一样。
另外，当前 panda 生产二进制里**这条路由根本不存在，实测 404**（不是 501）。

## Web UI

- 路径：`web/`（Next 16 + shadcn 风格 dashboard）
- **`git ls-files web` = 0 —— 整个目录未提交。**
  `.gitignore:13-15` 另外忽略 `web/node_modules/`、`web/.next/`、`web/out/`，
  所以按「commit → push → Actions → GHCR → panda pull」的 git 部署链路，
  静态产物**当前永远送不到 panda**。
- 成员：`/chat`（可用）；`/image`（**占位**）；`/register`（`web/src/app/register/page.tsx`，
  调 `/api/auth/register`）
- 管理员：`/dashboard`、`/accounts`、`/quota`、`/settings`；`/ops`、`/logs`（**占位**，
  `web/src/app/{ops,logs}/page.tsx` 为纯静态卡片，零 API 调用）
- 生产：gateway `GATEWAY_STATIC_DIR=web/out` 同域 cookie
- 开发：`NEXT_PUBLIC_API_BASE=http://127.0.0.1:8013 npm run dev`

> **⚠️ 上面这条 dev 流程当前跑不通 —— 先修 CORS。**
> 跨域 + 带 cookie 的前提是服务端 CORS 合法，而 `main.rs:129-135` 的 `CorsLayer` 用了
> `allow_origin(Any)` + `allow_credentials(true)`。tower-http 在此组合下**启动即 panic**
> （规范禁止 `Access-Control-Allow-Origin: *` 与凭据并存）。
> 必须先改成显式 origin 白名单，dev 模式才能起来。

> **`GATEWAY_STATIC_DIR` 静默降级：** `main.rs:75-78` 带 `.filter(|p| p.is_dir())`，
> 目录不存在时**既不报错也不告警**，直接当作未配置继续启动，UI 静默消失。
> 唯一可观测点是 `/health` 的 `static_ui` 字段（`main.rs:178`）。

### 页面与后端对应

| 页面 | 调用 |
|------|------|
| `/login` | `POST /api/auth/login` |
| `/register` | `POST /api/auth/register` |
| `/chat` | `POST /v1/chat/completions` |
| `/image` | 读 `GET /api/backend/capabilities`（不调用生图） |
| `/dashboard` | `/health` + `/api/backend/capabilities` |
| `/accounts` | `GET /v1/accounts/candidates` |
| `/quota` | `GET /v1/quota` |
| `/settings` | `GET /api/admin/users` |
| `/ops`、`/logs` | 无（静态占位） |

## 环境变量

### 鉴权

| 变量 | 默认 | 说明 |
|------|------|------|
| `AUTH_DB_PATH` | `data/auth.db` | SQLite 路径 |
| `AUTH_JWT_SECRET` | `dev-only-change-me-in-production-32b` | **CRITICAL**：有硬编码 fallback（`lib.rs:84-88`），长度恰好通过 `len() >= 32` 校验，**不设也能正常启动**，并非必填 |
| `AUTH_JWT_TTL_SECS` | `86400` | JWT 过期 |
| `AUTH_COOKIE_NAME` | `gws_session` | 会话 cookie 名（`lib.rs:96`） |
| `AUTH_COOKIE_SECURE` | `0` | 生产 HTTPS 设 `1` |
| `AUTH_ALLOW_PUBLIC_REGISTER` | `0` | `1` 允许自助注册 member |
| `AUTH_DISABLE` | `0` | `1` 跳过鉴权。**panda bringup（`panda_bringup_rust_face.sh:72`）强制 `${AUTH_DISABLE:-1}`，生产实际是关闭鉴权运行** |
| `AUTH_BOOTSTRAP_ADMIN_USER` | 回退 `admin` | 缺失/为空即静默回退（`lib.rs:106`） |
| `AUTH_BOOTSTRAP_ADMIN_PASSWORD` | 回退 `admin-change-me` | 同上（`lib.rs:107`） |

### 网关

| 变量 | 默认 | 说明 |
|------|------|------|
| `PIN_ACCOUNT_FILE` / `PIN_ACCOUNT_JSON` | — | **全仓唯一真正必填项**：二者全缺则 `bail!` 启动失败（`config.rs:54-61`） |
| `GATEWAY_LISTEN` | `0.0.0.0:8013` | 监听地址（`config.rs:37`） |
| `HELPER_URL` | `http://127.0.0.1:19001` | Helper 地址（`config.rs:38`） |
| `ACCOUNTS_FILE` | — | 追加账号池，与 pin 账号合并（`config.rs:65`） |
| `IMAGE_ENABLED` | **`0`** | `1` 开启 `/v1/images/generations`；对 `/v1/images/edits` 无效 |
| `IMAGE_GLOBAL_CONCURRENCY` | `3` | 全局生图并发（`config.rs:44`） |
| `MVP_MIN_IMAGE_QUOTA` | `1` | 最低生图配额（`config.rs:39`） |
| `GATEWAY_STATIC_DIR` | — | 托管 `web/out`；目录不存在时静默降级 |

### 前端

| 变量 | 默认 | 说明 |
|------|------|------|
| `NEXT_PUBLIC_API_BASE` | `""`（同域） | 构建期注入（`web/src/lib/api.ts:2`）；跨域取值需先修 CORS panic |

## 验收清单（Phase A+）

- [ ] admin 登录 → dashboard/accounts/settings —— **代码就绪，未验收**（未部署）
- [ ] member 登录 → `/chat`；直访 admin 路由 → 403 —— **代码就绪，未验收**。
      且 panda 现状 `AUTH_DISABLE=1` 下 `require_auth`（`auth_routes.rs:164-174`）直接注入
      `Role::Admin`，**403 分支永不触发**
- [ ] JWT cookie 登出清除 —— **代码就绪，未验收**（未部署）
- [ ] `cargo test` + `web` build 绿 —— **失败**：`crates/ticket_pool` 4 处 `E0277` 导致
      `cargo build --workspace` FAIL，19 个测试**一个都跑不了**
- [ ] 生图冒烟（`IMAGE_ENABLED=1` 后，CF 窗允许 upstream 失败）
