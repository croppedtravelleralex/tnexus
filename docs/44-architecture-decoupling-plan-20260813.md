# TNexus 架构解耦与优化方案（2026-08-13）

> 配套：[ARCHITECTURE.md](ARCHITECTURE.md) 拓扑导航 · [plan.md](../plan.md) 施工总控 · [43-standalone-readiness-audit-20260812.md](43-standalone-readiness-audit-20260812.md)
>
> 本文只写**已验证的结构缺陷**与**对应改造**，不重复拓扑图。
> 每条结论标注 `[实测]`（生产 curl/SQL 取证）或 `[代码]`（源码可核对）。

---

## 0. 结论先行

「哪哪都不通、哪哪都不知道行不行」不是错觉，有四个结构性来源：

| # | 根因 | 一句话症状 | 严重度 |
|---|------|-----------|--------|
| A | **运行时读启动快照，PG 是真源却传不到选号器** | 管理台改了没反应，要重启容器 | P0 |
| B | **同一事实多份副本，没有单一裁判** | 三个地方看到三个数，不知道信谁 | P0 |
| C | **失败被吞成"成功"或"空数据"** | 报告全绿，实际在坏 | P0 |
| D | **契约各写各的**（分页/封装/端口/cookie 四套） | 前端拿到的和后端给的对不上 | P1 |

外加一项历史包袱：**E. ETL 遗产表**（11 张 0 行 + 若干 Go 时代冻结快照）让管理台展示的一部分数据永远不会变。

---

## A. 启动快照耦合（P0，最高优先）

### A.1 事实

- `SimplifiedPool::load` 全仓库**只有一处调用**：`crates/grok2api-rs/src/main.rs:97`，在启动路径上。没有定时任务、没有 admin 刷新端点。`[代码]`
- `load()` 会**清空全部运行时调度状态**：`cooldown_until`、`rl_failure_count`、`success/failure_count`、`select_seq`、`last_selected_seq`（`crates/grok-pool/src/lib.rs:83-91`）。`[代码]`
- 生产实测（2026-08-13 20:44 CST）：`[实测]`
  - 容器启动 `11:04:51Z`，启动日志 `号池已从 PG 加载 537 个`
  - PG 当前 `enabled = true` 的 grok_web 账号：**537**（成员数暂时一致）
  - **容器启动后被改动过的 `grok_accounts` 行数：698**（表内共 707 行）

### A.2 这意味着什么

成员数今天恰好没漂，但**健康状态漂得很彻底**：PG 侧 698 行的 `cooldown_until` / `last_error` / `failure_count` 一直在被 `PgHealthSink` 和额度任务更新，而选号器完全看不见；反过来选号器自己那套 2 秒内存冷却也从不回写 PG。

于是形成两个平行宇宙：

- **管理台看 PG** → 显示某号在冷却
- **选号器看内存** → 照样把请求发给它
- 运维在管理台禁用一个坏号 → **在容器重启前不生效**

这解释了此前反复出现的"额度/调度看起来不对但查不出原因"。

### A.3 改造：增量对账（reconcile），不是整池重载

**不能**简单挂个定时器调 `load()` —— 那会把冷却和公平轮转游标每 60 秒清零一次，被限流的号立刻重新可选，直接重演饿死问题。

正确做法是新增 `SimplifiedPool::reconcile(&repo)`：

1. 拉取 PG 当前 `enabled + grok_web` 集合
2. **新增**：池中没有的账号入池（初始状态干净）
3. **移除**：PG 已禁用/删除的账号出池，并清掉它的计数条目
4. **保留**：两边都在的账号，其 `cooldown_until` / `last_selected_seq` / 计数**原样不动**
5. 合并 PG 侧 `cooldown_until`：取内存与 PG 二者中较晚的那个作为生效冷却

然后挂到既有任务调度器（`crates/grok2api-rs/src/tasks.rs`，受 `GROK_TASKS_ENABLED` 控制，生产已为 `1` `[实测]`），周期 60s。

**验收**：在 PG 里禁用一个账号，≤60s 后该号不再被选中，且其余账号的轮转顺序与冷却不受影响。

