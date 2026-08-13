# TNexus 架构文档

> 用途：读代码前的导航图。引用均指向当前 `main` 实际行号（2026-08-05 核实）。
> 配套：[plan.md](../plan.md) 施工总控 · [HANDOFF.md](../HANDOFF.md) 部署现状 · [docs/SOURCE.md](SOURCE.md) UI 对照源

---

## 1. 一句话定位

**TNexus** = 导演工作台（studio）+ chatgpt2api 号池/生图管理台 + Rust OpenAI 兼容网关，单仓 monorepo，唯一公网入口 `https://tnexus.relai.asia`。控制面（tnexus-api）与数据面（gateway）分离，同一进程树但各自独立容器。

---

## 2. 部署拓扑图

> ⚠️ **「gateway」在本项目里指三个不同的东西**，这是排查时最常见的误解来源。
> 说到 gateway 必须带端口号。

| 端口 | 进程 | 管什么 | 谁调它 | 公网可达 |
|------|------|--------|--------|----------|
| `:8014` | `tnexus-gateway`（`crates/gateway`） | **GPT** 生图 / 对话 | NewAPI ch114/115、nginx `/v1/` | 是 |
| `:8000` | `grok2api-rs`（`crates/grok-gateway`） | **Grok** 对话 / OCR / 生图 | tnexus-api 代理、NewAPI ch117 | 否 |
| `:8091` | grok-admin（与上同进程） | Grok 号池管理 `/admin/*` | tnexus-api 代理 | 否 |
| `:9000` | `tnexus-api` | 控制面 + 静态 UI + 上面两者的代理 | nginx `/*` | 是 |
| `:9011` | `account-ops` | GPT 养号/OAuth/refresh | tnexus-api 委托 | 否 |

```text
tnexus.relai.asia (nginx, host)   ← 唯一公网入口
  ├─ /v1/*            → 127.0.0.1:8014   tnexus-gateway（GPT 生图 OpenAI 兼容）
  ├─ /api/backend/*   → 127.0.0.1:8014   gateway 号池管理 API
  └─ /*               → 127.0.0.1:9000   tnexus-api（控制面 + 静态 UI）
                            ├─ /api/grok/v1/*  → 127.0.0.1:8000  grok2api-rs
                            └─ /api/grok/*     → 127.0.0.1:8091  grok-admin

服务（均为 host 网络）
  api        :9000   tnexus-api      控制面 + 静态渲染 + Grok 双向代理
  worker     —       tnexus-worker   异步任务消费者
  gateway    :8014   gateway         GPT 数据面（DATA_PLANE=upstream）
  grok2api-rs :8000/:8091            Grok 数据面 + 管理面
  account-ops :9011                  GPT 养号/OAuth/refresh/outlook 执行面

外部依赖（回环端口，仅本机可达）
  postgres  127.0.0.1:5433  (容器内 5432)
  redis     127.0.0.1:6380  (容器内 6379)
  Cloudflare R2       媒体面（原图/预览/缩略图三级 + 签名 URL）
```

**浏览器永远到不了 `:8000`/`:8091`**，前端一切 Grok 请求都经 tnexus-api 用 cookie 鉴权后转发。
任何写着「直连 grok 网关（:8000）」的说明都是错的。

### 2.1 同机容器回连宿主端口需要 UFW 放行

NewAPI 等容器经 `host.docker.internal`（→ docker0 `172.17.0.1`）回连宿主端口，
而 UFW 默认策略是 DROP。症状是**容器侧 `connect: connection timed out`，宿主本地
`curl` 却完全正常**，极易误判成服务故障（`:8000` 上线时实测卡 133 秒）。

放行按来源网段，不要图省事对全网开放（`:8014` 早期的 `ALLOW Anywhere` 是反面教材）：

```bash
ufw allow from 172.19.0.0/16 to any port 8000 proto tcp comment 'newapi to grok2api-rs 8000'
ufw allow from 172.17.0.0/16 to any port 8000 proto tcp comment 'docker0 to grok2api-rs 8000'
```

容器可能同时挂多个网络，需逐个放行；`deploy/panda/newapi_tnexus_ocr.sh` 的 `apply`
已内置预检，失败时直接打印所需命令。

