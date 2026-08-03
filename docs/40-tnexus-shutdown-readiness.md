# 40 — TNexus 停服 gptimage 就绪清单

最后更新：**2026-08-03**

> **当前策略**：**不停** `chatgpt2api-local :8012`；先把 TNexus 推到 **≥95% 停服就绪度** 再切流。

## 停服就绪度（加权）

| 阶段 | 加权 | 说明 |
|------|------|------|
| 2026-08-03 部署后 | **~90%** | Rust account-ops + gateway 调度子集 |
| 本轮代码（养号用量、8014 默认、ε-greedy） | **~91–92%** | 待 CI + Panda deploy 后复测 |
| **可停 :8012 目标** | **≥95%** | Postgres 切库 + humanlike 压测 + 灰度 7 天 |

一键自检（Panda）：

```bash
python3 /root/TNexus/scripts/shutdown_readiness_check.py
python3 scripts/test_humanlike_distribution.py --n 20 --concurrency 4
```

## 停服前必过项（P0）

| # | 项 | 验收 |
|---|-----|------|
| 1 | 号池 Postgres 独立 | `ACCOUNTS_BACKEND=postgres`；无 `/root/gptimage/data` 卷 |
| 2 | humanlike 对照 | `test_humanlike_distribution.py` 偏差 &lt;15% |
| 3 | 10 并发压测 | P95 ≤ `:8012` × 1.15 |
| 4 | 外部流量 | NewAPI / 客户端 upstream 改 TNexus gateway |
| 5 | 灰度 7 天 | 无 P0 |

## 本轮已推进（不停 8012）

| 项 | 说明 |
|----|------|
| 养号 `dialogues_nurture` | account-ops worker 写 `USAGE_EVENTS_FILE` |
| 默认 upstream | worker/api `GPTIMAGE_BASE` 默认 `:8014` |
| humanlike ε-greedy | `HUMANLIKE_EPSILON`（默认 0.12） |
| inflight 泄漏 | gateway 每 5min `reconcile_stale_inflight` |
| `helper_ok` | upstream 生图模式下 health 不再误报 false |
| account-ops 卷 | `/data/pool` 挂载供用量事件 |

## 仍缺（到 95%）

1. Panda 执行 `postgres_cutover.sh`（**仍保留 :8012 进程**）
2. humanlike / 背压生产压测与调参
3. Outlook 恢复 UI
4. Sentinel PoW 全自动密码重登（部分账号）
5. Phase 2 灰度切流（`docs/38`）

## 停服顺序（将来）

1. 外部 upstream 切 TNexus  
2. 灰度 7 天  
3. `postgres_cutover.sh`  
4. `docker stop chatgpt2api-local`（仅摘 HTTP，数据已迁）  
5. compose 去掉 gptimage 数据卷  

详见 [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md)、[38-tnexus-production-cutover.md](38-tnexus-production-cutover.md)。
