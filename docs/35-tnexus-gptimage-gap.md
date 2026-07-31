# 35 — TNexus 距离彻底替代 Python gptimage 还差多少

最后更新：**2026-07-31（去代理化 + 执行面接线）**

## 结论（一句话）

**管理台已去生产 HTTP 代理**（禁止 `GPTIMAGE_ADMIN_TOKEN` → gptimage `:8012`）。  
运维执行面走 **account-ops `:9011` + 本地 `GPTIMAGE_ROOT` Python 库**（与 refresh/relogin 同链路，非运行时依赖 Panda 生产 API）。  
**数据面生图/对话**走 **gateway `:8014` Rust upstream**。

---

## 进度总览（百分比）

| 维度 | 完成度 | 说明 |
|------|--------|------|
| **号池管理台 UI** | **~92%** | 全列排序、慢刷/软封/编辑、流水图、养号日历、图片管理优化 |
| **管理 API（tnexus-api）** | **~78%** | 去代理；本地 stats/breakdown/流水；account-ops 委托 |
| **运维执行面（养号/Outlook/预热）** | **~70%** | account-ops 已接线；需 `GPTIMAGE_ROOT` + 凭据；结果回写 JSON 待加强 |
| **Gateway OpenAI 兼容** | **~55%** | chat + image n=1 + b64/url；调度门已加；edits/n>1/背压未全 |
| **Gateway 调度门 / dispatch** | **~45%** | scheduling 文件门控 + inflight；无 dispatch_gate/interval/ticket_pool |
| **数据面（替代 Python 核心）** | **~35%** | upstream 主链通；humanlike/ACI/全队列背压未齐 |
| **彻底替代 Python gptimage（加权）** | **~38%** | 管理面可交付缺口缩小；数据面仍是大头 |

> 百分比为功能加权估算，对照 `docs/24-gap-inventory.md` 与 Panda 生产 `:8012` 行为，**不含**明确不做项（注册机、settings 七卡片、完整 ops-dashboard 壳等）。

---

## 本轮已完成（2026-07-31）

### 去代理化（P0）

| 项 | 状态 |
|----|------|
| 删除 `gptimage_proxy.rs` 及所有 `:8012` HTTP 代理 | ✅ |
| `GPTIMAGE_ADMIN_TOKEN` 废弃（`.env.example` 已标注） | ✅ |
| 无本地能力时返回 503 /「暂无」，不借用生产 | ✅ |

### 号池 UI / API

| 能力 | 实现 |
|------|------|
| 全列单点升/降序排序 | ✅ `account-sort.ts` + 表头 |
| 图片管理缩略图 / 懒加载 / WebP thumb API | ✅ |
| 灯箱滚轮 0.05×–20× + 拖拽 | ✅ |
| 本地 stats / schedulable-breakdown / activity 流水 | ✅ `usage_events` + 号池 |
| 全量慢刷 refresh-all | ✅ 本地 `refresh_all.rs` |
| 软封 / 删除 / 更新账号 | ✅ 本地 JSON |
| 养号日历预设/绑定 | ✅ 本地 JSON + `ip-nurture` API |
| 生图 b64 回传 | ✅ jobs + 图片列表 |

### 运维执行面（account-ops → gptimage 库）

| 能力 | 端点 | 前置 |
|------|------|------|
| 养号 status/enable/enqueue/process-one | `/v1/nurture/*` | `ACCOUNT_OPS_TOKEN` + `GPTIMAGE_ROOT` |
| Outlook 单账号恢复 + 进度 | `/v1/outlook/recover-*` | 同上 + Outlook 凭据 |
| Outlook 自动恢复环 | 启动时 `start_background()` | 同上 |
| 窗口预热 quota-window prime | `/v1/quota-window/prime` | 同上；回退 `GATEWAY_BASE` 生图 |
| TNexus API 委托 | `/api/ops/nurture/*`、`/api/accounts/recover-outlook` 等 | account-ops 可达 |

### Gateway（需单独部署 `:8014` 二进制/镜像）

| 能力 | 状态 |
|------|------|
| `scheduling_gate.rs` 读 `SCHEDULING_STATE_FILE` + `ACCOUNTS_FILE` | ✅ |
| 生图选号过滤（状态/调度/额度/软封/inflight） | ✅ |
| `scheduling_bulk` 写调度状态 | ✅ |
| 生图 inflight 计数 | ✅ |
| 429 `Retry-After: 30` | ✅ |

---

## 仍缺 —— 管理面