---

## B. 多副本无单一裁判（P0）

### B.1 已确认的副本清单

| 逻辑事实 | 副本 | 今日真源 | 读者怎么知道 |
|---------|------|---------|-------------|
| GPT access_token | SQLite `accounts.db` / PG `tnexus_accounts` / gateway 内存 / api 内存（**4 份**） | SQLite（8012 刷新）→ 每小时 ETL 进 PG | 只能看 JWT `exp` 或等 401 |
| Grok 账号可用性 | PG `grok_accounts.enabled` / `pure_http_keys/` 目录 / 内存池成员（**3 份**） | 二者都必要：PG 决定入池，keys 决定能否真的发请求 | 无 key 时 503 |
| Grok 冷却/健康 | PG 列 / 内存池 | **无裁判**（见 A） | 无 |
| 调度开关 | `scheduling_state.json` | 文件（已加 fs2 锁 + 原子 rename） | — |

### B.2 死副本

`tnexus_account_runtime` **实测 rows=0**，全仓库无代码引用 —— 迁移里建了但从未接线。`[实测]``[代码]`

### B.3 改造

- **短期**：为每个多副本事实指定唯一裁判并写进 `ARCHITECTURE.md`；在 health 端点暴露"副本是否一致"（见 C.3）
- **中期**：GPT token 四副本收敛到两副本（真源 + 各进程只读缓存 + 显式 reload 端点，已有）
- **清理**：`tnexus_account_runtime` 要么接线要么下线，不留"看起来有用"的空表

---

## C. 静默失败（P0，已开工）

### C.1 本轮已修

| 位置 | 原行为 | 现行为 |
|------|--------|--------|
| `tnexus-account-ops` `refresh-one` | 恒返回 `{ok:true}`，刷新失败也算成功，旧 token 被当新的写回库 | 失败返回 502 + 真实原因；错误标记回写账号，可在库里查 |
| `tnexus-api` `nurture_status` | account-ops 调不通时伪造"养号服务未配置" | 区分"未配置"与"不可达"，真实错误进 `last_error` |

`refresh-one` 那处是 Python→Rust 移植时丢失的错误处理（`helper/account_ops_face.py` 原本抛 502）。生产佐证：有账号自 07-30 / 08-07 起未换过 token，而刷新任务一路报成功。`[实测]`

### C.2 待修（按爆炸半径）

| 位置 | 问题 |
|------|------|
| `grok-audit/src/sink.rs` | 批量写失败只计数，计数**无任何 HTTP 暴露**；调用方 `let _ = record()` |
| `crates/upstream/src/poll.rs` | `if let Ok(tasks) = query_tasks(...)` 丢弃 Err，查询失败不计入终止条件，轮询空转到超时 |
| `web/**` 11 处 | `.catch(() => [])` 把错误变成空列表，UI 显示"没有数据"而非"出错了" |
| `grok-ops/four_pool.rs` | 大量 `let _ = ` 写回，探针结果失败无声 |

### C.3 通用对策：把"现在好不好"变成一条 curl

现状实测：`/healthz` 只回 `{"status":"ok"}`，号池空、审计写挂、额度全 stale 时**依然返回 200**。`[实测]`

建议扩展一个 `/readyz` 明细（或 `/admin/health`），至少包含：

```
pool_size / pool_reconciled_at        ← A 的验收面
quota_oldest_synced_at                ← 额度新鲜度
audit_batch_failures                  ← 审计是否在丢行
credential_missing_count              ← 有多少号缺 key
```

这是用户"哪哪都不知道行不行"的直接解药，成本最低、收益最高。

---

## D. 契约不一致（P1）

| 维度 | 现状 |
|------|------|
| 分页 | 三套：`offset/limit`（tnexus-api）、`page/pageSize`（grok-admin）、OpenAI `data[]` |
| 列表封装 | `{items}` / 裸数组（`/api/conversations`、`/admin/analytics/*`） / `{accounts}`（gateway candidates）三种并存 |
| `total` 语义 | accounts、request-audits 是真 COUNT；**models、client-keys、media 是 `items.len()`** → 分页控件静默错误 |
| 端口默认值 | gateway 监听默认 `8013`，生产 `8014`，`.env.example` 写 `8012` |
| 会话 cookie | tnexus `tnexus_session` vs gateway `gws_session`，两套登录态 |