- nginx 分流：`deploy/nginx/tnexus.relai.asia.conf` — `/v1/`(:30)、`/api/backend/`(:42) → 8014；其余(:59) → 9000；`proxy_buffering off` + 900s 超时支持长 SSE。
- 三容器（api/worker/account-ops）`network_mode: host`（`deploy/panda/docker-compose.yml`）；gateway 由独立镜像 `panda-gateway-1` 运行。
- 号池真源 `/root/gptimage/data/accounts.db` 由 8012(Python) 与 TNexus api/gateway 共享读写（WAL + 事务）；调度状态 `/opt/tnexus/data/pool/`。

---

## 3. 模块职责表

Workspace members 见 `Cargo.toml:3-27`。

### 控制面（TNexus 核心）

| crate | 职责 | 要点 |
|-------|------|------|
| `tnexus-api` | axum HTTP 控制面 | `/api/auth|chat|conversations|jobs|accounts|logs|images|ops`，托管静态 UI 回退 `ServeDir`（main.rs:78-97） |
| `tnexus-worker` | 异步任务消费者 | 生图/竞演任务后台执行 |
| `tnexus-auth` | 用户 JWT + argon2 | Postgres 持久，`AuthService::new(pool, jwt_secret, ttl)`（state.rs:33-37） |
| `tnexus-domain` | 导演/因子/任务模型 | `append_image_generation_hints` 拼生图 hint |
| `tnexus-storage` | R2 + 图片变体 | `ImageStore::from_env`，三级存储 + 签名 URL |
| `tnexus-accounts-db` | 号池 sqlite 访问 | 单行 upsert/delete 事务写，跨进程共享 WAL |
| `tnexus-account-ops` | 养号执行面 | 委托本地 Python 库，`GPTIMAGE_ROOT` |

### 数据面（gateway 侧）

| crate | 职责 | 要点 |
|-------|------|------|
| `gateway` | `/v1/` + `/api/backend/` | 生图/对话/编辑 + 号池管理 API；`upstream_face`(:14) 桥接 upstream |
| `gateway-auth` | gateway JWT + SQLite | `AUTH_JWT_SECRET` + username/jti |
| `protocol` | OpenAI 契约 | 请求/响应模型 |
| `helper_client` | Python helper HTTP 客户端 | 边际化（DATA_PLANE=helper 时用） |
| `upstream` | Rust 直连数据面 | TLS/PoW/Turnstile/SSE/生图全 Rust |
| `upstream-probe` | upstream 探测 | 能力探测 |
| `ticket_pool`+`control_client` | **已冻结** | 不再演进 |

### 其他

| 目录 | 职责 |
|------|------|
| `web/` | 单一 Next.js（studio + 管理台），静态导出 |
| `helper/` | Python protocol_bridge，`DATA_PLANE=upstream` 时可不跑 |
| `crates/grok-*` | Grok 平行主线（chat/生图/pool/audit/egress），见 [39a](39a-grok-roadmap.md) |

---

## 4. 两条数据面链路

gateway 通过 `DATA_PLANE` 选数据面，`DataPlane::from_env`（config.rs:19-30）——**默认 Upstream**，未设/非法值一律走 Upstream（config.rs:25-29）。

| 维度 | Upstream（默认） | Helper（边际化） |
|------|------------------|------------------|
| 实现 | Rust `upstream` crate 直连 ChatGPT | HTTP 调 `:19001` Python |
| TLS 指纹 | wreq impersonate | Python 侧 |
| PoW/Turnstile/SSE/生图 | 全 Rust | Python |
| 触发 | `helper_url` 不可用自动回退含义 | 仅当显式 `DATA_PLANE=helper` |

接线已核实（`crates/gateway/src/upstream_face.rs`）：
- `run_text` (:35) / `run_text_stream` (:40) / `run_image` (:50) / `run_image_edit` (:61) 四入口全走 Rust runtime。

---

## 5. 导演任务生命周期

状态机（阶段 → 百分位，`progress_for_status`，routes/jobs.rs:278-288）：

```text
queued(5) → directing(25) → generating(55) → uploading(85) → done(100) / failed(0)
```

**SSE 是"DB 轮询伪推送"**（routes/jobs.rs:211-276，`job_events_handler`）：
- 实际事件来自订阅 Redis `channel`（:223-239），但 Postgres 才是唯一事实源。
- 兜底 `900s` 强超时（:268），`KeepAlive` 保活。
- 前端另有轻量轮询 `/api/jobs/{id}/status`（:192-209）返回 progress 百分位。
- **Redis 事件通道是死代码**——没有任何进程向该 channel publish；SSE 头部事件即最终态（initial(:248) 若 done/failed 直接 return(:249)）。

