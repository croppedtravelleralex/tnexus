# 25 — Panda 生产（Python）× 本仓（Rust）实测对照

采集日期：2026-07-26
采集方式：`ssh panda` 只读诊断（`/proc` 读取、`docker inspect|stats|logs`、`git log|status|show`、`strings`、`sha256sum`、本机 `curl GET`）
**未在 panda 上编译、写文件、改配置、重启服务或发送压测/生图流量。**

本文与 [23-rewrite-progress.md](23-rewrite-progress.md) 的分工：23 算**进度百分比**，本文记**两侧实测差异**。
两文冲突时以本文为准 —— 23 的部分数字来自本地快照，本文来自生产机现采。

---

## 0. 三条最要紧的结论

| # | 结论 | 依据 |
|---|------|------|
| 1 | **本地 2,707 行 Rust 中只有 943 行进了 git**，其余 1,764 行（含全部 `auth`/`ticket_pool`/`control_client`）是未跟踪工作树；**整个 `web/` 也未跟踪** | `git ls-files crates/` = 943；`git ls-files web` = **0**；panda HEAD `6509fba` 与本地 HEAD 源码逐字节相同；`git rev-list --count main..develop` = **0** |
| 2 | **文档描述的系统 ≠ 运行的系统** —— `21-auth-and-ui.md`(80 行) 与 `17` 的鉴权/UI/capabilities 段落，描述的功能在生产二进制里根本不存在 | `/api/auth/me`、`/api/auth/login`、`/api/admin/users`、`/api/backend/capabilities`、`/v1/images/edits` 实测**全部 404**；`strings bin/` 无 auth 符号 |
| 3 | **Rust 进程 3 天累计只用了 0.33 秒 CPU** —— 它的「低资源」不是重写效率的证明，是空闲进程的证明 | `/proc/2769070/stat` 字段 14/15 |

按部署铁律的 git 链路，结论 1 意味着这 1,764 行**当前不可能到达 panda**；而一旦提交，`cargo build --workspace` 又过不了（4×E0277）。

---

## 1. 代码维度

### 1.1 规模

| 侧 | 范围 | 数值 |
|----|------|------|
| Python 生产 | 去噪后核心 `.py`（排除 backups/web_dist/.venv/reports/pycache） | **120 文件 / 49,555 行** |
| Python 生产 | `services/` | 96 文件 / 43,450 行 |
| Python 生产 | `api/` | 12 文件 / 3,647 行 |
| Python 生产 | `utils/` | 10 文件 / 1,731 行 |
| Rust 本仓 | `crates/**/*.rs` 工作树 | 13 文件 / **2,707 行** |
| Rust 本仓 | **已进 git** | 4 文件 / **943 行** |
| Rust 已部署 | panda 运行中的 ELF | 对应 commit `7c34159`，编于 **2026-07-21 10:31** |

