# TNexus 文档索引

最后更新：**2026-07-30**（gateway-rs 文档同步入仓）

## 合并施工（先看）

| 文档 | 说明 |
|------|------|
| [../plan.md](../plan.md) | **施工总控** — 阶段划分与详细待办 checklist |
| [../HANDOFF.md](../HANDOFF.md) | 当前状态、Panda 拓扑、已知问题 |
| [SOURCE.md](SOURCE.md) | UI/API 对照源（`gptimage` Python 仓） |

## 自 gptimage-gateway-rs 同步

| 文档 | 说明 |
|------|------|
| [00-contract.md](00-contract.md) | 协议契约 / error_class |
| [17-operator-guide.md](17-operator-guide.md) | 运维拓扑与故障树 |
| [18-test-matrix.md](18-test-matrix.md) | 验收矩阵 |
| [21-auth-and-ui.md](21-auth-and-ui.md) | 鉴权、Web UI、环境变量 |
| [22-audit-2026-07-26.md](22-audit-2026-07-26.md) | 全量审计 |
| [23-rewrite-progress.md](23-rewrite-progress.md) | Rust 重写进度 |
| [24-gap-inventory.md](24-gap-inventory.md) | **能力 gap**（号池 26 端点等） |
| [25-panda-vs-rust-20260726.md](25-panda-vs-rust-20260726.md) | 生产现采对照 |
| [26-perf-measured-20260726.md](26-perf-measured-20260726.md) | 性能实测 |
| [27-tls-fingerprint-spike-20260726.md](27-tls-fingerprint-spike-20260726.md) | TLS 指纹 |
| [28-decisions-20260727.md](28-decisions-20260727.md) | 架构决策 |
| [29-cf-pass-rate-ab-20260727.md](29-cf-pass-rate-ab-20260727.md) | CF 通过率 AB |
| [30-phase1-probe-panda.md](30-phase1-probe-panda.md) | Panda 探针 |
| [32-independent-deploy.md](32-independent-deploy.md) | 独立部署 |
| [33-panda-deploy-20260728.md](33-panda-deploy-20260728.md) | Panda TNexus+gateway 部署记录 |

## TNexus 专属

| 文档 | 说明 |
|------|------|
| [R2.md](R2.md) | R2 存储配置 |
| [../TNexus.md](../TNexus.md) | 产品愿景 |

## 路径约定（合并后）

- 原 `gptimage-gateway-rs/crates/*` → `TNexus/crates/gateway*`（见 plan.md §2.1）
- 原 `gptimage-gateway-rs/docs/*` → 本目录
- UI 对照源 **不在本仓**：`AutoRegister/gptimage/web`（见 SOURCE.md）
