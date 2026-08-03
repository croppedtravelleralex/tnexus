# 38 — TNexus 1:1 替代 gptimage 生产路线图

最后更新：**2026-08-02**（Panda 已部署 `1ab5d25`；Phase 0 冒烟通过；edits 已接线）

## 目标

TNexus（gateway `:8014` + worker + api `:9000`）**独立承担全部生图/对话生产**，可在运维确认后**切换流量、下线对 Panda `chatgpt2api-local :8012` 的运行时依赖**。

> 红线不变：切换前不得影响 `:8012`；只走 Git → GHCR → Panda `deploy.sh`。

---

## 当前阻塞项（2026-08-01 状态）

| # | 问题 | 根因 | 状态 |
|---|------|------|------|
| 2 | 工作台比例/风格无效 | Gateway 未传 `size`；风格无 hint 注入 | ✅ **已部署** `b8d6fa8` |
| 3 | 额度 badge 灰色 | API 缺 `image_quota_state` | ✅ **已部署** |
| 4 | IP 热力图空白 | binding key / worker 写 `default` | ✅ **已部署** |
| 5 | 日志总时长 ≠ 分阶段之和 | 缺 `ps_ms`、无「其他」补差 | ✅ **已部署** |
| 6 | 对话未独立 | UI 直连 gateway；无 API 代理 | ✅ **P1 主链已部署**（见下）；多轮/对话生图仍缺 |

### 本轮代码修复摘要（`b8d6fa8` … `1ab5d25`）

**Studio / 对话（`b8d6fa8` + `deefbaa` + `9e8105b`）**

- `tnexus_domain::append_image_generation_hints` — 对齐 Python `build_image_prompt`
- Worker 生图前注入 hint；Gateway upstream 传 `req.size`
- `usage_metrics::binding_key_for_account_fields` — 对齐 gptimage `binding_key_for_account`
- Worker `record_usage_event` 空 binding → API 按 email 解析
- `image-quota.ts` — 调度中且正常 → 绿色 badge
- `image-log-phases.ts` — `ps_ms` +「其他」补差
- `tnexus-api` `POST /api/chat/completions` — SSE 透传 gateway + `dialogues_real` 用量
- `refresh_upstream_jwt.sh` — 同步写入 **`GATEWAY_AUTH_KEY`**（对话代理 Bearer）
- `patch_env.sh` — 默认 `GATEWAY_BASE=http://127.0.0.1:8014`
- `accounts_store.rs` — `Self::is_unlimited_type` 编译修复（`deefbaa`）

**出图元数据 / 号池（`0bc7463` … `9248d34`）**

- migration `008`：`job_results.width/height/size_bytes`
- Worker 存图时解析尺寸；无 R2 时从 `source_url` 下载后落库
- Studio「出图效果」角标显示分辨率与大小
- 号池无选中账号时工具栏「同步全部额度」→ `refresh-all`

**图生图 edits（`1ab5d25`）**

- `upstream/upload.rs` — 对齐 Python `_upload_image_once`
- `runtime::run_image_edit_with_metrics` — 上传参考图 + multimodal start
- Gateway `POST /v1/images/edits` — 需 `IMAGE_ENABLED=1` + `DATA_PLANE=upstream`
- `capabilities.image_edits: true`（同上条件）

---

## 分阶段切流计划

### Phase 0 — 观测与对齐（当前）

- [x] Pipeline 埋点（`phase_timings_ms`、quota、bandwidth）
- [x] 工作台参数链路修复（size/style）
- [x] 号池 UI 对齐（额度色、热力图 binding）
- [x] **部署到 Panda**（`1ab5d25`，2026-08-02）
- [x] 生产冒烟：热力图有数据；额度调度中绿；日志阶段之和 ≈ 总时长；对话 SSE
- [ ] 对比 `:8012` vs `:8014` 同 prompt 同 size 出图尺寸（像素级）
- [ ] 10 并发压测

**验收脚本**（在 Panda 上执行）：

```bash
python3 /root/TNexus/scripts/prod_url_chain_test.py      # 生图 E2E
python3 /root/TNexus/scripts/test_ux_coverage.py       # Studio UX 七项
python3 /root/TNexus/scripts/test_studio_modes.py        # 导演 vs 竞演
```

**2026-08-01 实测摘要**：