> ⚠️ panda `/root/gptimage` 全树 `.py` 是 4,066 文件 / 255,670 行，但其中含 39 个 `web_dist.bak.*`、119 个 `backups/`、一个 45MB 的 `_tmp_image_schedule_trace.tgz`，以及名为 `;` `\` `chmod` `cp` `700` 的误建目录。**未去噪的数字没有意义。**

### 1.2 已提交 vs 未提交（本仓）

| 文件 | 行数 | 进 git |
|------|------|--------|
| `crates/gateway/src/main.rs` | 677 | 部分（HEAD 456） |
| `crates/helper_client/src/lib.rs` | 250 | 是（HEAD 237） |
| `crates/protocol/src/lib.rs` | 158 | 是（HEAD 148） |
| `crates/gateway/src/config.rs` | 108 | 是（HEAD 102） |
| `crates/auth/src/lib.rs` | 387 | **否** |
| `crates/ticket_pool/src/lib.rs` | 285 | **否** |
| `crates/gateway/src/auth_routes.rs` | 264 | **否** |
| `crates/protocol/src/image_contract.rs` | 202 | **否** |
| `crates/protocol/tests/fixtures.rs` | 138 | **否** |
| `crates/control_client/src/lib.rs` | 107 | **否** |
| `crates/protocol/src/error_class.rs` | 74 | **否** |
| `crates/gateway/src/backend_routes.rs` | 36 | **否** |
| `crates/gateway/src/state.rs` | 21 | **否** |

本地相对 panda 只领先两个**纯文档** commit（`6508873`、`02ce63c`），
且 `git rev-list --count main..develop` = **0** —— `develop` 与 `main` 逐字节相同，不存在任何 feature 分支。

**除 `crates/` 外，以下也全部未跟踪**：

| 路径 | `git ls-files` |
|------|---------------|
| `web/`（整个 Next 16 dashboard） | **0 条** |
| `fixtures/protocol/*.json` | 8 份未跟踪 |
| `docs/21` ~ `docs/26` + `docs/README.md` | 7 份未跟踪 |
| `scripts/_tmp_{deploy_rust_ticket_round,rust_ticket_image_round}.py` | 2 份未跟踪（含真实邮箱） |

`web/` 尤其要注意：`.gitignore` 另忽略 `node_modules/`、`.next/`、`out/`，
所以**按 git 部署链路，静态 UI 产物当前永远送不到 panda**，`GATEWAY_STATIC_DIR` 无从指向。

### 1.3 运行中二进制的溯源

| 项 | 值 |
|----|-----|
| 路径 | `/root/gptimage-gateway-rs/bin/gptimage-gateway-rs` |
| 大小 / mtime | 7,242,096 B / 2026-07-21 10:31 |
| 类型 | ELF 64-bit LSB pie, x86-64, **not stripped** |
| BuildID | `d72eb5399291d757ef4c6f48033b70961193fadc` |
| 磁盘 sha256 | `c2f5a6fe868d5a565f19eb340e0710fecc85a14286b60039ed6d105a4b1a2aff` |
| `/proc/2769070/exe` sha256 | **同上，完全一致**（无 deleted 后缀） |
| 对应 commit | `7c34159`（Ship Rust MVP face）→ `7cb702d`（chmod +x），此后 bin 未再变更 |
| `strings` 可见源码路径 | **仅** `gateway/src/{main,config}.rs`、`helper_client/src/lib.rs` |

`strings` 里找不到 `auth`、`ticket_pool`、`control_client`、`error_class`、`image_contract` 任何符号。

> 附带泄露：二进制未 strip，`strings` 可提取构建主机路径 `/home/lenovo`。

### 1.4 端点覆盖

| 侧 | 端点数 |
|----|--------|
| Python `:8012` | **129** |
| Rust `:8013` 生产态 | **7** |
| Rust 本地工作树 | 15 |

生产的 7 条：`/health`、`/v1/models`、`/v1/accounts/candidates`、`/v1/quota`、`/v1/quota/refresh`、`/v1/chat/completions`、`/v1/images/generations`。

Rust 缺失的大类：

| 大类 | Python 端点数 | Rust | 说明 |
|------|--------------|------|------|
| 账号池管理 `/api/accounts/*` | 52 | 0 | 导入导出、re-login、Outlook 恢复、维护环、软封、调度策略、CPA 池、sub2api |
| 系统运维 `/api/{settings,logs,backups,images,proxy}` | 30 | 0 | 配置热更新、日志、备份、图床、代理运行时 |
| Ops 面 `/api/ops/*` | 20 | 0 | 风控日历、IP 养号、看门狗快照、humanlike 仪表盘、webshare CF 扫描 |
| OpenAI 兼容面 `/v1/*` | 11 | 4（部分） | 缺 `/v1/responses`、`/v1/messages`、`/v1/search`、PPT/PSD、`/files/*` |
| 注册机 `/api/register/*` | 7 | 0 | **永久非目标** |
| 异步图片任务 `/api/image-tasks/*` | 6 | 0 | 队列、取消、续轮询 |
| 图片资产 `/api/image-assets/*` | 3 | 0 | 参考图上传 |

### 1.5 路径 A 的溯源缺口（本次新发现）

| crate | 本地 `../gptimage` | panda 源码 | panda `.so` |
|-------|-------------------|-----------|------------|
| `image_schedule_trace` | 510 行 | **510 行，在** | 529,712 B @ 07-25 16:46 |
| `image_schedule_core` | 597 行 | **不存在** | 496,080 B @ 07-25 16:46 |

两个 `.so` 均被生产容器 `ro` 挂载进 `/app/native`，且**确认已启用**：
`services/image_pipeline/slot_ledger.py:268-277` 的 `SlotLedgerFacade` 检测到 `native/*.so` 即切 rust 后端
（`backend` 属性返回 `"rust"`），非死代码。

两个问题：

1. **`libimage_schedule_core.so` 正在生产中被调用，但 panda 上没有它的源码**，无法从本机溯源到任何 commit。
2. **路径 A 自己也大量未提交** —— `../gptimage` 里 `dispatch_gate.rs`、`lease_pool.rs`、`sediment.rs`、整个 `image_schedule_trace/` 全是 `??`。1,107 行中只有 509 行（`lib.rs` 296 + `slot_ledger.rs` 213）进了 git。

另：panda 的 `crates/image_schedule_trace/target/` 下有 debug + release + `x86_64-unknown-linux-gnu` 三套产物 ——
**这个 crate 曾在 panda 上编译过**，属部署铁律一的历史违规。本次未触碰，仅记录。

---

## 2. 文档维度

### 2.1 panda `/root/gptimage/docs/` 清单

| 文件 | 行数 | owner | 来源 |
|------|------|-------|------|
| `04-improvement-backlog.md` | 481 | 197609 | Windows 同步 |
| `28-scheduling-queue-slot-audit-20260726.md` | 422 | 197609 | Windows 同步 |
| `deployment.md` | 363 | root | 原生 |
| `upstream-sse-conversation.md` | 271 | root | 原生 |
| `flaresolverr-cloudflare.md` | 124 | root | 原生 |
| `27-pipeline-watchdog-monitoring-matrix.md` | 90 | 197609 | Windows 同步 |
| `README.md` | 52 | 197609 | Windows 同步 |
| `feature-status.en.md` | 44 | root | 原生 |
| `review.md` | 9 | root | 原生 |
| 合计 | **1,856** | | |

### 2.2 本地「生产快照」漏了 4 份

`comm` 文件名 diff 结果：

| 方向 | 文件 |
|------|------|
| **仅 panda 有** | `04-improvement-backlog.md`、`27-pipeline-watchdog-monitoring-matrix.md`、`28-scheduling-queue-slot-audit-20260726.md`、`README.md` |
| 仅本地快照有 | 无 |
| 两边都有（5 份） | `deployment` / `feature-status.en` / `flaresolverr-cloudflare` / `review` / `upstream-sse-conversation` |

共有的 5 份行数完全一致（9/44/124/271/363 = 811），mtime 均为 6-16，内容零漂移。

**即 `../gptimage-panda` 快照漏掉了 panda 上最新的 4 份文档**，恰好是唯一 4 份带当期审计结论的
（含 422 行的 `28` 调度审计）。**基于该快照做的 [23](23-rewrite-progress.md) / [24](24-gap-inventory.md) 分母口径需要复核。**

参照：本地开发树 `../gptimage/docs/` 有 38 个 `.md`，是 panda（9）的 4 倍多。

### 2.3 主题覆盖差异

| 维度 | Python 侧 | Rust 侧 |
|------|----------|---------|
| 部署运维 | `deployment.md` 363 行（compose/Nginx/env/备份） | `17-operator-guide.md` 57 行（bringup 三行 + 回滚） |
| CF403 / egress | 专文 124 行 + compose 成套 flaresolverr/warp | **零**，一行外推给「号池侧」 |
| 调度 / 队列 / 槽位 | `28` 422 行 + `27` 90 行 | **零**（而 `ticket_pool` 正在重造这块） |
| SSE 上游语义 | `upstream-sse-conversation.md` 271 行 | **零**（事件覆盖 0/20） |
| 配置热更新 | 71 个顶层键运行时可改 | 18 个 env，需重启；**该降级未记录在任何文档** |
| 后台循环 / 看门狗 | 7 个文件含 `create_task`/`while True` | 0 循环 0 文档 |
| 协议契约 | 无独立文档 | `00-contract.md` 88 行 |
| 鉴权 / UI | 无 | `21-auth-and-ui.md` 80 行 |
| 自我审计 | 无 | **879 行**（`22`+`23`+`24`） |

### 2.4 文档矛盾清单

| # | 矛盾点 | 声称 | 实测 |
|---|--------|------|------|
| 1 | 能力探测端点 | `17`：`curl :8013/api/backend/capabilities` | **HTTP 404** |
| 2 | 鉴权已上线 | `17` + `21` 整篇讲 JWT/登录/UI | `/api/auth/me`、`/api/auth/login`、`/api/admin/users` 全 **404**，`strings` 无 auth 符号 |
| 3 | bringup 注入 `AUTH_*` | `17:23` | 脚本实设 `AUTH_DISABLE=1`，不注入 |
| 4 | `IMAGE_ENABLED` 默认关 | `17:32` | 代码默认 false 但 bringup 设 `=1` |
| 5 | **Python 基线 CPU/内存** | `13`：空闲 CPU ~0.5–0.7% / Mem 160–230MiB | **98.5% CPU / 224MB** |
| 6 | `/v1/images/edits` 错误码 | `21`：`image_deferred` | 实际 `image_edits_deferred` |
| 7 | 仓库布局 | `plan.md` §4 列 5 crate | 工作树实有 6 个 |
| 8 | **panda `docs/README.md` 索引整体失效** | 索引 12 个目标（`16-camoufox`/`17-cf403`/`19`–`26`/`plans/`/`reference/`/`captures/`/`archive/`/`logs/`） | panda `docs/` 下**一个都不存在** |
| 9 | `27` 被 `28` 推翻 | `27` 多处标 ✅ 已实现 | panda 自家 `README` 明写「多处 ✅ 已被 `28` 推翻」 |

第 8 条本次新发现：那份 README 从 Windows 同步过去时照搬了开发树 38 份文档的结构，
但被索引的文件一个都没同步 —— **panda 上的文档导航整体失效**。

---

## 3. 运行时维度

### 3.1 端点响应实测

| 端口 | 路径 | 状态 | 响应 |
|------|------|------|------|
| 8012 | `/health` | 200 / 0.813s | 3,798B **HTML 仪表盘**（`<title>号池健康监控</title>`） |
| 8012 | `/version` | 200 | `{"version":"1.5.0","backend_commit":"","build_drift":false}` |
| 8013 | `/health` | 200 / 0.675s | 258B JSON，见下 |
| 8013 | `/v1/models` | 200 | 硬编码 2 个：`gpt-4o-mini`、`gpt-image-2` |
| 8013 | `/api/auth/me` | **404** | 空 |
| 8013 | `/api/auth/login` | **404** | 空 |
| 8013 | `/api/admin/users` | **404** | 空 |
| 8013 | `/api/backend/capabilities` | **404** | 空 |
| 8013 | `/v1/images/edits` | **404** | 空 |
| 19001 | `/health` | 200 | `{"ok":true,"service":"protocol-bridge","gptimage_root":"/app"}` |

> ⚠️ 注意路由前缀：真实路径是 `/api/auth/me`（`main.rs:99` 的 `.nest("/api/auth", ...)` 之下），
> **`/me` 从未在任何版本注册过** —— 拿 `/me` 的 404 当「未部署」的证据是无效的。
> 上表用的是真实路径，结论成立。

`:8013/health` 全文：

```json
{"accounts":1,"helper_ok":true,"image_global_concurrency":3,"listen":"0.0.0.0:8013",
 "min_image_quota":1,"multi_account":true,"ok":true,
 "pin_email":"qaf***@proton.me","proto_bridge":true,
 "runtime":"rust","service":"gptimage-gateway-rs","wave":"mvp"}
```

注意 `"wave":"mvp"`、`accounts:1`、`0.0.0.0` 监听、且**未鉴权回显 `pin_email` 明文**。

### 3.2 容器挂载差异

镜像 ID 完全相同：`sha256:aebeee26a05a…`（两者均 `chatgpt2api:local`，960MB）。

| 挂载 | 生产 `chatgpt2api-local` | helper |
|------|-------------------------|--------|
| `api` / `services` / `utils` / `scripts` | → `/app/*` (ro) | → `/app/*` (ro) |
| `data` | → `/app/data` (**rw**) | → `/app/data` (**rw**) |
| `config.json` | (**rw**) | (ro) |
| `web_dist` | (ro) | **无** |
| `native` | (ro) | **无** |
| `/root/gptimage-gateway-rs` | 无 | → `/opt/gws` (rw) |
| 启动命令 | `uv run uvicorn main:app --host 0.0.0.0 --port 80` | `/app/.venv/bin/python3 protocol_bridge.py` |
| 网络 | bridge，`8012→80` | **host 模式** |

**结构性事实**：helper 把生产的 `api`/`services`/`utils` 原样挂进 `/app`，
`protocol_bridge.py:28-29` 再 `sys.path.insert(0, GPTIMAGE_ROOT)`（`/health` 回显 `"gptimage_root":"/app"` 佐证）。

即 **Rust 网关的数据面不是移植副本，而是通过 `sys.path` 反向调用同一份 Python 生产代码**。推论：

- Rust 二进制**无法脱离 gptimage Python 树独立部署**
- 两个服务共享同一份 `/root/gptimage/data` 且**都是 rw** —— 存在并发写同一 SQLite/JSON 的风险面
- 「Rust 重写」在数据面上目前是 0 —— 换的是外壳，不是内核

---

## 4. 差异总览

### Rust 侧缺什么

1. **122/129 个 HTTP 端点**（生产 129，Rust 生产态 7）
2. **全部上游数据面能力** —— SSE 事件 0/20、重试退避 0/13、后台循环 0/7+
3. **配置热更新** —— 71 键可热改 vs 18 个 env 需重启
4. **全部运维知识文档** —— CF403/egress、调度双槽（422 行）、SSE 语义（271 行）、看门狗矩阵
5. **CI 与编译闸门** —— 无 `.github/`，所以 E0277 与 CORS panic 能长期留存

### Rust 侧多什么

1. **JWT 鉴权与用户表**（651 行）—— 但未提交也未部署，`/api/auth/me` 实测 404
2. **协议契约的机器可验证化** —— `00-contract.md` + `fixtures.rs` + `error_class.rs` + `image_contract.rs`
3. **自我审计文档 879 行** —— 这份自查密度 Python 侧没有
4. **单进程足迹极小** —— RSS 5.2MB / 3 线程 / 连续 2d23h 无重启
5. `ticket_pool` 285 行 —— **负资产**：与已上生产的 `pre_ticket_pool.py` 重叠 ~80%，零引用、编译失败

### 不一致什么

1. **本地 2,707 行 vs 已提交 943 行** —— 1,764 行滞留未跟踪工作树，按 git 链路当前到不了 panda
2. **文档描述的系统 ≠ 运行的系统** —— 约 137 行文档写的是一个从未部署过的版本
3. **性能基线数字失真** —— `13` 记 CPU 0.5–0.7%，实测 98.5%，偏两个数量级
4. **两条 Rust 化路径互不知晓**，且路径 A 的 `image_schedule_core` **源码在 panda 上不存在**
5. **文档同步单向且不完整** —— 本地快照漏 4 份；panda README 索引 12 个目标全部不存在

---

## 5. 复现方式

```bash
# 代码规模（去噪）
ssh panda 'cd /root/gptimage && find . -name "*.py" \
  -not -path "./backups/*" -not -path "./web_dist*" -not -path "./.venv/*" \
  -not -path "*/__pycache__/*" | xargs wc -l | tail -1'

# 已提交 vs 工作树
git ls-files crates/ | xargs wc -l | tail -1     # 943
find crates -name '*.rs' | xargs wc -l | tail -1  # 2707

# 二进制溯源
ssh panda 'sha256sum /root/gptimage-gateway-rs/bin/gptimage-gateway-rs /proc/$(pgrep -f gptimage-gatewa)/exe'
ssh panda 'strings /root/gptimage-gateway-rs/bin/gptimage-gateway-rs | grep -o "crates/[a-z_]*/src/[a-z_]*\.rs" | sort -u'

# 文档 diff
ssh panda 'ls /root/gptimage/docs/' > /tmp/panda.txt
ls ../gptimage-panda/docs/ > /tmp/snap.txt
comm -23 /tmp/panda.txt /tmp/snap.txt

# 端点实测
ssh panda 'for p in /health /me /api/backend/capabilities; do
  curl -s -o /dev/null -w "$p %{http_code}\n" localhost:8013$p; done'
```