**改造顺序**：先修假 `total`（3 处，用户可见的错）→ 再统一列表封装 → 最后收敛端口/cookie 默认值。共享类型（OpenAPI 生成或 Rust→TS 类型导出）作为 P2，收益高但工程量大。

---

## E. ETL 遗产（P1，清理）

生产实测的 grok_* 表行数：`[实测]`

- **0 行（11 张）**：`grok_account_provider_links`、`grok_billing_reservations`、`grok_billing_snapshots`、`grok_chrome_tickets`、`grok_media_assets`、`grok_media_jobs`、`grok_model_quota_blocks`、`grok_pipeline_segments`、`grok_pipeline_traces`、`grok_quota_recovery`、`grok_runtime_settings`
- **Go 时代冻结快照**：`grok_pool_snapshots` 4155、`grok_model_capabilities` 1450、`grok_egress_traffic_hops` 1186、`grok_web_profiles` 698、`grok_model_sync_states` 707、`grok_egress_nodes` 124

其中两条值得单独点名：

- `grok_pipeline_traces` / `grok_pipeline_segments` **均为 0** → Grok 生图全链路**没有任何追踪落库**，出问题只能靠日志
- `grok_runtime_settings` **为 0** → 运行时设置功能实际是空转

**改造**：每张表二选一 —— 接线（补 writer）或下线（迁移里删表 + 管理台移除入口）。不允许继续以"有表有数据"的样子误导排查。

---

## F. 额度刷新的真实新鲜度契约

实测：`grok_quota_windows` 共 2055 行，最新同步就在当前分钟，但 **1809 行（88%）超过 10 分钟未同步**。`[实测]`

这不是故障，是吞吐算术：每 60s 刷约 32 个窗口，跑完 2055 个需要 **约 64 分钟**。也就是说单个账号额度的真实新鲜度是"约 1 小时"，而不是任何人以为的 60 秒。

**改造**：把这个数字写进文档和 UI（额度旁标注同步时间），而不是让人误以为是实时值。若要提高，只能加并发或缩小刷新集合（例如只刷最近被选中过的账号）。

---

## 优先级与收益

| 序 | 动作 | 工作量 | 收益 |
|----|------|--------|------|
| **P0-1** | Grok 号池增量对账 + PG 冷却合并 | 中（新方法 + 任务接线 + 单测） | 管理台操作即时生效；不再靠重启；消除 A 类"两个平行宇宙" |
| **P0-2** | health/readyz 暴露四项关键指标 | 小 | 一条 curl 回答"现在好不好"，直接解决可验证性缺口 |
| **P0-3** | 修 `poll.rs` 丢弃 Err + audit sink 计数暴露 | 小 | 生图不再空转到超时；审计丢行可见 |
| **P1-1** | 修 3 处假 `total` | 小 | 管理台分页不再静默错 |
| **P1-2** | 前端 11 处 `.catch` 改错误态 | 中 | "没有数据"与"出错了"可区分 |
| **P1-3** | 11 张空表接线或下线 | 中 | 排查时不再被死数据误导 |
| **P2-1** | 列表封装统一 + 共享类型 | 大 | 根治契约漂移 |
| **P2-2** | 端口/cookie 默认值收敛 | 小 | 减少配错 |

**整体收益**：P0 三项做完后，"改了生效吗 / 现在好不好 / 失败了会不会告诉我"这三个问题都能用一条命令回答；这正是当前最缺的东西，也是后续所有优化的前提。

---

## 明确不做

- 不拆分 monorepo：两棵 Rust 树（GPT / Grok）运行时零共享 crate，边界已经清晰，拆仓只增加发布复杂度
- 不合并两套鉴权体系：gateway 独立 auth DB 是 OpenAPI 对外服务的隔离需求
- 不重跑 grok ETL：它是 `TRUNCATE ... RESTART IDENTITY` 全量替换，会抹掉 Rust 运行时写入的行
