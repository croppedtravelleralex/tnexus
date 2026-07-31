# 17 — 操作指导

最后更新：2026-07-26

> ⚠️ 本文 2026-07-23 版描述的鉴权 / 静态 UI / capabilities **在 panda 上并不存在**。
> 2026-07-26 panda 本机 curl 实测：`/api/auth/me`、`/api/auth/login`、`/api/admin/users`、
> `/api/backend/capabilities`、`/v1/images/edits` **全部 404**，且运行中二进制 `strings` 无任何 auth 符号。
> 原因见 [25](25-panda-vs-rust-20260726.md) §0 —— 相关代码 1,764 行**从未提交进 git**
> （`web/` 更是整个未跟踪），panda 跑的是 2026-07-21 的 943 行 MVP。
> 下文已按实测改写；恢复这些能力的前置条件见 §「让文档变成真的」。

## 读哪份

| 意图 | 文档 |
|------|------|
| 进度与路线 | `../plan.md` |
| 两侧实测差异 | [25-panda-vs-rust-20260726.md](25-panda-vs-rust-20260726.md) |
| 性能实测与预估 | [26-perf-measured-20260726.md](26-perf-measured-20260726.md) |
| 问题清单 / 虚假门禁 | [22-audit-2026-07-26.md](22-audit-2026-07-26.md) |
| 能力差距 | [24-gap-inventory.md](24-gap-inventory.md) |
| 鉴权 / Web UI | `21-auth-and-ui.md`（**描述的是未部署版本**） |
| 协议红线 | `00-contract.md` |
| 是否出门 | `18-test-matrix.md` |
| CF403 / 出口 | `../gptimage/docs/17-cf403-and-egress.md`（**号池侧**） |

## Panda 拓扑（2026-07-28 更新）

| 服务 | 端口 | 状态 |
|------|------|------|
| `chatgpt2api-local` | **8012** | 生产 Python（公网），**不动** |
| ~~`gptimage-gateway-rs`~~ | ~~**8013**~~ | **已退役**（2026-07-28 停 helper + 无监听） |
| ~~`protocol_bridge` helper~~ | ~~**19001**~~ | **已移除** |

Rust 重写项目**不在 Panda 半成品阶段部署**。开发在本地 WSL；Panda 仅用于只读导号（`export_pin_account.py`）或一次性 `upstream-probe` 验证。

### 历史（:8013 MVP，已取消）

- 曾用 `scripts/panda_bringup_rust_face.sh` —— **脚本已禁用**
- 回滚（若误启动）：`docker rm -f gptimage-gateway-rs-helper gptimage-gateway-rs-mvp` + `pkill -f gptimage-gateway-rs`

### helper 与生产共享同一份代码

helper 容器与生产容器用**同一个镜像**（`chatgpt2api:local`，镜像 ID 相同），
把 `/root/gptimage` 的 `api`/`services`/`utils` 原样 ro 挂进 `/app`，
再由 `protocol_bridge.py:28-29` `sys.path.insert`。

推论：Rust 二进制**无法脱离 gptimage Python 树独立部署**；
且两个容器都以 **rw** 挂载同一份 `/root/gptimage/data`，存在并发写风险面。

## 鉴权 / UI（Phase A+，**代码在、未部署**）

以下为**目标状态**，当前 panda 上未生效：

- 首次启动：设 `AUTH_JWT_SECRET`（≥32B）+ `AUTH_BOOTSTRAP_ADMIN_*`
- 数据：`data/auth.db` 持久卷
- 静态 UI：构建 `web/out` 后设 `GATEWAY_STATIC_DIR=/root/gptimage-gateway-rs/web/out`
  —— ⚠️ **当前不可达**：`git ls-files web` = 0 条，整个 `web/` 未跟踪，且 `.gitignore` 另忽略
  `node_modules/` `.next/` `out/`，所以按 git 部署链路静态产物永远送不到 panda。
  另：`main.rs:75-78` 带 `.filter(|p| p.is_dir())`，目录不存在时**静默降级不报错**，
  只在 `/health` 的 `static_ui` 字段可观测。
- 生图：**代码默认关** `IMAGE_ENABLED=0`，但 bringup 设 `${IMAGE_ENABLED:-1}`，
  **panda 当前实际为 1**；勿在 CF 窗前强行验收生图 KPI

### 让文档变成真的（前置条件）

1. **摘除或冻结 `ticket_pool`**（`../plan.md` §6.0 / §6.3）—— 4×E0277 全在这个 crate
2. 修 `main.rs:129` CORS —— `allow_origin(Any)` + `allow_credentials(true)` 组合下
   带 auth 的构建**启动即 panic**。现网没崩只是因为二进制里没有这段代码
3. 清 7 个含真实邮箱的脚本，然后提交 1,764 行 Rust **外加整个 `web/`** 与 `fixtures/protocol/`
4. 反转 bringup 默认值并注入 `AUTH_*` / `GATEWAY_STATIC_DIR`
5. 重新编译 → git push → panda `git pull` + bringup（**禁止在 panda 上编译**）

顺序不可换：第 2 步必须在部署之前，否则部署上去的是一个启动就 panic 的二进制。

## 观察号纪律

- 并发压测用异号 + 异 `proxy_host`
- 矩阵（生图）：`python3 scripts/mvp_rust_conc_matrix.py http://127.0.0.1:8013`
  （panda 当前 `IMAGE_ENABLED=1`，可直接跑；受 CF 窗影响，见 `18-test-matrix.md`）

## 故障树（self vs upstream）

| 现象 | 先查 |
|------|------|
| `401` / `403` on `/v1/*` | 未登录或 member 访问 admin 路由（**当前 `AUTH_DISABLE=1`，不会出现**） |
| `404` on `/api/auth/*` `/api/admin/*` `/api/backend/capabilities` `/v1/images/edits` | **正常**：生产二进制不含这些路由，见本文顶部 |
| `501 image_deferred` | 仅当显式传 `IMAGE_ENABLED=0` 才会出现（panda 当前为 1） |
| `501 image_edits_deferred` | **部署后**才会出现 —— `image_edits` 不读 `image_enabled`，无条件返回；生产态该路由是 404 |
| CF HTML / conversation 403 | **`upstream`**；egress |
| estuary 403 | `self`：是否丢 Bearer |
| SSE 僵死 | `self`：post_ready/wall/cancel |

## 脱敏

`rg -n "Bearer |eyJ" data/runlogs` 应无业务命中。

> ⚠️ `scripts/check_runlog_desense.py` 的扫描目录写死在被 `.gitignore` 清空的路径，
> 干净检出下**恒返 `DESENSE_OK`**，是构造性虚假门禁。修法见 [22](22-audit-2026-07-26.md) §2.2。

## 本阶段不要做

- 以生图 KPI 阻塞 UI/鉴权交付
- Panda 上 `cargo build` / `docker build`
- 改生产 Nginx / `:8012`
- **引用 `13-perf-baseline-compare.md` 的 Rust 收益预估**（已作废，见 [26](26-perf-measured-20260726.md)）
- 把 `/api/auth/me` 等 404 当成故障 —— 那是未部署，不是坏了