| 检查项 | 结果 |
|--------|------|
| 16:9(4k) `gen_config` | `3840×2160`；缩略图约 `1672×941` |
| 1:1 | `1024×1024`；缩略图约 `1254×1254` |
| 竞演 + 极端 `ps_factors` | 双 provider 完成，preview >8KB |
| 排队黄字（30s 内开工） | `queue_start_s≈2s`，无「任务仍在排队…」 |
| 调度账号绿 badge | 40/40 `success`（emerald） |
| 对话流式 | `text/event-stream`；UI 流式出字 |
| 日志阶段 | `wall_clock_ms=95431`，阶段和比 ≈1.01 |

### Phase 1 — 数据面能力补齐（~2–3 周）

对照 [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) 与 [24-gap-inventory.md](24-gap-inventory.md)：

| 能力 | 生产 `:8012` | TNexus | 优先级 |
|------|-------------|--------|--------|
| 文生图 SSE 主链 | ✅ | ✅ upstream | — |
| size/quality/透明背景 | prompt hint + API | ✅ hint 已上线 | — |
| 对话经 tnexus-api 代理 | ✅ | ✅ **已上线** | — |
| 图生图 / edits | ✅ | ✅ **upstream 已上线** `1ab5d25` | — |
| 对话生图（chat 内出图） | ✅ | ❌ | P1 |
| humanlike 调度/背压 | ✅ ~8k 行 | scheduling_gate 子集 | P1 |
| dispatch_gate / 并发槽泄漏修复 | ✅ | 部分 | P1 |
| 异步 url 任务语义 | ✅ image_task_service | Job 队列 | P2 |
| OpenAI 兼容 usage/tokens | ✅ | 常 0 | P2 |

**任务清单**：

1. ~~Gateway `POST /v1/images/edits` 接线 upstream~~ ✅ `1ab5d25`（单图 base64；mask / 多图待补）
2. ~~Chat 对话页改走 `tnexus-api` 代理~~ ✅ `b8d6fa8`
3. 养号/拟人对话写 `dialogues_nurture` 到 `usage_events`
4. 对话多轮 + 会话持久化（对齐 gptimage conversations）
5. `dispatch_gate` 或等价：inflight 泄漏检测 + 账号自动隔离
6. 背压：全局/ per-binding 并发与 `:8012` 对齐配置

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

## 对话功能（#6）

**现状（2026-08-01）**：`ChatWorkbench` → `POST /api/chat/completions`（cookie 鉴权）→ gateway `/v1/chat/completions`（`GATEWAY_AUTH_KEY` Bearer）。

| 项 | gptimage | TNexus |
|----|----------|--------|
| 同源 API 代理 | ✅ | ✅ `crates/tnexus-api/src/routes/chat.rs` |
| 调用用量 `dialogues_real` | ✅ | ✅ `usage_metrics::record_event` |
| 流式 SSE | ✅ | ✅ 已验 |
| 多轮 + 会话持久化 | ✅ | ❌ Phase 1 |
| 对话生图 | ✅ | ❌ Phase 1 |

**注意**：若仅配置 `UPSTREAM_API_KEY` 而未同步 `GATEWAY_AUTH_KEY`，对话代理会返回 gateway 的 `{"error":"login required","ok":false}`（401）。`deploy.sh` 自 `9e8105b` 起自动同步。

---

## 部署检查清单（每次发版）

```bash
# Panda（仅 pull + up，不编译）
export TNEXUS_ROOT=/root/TNexus
cd "$TNEXUS_ROOT" && git pull && bash deploy/panda/deploy.sh

# 健康
curl -fsS http://127.0.0.1:9000/health
curl -fsS http://127.0.0.1:8014/health

# 冒烟（推荐在 Panda 本机）
python3 scripts/prod_url_chain_test.py
python3 scripts/test_ux_coverage.py
```

环境（`/opt/tnexus/.env`）：

- `USAGE_EVENTS_FILE=/data/pool/usage_events.ndjson`（api + worker 一致）
- `UPSTREAM_API_KEY` — worker → gateway JWT（≈24h TTL）
- **`GATEWAY_AUTH_KEY`** — api 对话代理 → gateway（与上同步刷新）
- **`GATEWAY_BASE=http://127.0.0.1:8014`**
- `DATA_PLANE=upstream`

---

## 相关文档

- [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) — 能力差距量化
- [37-gptimage-tnexus-comparison.md](37-gptimage-tnexus-comparison.md) — 横向对比
- [36-image-delivery-bandwidth-strategy.md](36-image-delivery-bandwidth-strategy.md) — 带宽与图床
- [HANDOFF.md](../HANDOFF.md) — 现网拓扑与已知问题
