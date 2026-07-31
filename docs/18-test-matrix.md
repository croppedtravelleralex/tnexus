# 18 — 测试矩阵与严格验收

最后更新：2026-07-22  
预估值标注「预估」；实测后改「实测」并附 runlogs 路径。

## 0. 晋级规则

| 从 | 到 | 条件 |
|----|----|------|
| 文档/L0 | MVP 实打 | STORE S1/L1（磁盘工程）按 gptimage `15`；契约冻结 |
| MVP 接线 | Panda 签字 | Phase A 已接线（Rust `:8013`）；**健康 egress 窗**可测 |
| Panda MVP | 全量开发 | **MVP 节签字**：`self=0` + 生图成功样张 |
| 全量测试 | R2 生产立项 | 全量节签字 + 观察窗 |

未达标：停层修复；禁止放宽超时/删测例过门。  
**CF403 / 出口不可用**：记 upstream，**不阻断** Phase A「接线完成」认定；但**阻断** MVP 正式出门签字（须可测窗补跑）。

---

## 1. MVP 矩阵（Panda 隔离）

样本建议：生文 n≥5，生图 n≥5；preferred 异号异代理；模型按现网（生图例 `gpt-image-2`）。  
脚本：`scripts/mvp_rust_conc_matrix.py`（conc=1 串行 + conc=3 并行）。

| ID | 用例 | 期望 outcome | 期望 class | 预估延迟量级 | 严格 |
|----|------|--------------|------------|--------------|------|
| M-T1 | 文本非流式短回复 | 200 + 非空 content | ok | P50 ~3–15s | self=0 |
| M-T2 | 文本 SSE | 事件完整结束 | ok | 同左 | self=0 |
| M-T3 | Temporary Chat 默认 | body 含 history disabled | ok | n/a | 形状差分 0 |
| M-T4 | CF HTML bootstrap | soft-fail 后仍可继续或明确 upstream | upstream/ok | — | 禁止当 self 空成功 |
| M-I1 | generations 成功 | 200 + b64_len>1000 | ok | **健康窗 ~40–60s** | self=0 |
| M-I2 | SSE ready=cid / file_id 早退 | 无 post_ready=15s；有 complete_predicate | ok | — | 硬 |
| M-I3 | skipped_mainline | 继续 poll 出图或明确 upstream | ok/upstream | — | 禁止换号 |
| M-I4 | estuary 带 Bearer | 下载成功 | ok | — | 硬 |
| M-I5 | estuary 无 Bearer（负例） | **失败** | self 探测用 | — | 负例必须失败 |
| M-I6 | edits（单图） | 200 + b64 | ok | 同生图 | 与 generations 同级 |
| M-I7 | Arkose required（若触发） | 显式错误 | upstream/client | — | 不伪装成功 |
| M-C1 | conc=1 串行 | 单张 ~40–60s（健康窗） | ok | — | 剔 upstream |
| M-C3 | conc=3 并行 | 单张同量级；墙钟≈单张窗 | ok | — | 异号异出口 |
| M-S1 | 脱敏 | runlogs 无 Bearer/eyJ | ok | — | grep 通过 |

### MVP 预估结果矩阵（填表模板）

| 指标 | 预估（相对 Python 生产基线） | 实测（签字栏） |
|------|------------------------------|----------------|
| text 成功/总数 | ≥ Python − 0（剔 upstream） | ⏳ CF 窗后补 |
| image 成功/总数 | ≥ Python − 0（剔 upstream/gate） | ⏳；接线期 CF 窗 0 成功 / **self=0** |
| self 计数 | **0** | **0**（2026-07-21 矩阵） |
| 假成功空 data | **0** | **0** |
| text P50 | ≤ Python × 1.10 | |
| image P50 / P95 | 健康窗 ~40–60s；≤ ×1.15 / ×1.20 | ⏳ |
| runtime | `rust` | ✅ `:8013` |
| runlog 路径 | | `data/runlogs/rust-conc-matrix-out.txt` 等 |

**Phase A 接线**：2026-07-21 ✅（`runtime=rust` + helper `:19001`）  
**MVP 出门签字**：日期 ____ 证据路径 ____ `self=0` □（待 egress 可测）

---

## 2. 全量矩阵（含 RCA；MVP 通过后）

| ID | 用例 | 期望 | 严格 |
|----|------|------|------|
| F-S1 | 选号 + 释槽 | inflight 回零 | 泄漏=0 |
| F-S2 | 迟到 account_acquired | 仍释放 | 回归 |
| F-S3 | hard-timeout + cid | timeout_pending 语义 | 对齐 Python |
| F-S4 | admission 满 | 429 `image_service_busy` | 禁假成功 |
| F-S5 | pause | 503 同码 | |
| F-B1 | schedulable-breakdown | 空池主因桶 | |
| F-R1 | RCA agent 只读多步 | 叙述与 breakdown 一致 | 无 mutate 越权 |
| F-R2 | llm_ops 字段 | source/outcome/error_class 可对账 | |
| F-R3 | humanlike-dashboard KPI | 与 Rust 计数误差约定内 | |
| F-N1 | nurture | 不破坏业务 Temporary Chat 默认 | 禁假聊 |
| F-X1 | 注册/FlareSolverr | **无此用例** | 仓库不得出现 |

### 全量预估结果

| 指标 | 预估 | 实测 |
|------|------|------|
| inflight 泄漏 | 0 | |
| RCA 只读路径成功率 | ≥99%（工具层） | |
| 看板对账 | 关键计数一致 | |

**全量出门签字**：日期 ____ 证据 ____

---

## 3. 不做的用例

- 注册机 E2E、FlareSolverr clearance 刷新、`/api/register`
- 生产公网切流（属 R2）
