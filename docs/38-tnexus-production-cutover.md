# 38 — TNexus 1:1 替代 gptimage 生产路线图

最后更新：**2026-08-01**

## 目标

TNexus（gateway `:8014` + worker + api `:9000`）**独立承担全部生图/对话生产**，可在运维确认后**切换流量、下线对 Panda `chatgpt2api-local :8012` 的运行时依赖**。

> 红线不变：切换前不得影响 `:8012`；只走 Git → GHCR → Panda `deploy.sh`。

---

## 当前阻塞项（本轮已修 / 待部署）

| # | 问题 | 根因 | 状态 |
|---|------|------|------|
| 2 | 工作台比例/风格无效 | Gateway `run_upstream_image` 未传 `size`；upstream 仅靠 prompt 暗示尺寸；风格预设无选中态/无文案注入 | **代码已修**（见下） |
| 3 | 额度 badge 灰色 | API 未填 `image_quota_state`；前端未按「调度中+正常」着色 | **代码已修** |
| 4 | IP 热力图空白 | Worker 写 `binding:"default"` 覆盖 email 解析；API binding key 与前端 `egress:` 前缀不一致 | **代码已修** |
| 5 | 日志总时长 ≠ 分阶段之和 | 内联阶段缺 `ps_ms`（构思）等；无「其他」补差 | **代码已修** |
| 6 | 对话未独立 | UI 直连 `NEXT_PUBLIC_GATEWAY_BASE`；无 tnexus-api 代理/调用日志/多轮持久化 | **规划中**（P1） |

### 本轮代码修复摘要

- `tnexus_domain::append_image_generation_hints` — 对齐 Python `build_image_prompt` 的尺寸/质量/透明背景 hint
- Worker 生图前注入 hint；Gateway upstream 路径传入 `req.size`
- `usage_metrics::binding_key_for_account_fields` — 对齐 gptimage `binding_key_for_account`
- Worker `record_usage_event` 传空 binding，由 API 按 email 解析
- `image-quota.ts` — 调度中且正常 → 绿色 badge
- `image-log-phases.ts` — 展示 `ps_ms` + 「其他」补差

---

## 分阶段切流计划

### Phase 0 — 观测与对齐（当前，~1 周）

- [x] Pipeline 埋点（`phase_timings_ms`、quota、bandwidth）
- [x] 工作台参数链路修复（size/style）
- [x] 号池 UI 对齐（额度色、热力图 binding）
- [ ] **部署上述修复到 Panda** 并复测 10 并发
- [ ] 对比 `:8012` vs `:8014` 同 prompt 同 size 出图尺寸（像素级）
- [ ] 确认 `USAGE_EVENTS_FILE` 容器挂载与 worker/api 同路径

**验收**：热力图有数据；额度调度中为绿；日志阶段之和 ≈ 总时长（±0.5s）

### Phase 1 — 数据面能力补齐（~2–3 周）

对照 [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) 与 [24-gap-inventory.md](24-gap-inventory.md)：

| 能力 | 生产 `:8012` | TNexus | 优先级 |
|------|-------------|--------|--------|
| 文生图 SSE 主链 | ✅ | ✅ upstream | — |
| size/quality/透明背景 | prompt hint + API | **已补 hint** | P0 |
| 图生图 / edits | ✅ | ❌ | P1 |
| 对话生图（chat 内出图） | ✅ | ❌ | P1 |
| humanlike 调度/背压 | ✅ ~8k 行 | scheduling_gate 子集 | P1 |
| dispatch_gate / 并发槽泄漏修复 | ✅ | 部分 | P1 |
| 异步 url 任务语义 | ✅ image_task_service | Job 队列 | P2 |
| OpenAI 兼容 usage/tokens | ✅ | 常 0 | P2 |

**任务清单**：

1. Gateway `POST /v1/images/edits` 接线 upstream
2. Chat 对话页改走 `tnexus-api` 代理 → gateway，写入调用日志（`dialogues_real`）
3. 养号/拟人对话写 `dialogues_nurture` 到 `usage_events`
4. `dispatch_gate` 或等价：inflight 泄漏检测 + 账号自动隔离
5. 背压：全局/ per-binding 并发与 `:8012` 对齐配置

### Phase 2 — 灰度切流（~1 周）

1. NewAPI / 外部调用方增加 **第二 endpoint** `https://tnexus.relai.asia/gateway`（或独立子域）
2. 10% 流量 → TNexus，对比成功率、P50/P95、CF 率、额度消耗
3. Studio 生产默认走 TNexus worker（已如此）；禁止回退 `:8012` 代理

**验收**：7 天无 P0；P95 生图耗时 ≤ `:8012` × 1.15

### Phase 3 — 生产主路径切换（~3 天）

1. 将 NewAPI primary upstream 从 `:8012` 改为 TNexus gateway
2. `:8012` 保留 **只读/回滚**（不删容器，仅摘流量）
3. 监控：Postgres job 失败率、gateway 429、磁盘、Postgres 连接

**回滚**：DNS/NewAPI 指回 `:8012`（无需重新部署 gptimage）

### Phase 4 — 下线 Python 生图运行时（可选，~2 周）

- account-ops 仍依赖 `GPTIMAGE_ROOT` Python **库**（刷新/养号）— **不删**
- 仅停 `chatgpt2api-local` 生图 HTTP 服务
- 文档更新 HANDOFF / 部署脚本

---

## 对话功能（#6）独立实现路线

**现状**：`web/src/app/(console)/chat/page.tsx` → `ChatWorkbench` → 浏览器直连 `NEXT_PUBLIC_GATEWAY_BASE/v1/chat/completions`。

**缺口**：

| 项 | gptimage | TNexus |
|----|----------|--------|
| 同源 API 代理 | ✅ 经后端 | ❌ 需配 CORS + 暴露 Gateway Key |
| 调用日志 / 热力图 `dialogues_real` | ✅ | ❌ |
| 多轮 + 会话持久化 | ✅ | ❌ |
| 对话生图 | ✅ | ❌ |

**P1 实现顺序**：

1. `tnexus-api` 增加 `POST /api/chat/completions`（SSE 透传 gateway，`require_auth`）
2. `chatApi` 改调 `/api/chat/completions`（cookie 鉴权）
3. 成功调用写 `usage_events` + system_logs（`dialogues_real`）
4. （P2）对话生图：检测 tool call / 图片 SSE，复用 worker 生图链

---

## 部署检查清单（每次发版）

```bash
# Panda（仅 pull + up，不编译）
cd /opt/tnexus && git pull && ./deploy/panda/deploy.sh

# 验证
curl -s localhost:9000/health
curl -s localhost:8014/health
# 生图 smoke（管理员 JWT）
# 号池：按 IP 分组 → 热力图非空
# 日志：总时长 ≈ 阶段之和
```

环境：

- `USAGE_EVENTS_FILE=/data/pool/usage_events.ndjson`（api + worker 一致）
- `UPSTREAM_API_KEY` gateway JWT 未过期
- `DATA_PLANE=upstream`

---

## 相关文档

- [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) — 能力差距量化
- [37-gptimage-tnexus-comparison.md](37-gptimage-tnexus-comparison.md) — 横向对比
- [36-image-delivery-bandwidth-strategy.md](36-image-delivery-bandwidth-strategy.md) — 带宽与图床