| # | 能力 | 状态 | 优先级 |
|---|------|------|--------|
| 1 | 养号/预热结果自动 merge 回 TNexus JSON | ⚠️ 部分（refresh 有，预热/养号待加强） | P1 |
| 2 | 号池 Outlook 恢复 UI（进度条/自动恢复卡片） | ❌ API 已有 | P1 |
| 3 | `image_inflight` 与 gateway 双写一致性 | ⚠️ 文件竞态风险 | P2 |
| 4 | Outlook 凭据 / YUMAIL 生产配置文档 | ⚠️ 依赖 Panda secrets | P2 |
| 5 | Panda sync / backup / CPA | ❌（明确不做或低优） | — |
| 6 | settings 七卡片 / 完整 ops-dashboard | ❌（明确不做） | — |
| 7 | 账号 Postgres 持久化 | ❌ 仍 JSON 文件 | P2-D |

**管理面可交付缺口**：约 **8%**（主要为 UI 与状态回写）。

---

## 仍缺 —— Gateway / 数据面

| 模块 | 状态 |
|------|------|
| `dispatch_gate` / lease_pool / interval 门 | ❌ |
| `ticket_pool` 接入 upstream | ❌ 冻结 |
| `POST /v1/images/edits` | ❌ 501 |
| `n>1`、`quality`、duplicate_prompt | ❌ |
| OpenAI 标准 error `type` 分档 | ⚠️ 部分 |
| humanlike 调度 / workload 策略 | ❌ |
| `image_task_service` 同级队列背压 | ⚠️ TNexus worker 子集 |

**数据面功能加权**：约 **35%**（与 `docs/23-rewrite-progress.md` 口径接近，调度门 +5%）。

---

## 部署与验收（Panda）

### 环境变量（`/opt/tnexus/.env`）

```bash
# 必配
ACCOUNTS_FILE=/data/pool/accounts_pool.json
SCHEDULING_STATE_FILE=/data/pool/scheduling_state.json
ACCOUNT_OPS_BASE=http://127.0.0.1:9011
ACCOUNT_OPS_TOKEN=<随机>
GPTIMAGE_ROOT=/gptimage          # account-ops 只读挂载

# 窗口预热回退（account-ops 不可用时）
GATEWAY_BASE=http://127.0.0.1:8014
GATEWAY_AUTH_KEY=<gateway JWT 或 API key>
QUOTA_PRIME_PROMPT=a tiny red dot on white background

# Gateway 容器/进程（与 TNexus 镜像分离）
IMAGE_ENABLED=1
# 与 api 共享 pool 路径时需挂载同一 ACCOUNTS_FILE
```

**禁止**：`GPTIMAGE_ADMIN_TOKEN` 指向生产 `:8012`。

### 部署命令

```bash
export TNEXUS_ROOT=/root/TNexus
cd "$TNEXUS_ROOT" && git pull
bash deploy/panda/export_pool.sh
bash deploy/panda/deploy.sh
# gateway :8014 若单独进程，需另行更新 gateway 二进制并重启
```

### 验收清单

```bash
# 健康
curl -fsS http://127.0.0.1:9000/health
curl -fsS http://127.0.0.1:9011/health

# 登录后（cookie）
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts?limit=1'
curl -fsS -b cj 'https://tnexus.relai.asia/api/ops/nurture/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/outlook-recovery/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/quota-window/prime/status'
curl -fsS -b cj 'https://tnexus.relai.asia/api/accounts/schedulable-breakdown'

# 静态页
curl -fsS -o /dev/null -w '%{http_code}\n' https://tnexus.relai.asia/accounts
curl -fsS -o /dev/null -w '%{http_code}\n' https://tnexus.relai.asia/image-manager

# 全链路（可选）
python3 /root/TNexus/scripts/prod_url_chain_test.py
```

### UI 验收要点

1. **号池** — 各列表头点击排序；慢刷面板；行内预热/对话/软封  
2. **图片管理** — 小卡片、懒加载；点击灯箱滚轮缩放/拖拽  
3. **运维** — 养号 Tab（需 account-ops + GPTIMAGE_ROOT）；无配置时应显示不可用而非连生产  
4. **Outlook** — API 可通；UI 卡片待下一版  

---

## 架构示意

```text
浏览器 → tnexus.relai.asia (nginx)
           ├─ /api/*     → tnexus-api :9000  (JSON 号池 + 委托 account-ops)
           ├─ /v1/*      → gateway :8014     (生图/对话 + scheduling_gate)
           └─ /*         → 静态 UI

account-ops :9011 → GPTIMAGE_ROOT Python 库（sqlite accounts.db）
                  → text_nurture / outlook_recovery / quota_window_prime

禁止 → gptimage 生产 :8012 HTTP
```