---

## 6. upstream 数据面关键技术

模块 `crates/upstream/src/lib.rs`：

| 机制 | 实现要点 |
|------|----------|
| TLS 指纹 | wreq impersonate（**非自握手**）；Chrome 120/124/131 三档，**默认 Chrome124 + Windows** |
| PoW | 24 元素 config 数组 + SHA3-512 前缀比较，`gAAAAAB` 前缀 |
| Turnstile VM | 34 opcode 解释器 |
| chat-requirements 三票据链 | bootstrap→prepare→finalize，**arkose 未实现** |
| SSE 收敛 | 4 种 `SseEvent` + `ConversationState`（sse.rs `re-export` lib.rs:30-33） |
| poll/estuary | 固定退避（lib.rs:25-28 `ImagePollConfig/ImagePollOutcome`） |

**已知差距（相对 Python）**：poll/estuary 固定退避、**无熔断**——标记待补。
gateway 侧重试：`image_max_attempts`（main.rs:524-534）admin 上限 `IMAGE_ADMIN_RETRY_MAX`(默认3)；`classify_fault`(main.rs:44，:975/:1196 使用)。

---

## 7. 双 JWT 体系（两套，不互通）

| 体系 | 载体 | claims | 校验 |
|------|------|--------|------|
| gateway-auth | SQLite + `AUTH_JWT_SECRET` | `username` + `jti` | 每次查库 |
| tnexus-auth | Postgres + `JWT_SECRET` | `email` + `sub` | **不重读 DB**（签时查，验时只对 secret） |

两者 **claims 结构不同、secret 不同、无法互验**。worker→gateway 走 `UPSTREAM_API_KEY`（gateway JWT），过期报 `invalid session`。

---

## 8. 号池存储（共享 DB + scheduling 文件）

- **账号池真源在共享 DB**，不再读写 `accounts_pool.json`（`lib.rs:10-11` 注释 "JSON snapshot removed"）。后端由 `AccountsBackend` 统一，= `ACCOUNTS_BACKEND` env：`postgres` → PG `tnexus_accounts`，默认 → sqlite gptimage `accounts.db`（`accounts_db_path()` 要求 `ACCOUNTS_DB` env）。实现在 `crates/tnexus-accounts-db/`（backend.rs 89 行 + lib.rs 490 行 + pg.rs 204 行）。
- **原子单行写**：`touch_inflight` 已是 DB 层原子 upsert（backend.rs `touch_inflight(&self, email, delta)`）；`persist_account_row`（`crates/tnexus-api/src/accounts_store.rs:788`）走 `db.upsert_account_value`，**无整文件重写**，规避双进程整表互相覆盖。
- **剩余文件态**：`scheduling_state.json` 仍是文件（api `accounts_store.rs:798-820` 的 `load/save_scheduling_state`，`fs::write` :818；gateway `scheduling_gate.rs:70-92` 的 `load/save_scheduling`）——双进程各自整文件写，无跨进程锁，见第 11 节 P1。

---

## 9. web 前端

- **Next 16 静态导出**：`next.config.ts` `output: "export"` + `trailingSlash`（image 关闭优化）。导航走原生 `<a>`（扩展名补后缀，nginx:54-56 302 补尾斜杠）。
- **全 client 组件**：`(console)/layout.tsx` 包 `ConsoleLayout`。
- **ConsolePageCache 6 页保活**（console-page-cache.tsx:6）：`/studio` pinned，`/accounts`、`/image-manager` 为重路由不缓存(:10-13)。
- **api-cache 内存 TTL**：前端响应缓存。
- **SSE + 轮询双通道**：进度/事件走 SSE（api 伪推送 + 前端轮询兜底）。

---

## 10. 部署铁律

1. **禁止在 Panda 上编译**：`cargo`/`docker build`/`npm build`/`buildx` 一律禁止（曾爆 CPU/内存）。
2. **只走 Git→GHCR→pull+up**：本地或 CI 构建 → commit → push → GitHub Actions → GHCR → Panda `deploy.sh`（仅 `docker compose pull && up`）。
3. **禁 scp/docker cp**：不传二进制/不替换容器内文件/不在 Panda 改生产代码。
4. 生产 `:8012` 只读诊断不受限，禁止替换或重启。
5. Cursor 强制：`.cursor/rules/panda-no-remote-build.mdc`。

---

## 11. 已知风险表

| 风险 | 位置/事实 | 影响 |
|------|-----------|------|
| scheduling_state.json 无锁整文件重写 | 仅此一文件仍双进程各自 `fs::write`（api vs gateway），无跨进程锁 | 并发写互相覆盖丢数据（已列 P1：fs2 锁 + 原子写） |
| verify_token 不重读 DB | tnexus-auth | 禁用用户仍可用旧 token 到过期 |
| upstream 缺重试策略 | 无熔断、固定退避 | 高失败时尾延迟不可控 |
| Disabled 模式人人 admin | Disabled 分支 | 权限退化为 admin |
| 资产签名密钥混用 JWT 密钥 | 同 secret | 密码学边界模糊 |
| migration 003/005 命名错位 | migrations/ | 回滚/历史不直观 |
| to_upstream 兜底 Chrome120 | TLS 指纹缺失时 | 兼容性兜底，无明确锁定 |

---

## 12. 接线现状（2026-08-05 核实）

gateway 数据面接线（`crates/gateway/src/main.rs`）：
- `classify_fault` — import :44，调用 :975/:1196
- `run_image`（upstream_face）— :551
- `run_text_stream`（upstream_face）— :867
- `run_text`（upstream_face）— :938
- `run_image_edit`（upstream_face）— :1478

upstream 全部模块已接线（lib.rs:6-19 共 14 模块：tls/pow/turnstile/sse/requirements/estuary/poll/upload/conversation/sentinel/account/runtime/openai_stream/image_metrics）。

> ⚠️ `docs/24-gap-inventory.md`（2026-07-26）已过期——`run_image_edit` 已实现，编辑生图已支持。

---

## 13. Grok 子系统现状（2026-08-13 核实，已在生产）

Grok 不再是「开发中的平行主线」，它在生产承载对话与 OCR。

| 项 | 现状 |
|----|------|
| 号池 | PG `grok_accounts` 707 行 / 546 启用 grok_web；额度在 `grok_quota_windows` |
| 额度刷新 | 后台任务 `web_quota_refresh`，60s 一批 32 个，按 fast 窗口 `synced_at` 升序轮换，全池约 17 分钟一轮 |
| 选号 | `SimplifiedPool` **LRU**（`last_selected_seq`），非随机；`Selector` 全量能力仍未接生产 |
| 失败处理 | 429/403 跨账号重试（`GROK_CHAT_RETRY_MAX`，默认 4）；限速退避 60s→300s；健康状态回写 PG |
| OCR | `:8000` `/v1/chat/completions`，model `grok-vision-ocr` 或 messages 含 `image_url` |
| 计费 | NewAPI ch117 分组 `tnexus-ocr` 倍率 1.0，`ModelPrice` 0.01/次（= 5000 配额点） |

### 13.1 额度语义（读 `grok_quota_windows` 前必看）

| mode | 含义 | 陷阱 |
|------|------|------|
| `fast` | 24h 对话额度，来自 `POST /rest/rate-limits` | 唯一持续刷新的窗口，判断账号可用性看它 |
| `auto` | 历史遗留 | 无任务刷新它，时间戳会一直陈旧 |
| `imagine` | 生图额度 | `total ≈ 1.155e10` 是**「不限」哨兵值**，不是真实张数 |
| `console` | Console 账号 | `source='default'` = 从未探测过的占位值，不可当真 |

`source='upstream'` 才是上游真实回包；判断新鲜度看 `synced_at` 而非 `updated_at`。

### 13.2 长期约束：单一代理出口

`GROK2API_PROXY_LIST` 目前只有 1 个可用出口，698 个账号共用。宿主自身 IP 已被
grok.com 硬封（403）。廉价端点（`/rest/rate-limits`）能过，昂贵端点（chat/imagine）
按 IP 限流 → 这是 429 的**根因，代码层无法根治**，跨账号重试只能缓解。

采购新代理前用 `scripts/grok_proxy_probe.sh` 筛选：它会拿当前生产代理做对照组，
只有 `grok=200` 的才可用。2026-08-13 实测 20 个 webshare 静态住宅代理（`82.25/26/27/29.x`）
**全部被边缘 403**（45ms 级 IP 封禁），整段被封，不可用。