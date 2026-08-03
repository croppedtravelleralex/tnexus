# 37 — gptimage（Python :8012）与 TNexus 横向对比

最后更新：**2026-08-02**

对照基准：Panda 生产 `chatgpt2api-local :8012` vs `tnexus.relai.asia`（`:9000` API + `:8014` gateway）。

---

## 1. 架构对照

| 维度 | Python gptimage `:8012` | TNexus |
|------|-------------------------|--------|
| 运行时 | Python 3.12 + FastAPI + curl_cffi | Rust（gateway/worker/api）+ Next.js UI |
| 号池 | live `accounts.db` | **同一** `accounts.db`（WAL） |
| 生图入口 | `POST /v1/images/generations` | Studio → `/api/jobs` → worker → `:8014/v1/images/generations` |
| 默认回包 | 异步任务 **url**（~0.7KB）；同步 **b64**（~1.8MB） | URL 模式 + Gateway asset；历史曾用 `inline_preview_b64` |
| 调度 | `humanlike_scheduler` + `image_task_service` + dispatch_gate | `scheduling_gate`（无 humanlike/背压） |
| 运维 | 内置 scripts/API | account-ops `:9011` + `GPTIMAGE_ROOT` |
| 公网 | `gptimage.relai.asia` | `tnexus.relai.asia` |

---

## 2. 能否从 API-b64 生图侧完全替代 Python？

### 2.1 结论：**不能「完全」替代；可替代「Studio 导演台 + 号池管理 + 基础生图」子集**

| 能力 | Python :8012 | TNexus | 可替代？ |
|------|--------------|--------|----------|
| `POST /v1/images/generations` n=1 b64/url | ✅ | ✅ gateway | ✅ 基础生图 |
| 异步队列 + `image_task_id` | ✅ | ❌ 同步 job 队列 | ❌ |
| `response_format=url` 上游 CDN 直链 | ✅ 默认异步 | ⚠️ Gateway 内存 asset URL | ⚠️ 语义不同 |
| edits / inpainting（单图 base64） | ✅ | ✅ upstream `1ab5d25` | ✅ 基础图生图 |
| edits mask / 多图 | ✅ | ❌ | ❌ |
| n>1 同请求多图 | ✅ | ❌ MVP n=1 | ❌ |
| humanlike 调度 / dispatch_gate | ✅ | ❌ | ❌ |
| CF 探活 / 背压 / b64 回传窗口 | ✅ | ❌ | ❌ |
| 129 个管理 HTTP 端点 | ✅ | ~82% API + account-ops | ⚠️ |
| 号池 UI | gptimage web | TNexus web ~94% | ✅ 管理台 |
| Studio 多演员并行 | N/A | ✅ casting 10–40 槽 | ✅ TNexus 独有 |
| OpenAI 兼容 API Key 鉴权 | ✅ | JWT / member | ⚠️ 鉴权模型不同 |

**加权替代进度**：约 **50%**（[35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md)）。

**若「完全替代」定义 = 任意客户端拿 API Key 调 `:8012` 全功能无缝迁移**：**仍不行**（缺 mask/多图 edits、异步 url 语义、humanlike、背压、对话生图）。

**若定义 = TNexus Studio + 号池 + 并行 casting + 基础生图上线**：**已可生产使用**；与 `:8012` **并行**，非替换。

---

## 3. 生图延迟（E2E 墙钟）

> 测试条件不同不可直接比绝对值；看数量级与瓶颈占比。

| 场景 | Python :8012 | TNexus | 备注 |
|------|--------------|--------|------|
| 单张同步 b64 | 29–46s | ~30–62s/槽（gateway_wall p50） | 瓶颈均在 **上游 SSE ~85–95%** |
| serial10 | p95 ~118s（含排队） | — | Python pipeline 日志 |
| conc10 | **10/10**；P50 ~44s；wall ~135s | **10/10**；p50 **47.5s**；wall **239s**（2026-08-01） | TNexus 为 Studio casting |
| conc20 | **20/20**（验收） | wall **~93s**（早期）；待复测 | Python `06-handoff` |
| 回包大小 | b64 **1.8MB** / url **0.7KB** | url JSON **~163B** + thumb | TNexus 展示若 302 PNG 则仍大 |

---

## 4. 资源占用（Panda 实测快照 2026-08-01 空闲）

| 容器/进程 | CPU % | 内存 | 说明 |
|-----------|-------|------|------|
| `chatgpt2api-local` | ~2.1% | **643 MiB** / 1.5GiB limit | 10 submit worker；历史 idle 采样曾报 GIL 高占用 |
| `panda-gateway-1` | ~0% | **45 MiB** | Rust gateway |
| `panda-worker-1` | ~0% | **7 MiB** | 空闲 |
| `panda-api-1` | ~0% | **83 MiB** | API + 静态 UI |
| `panda-postgres-1` | ~0% | 45 MiB | TNexus DB |
| **TNexus 生图栈合计** | ~0% | **~135 MiB** | 不含 Postgres/Redis |

**并发生图时（经验 / 文档）**：

| | Python | TNexus |
|--|--------|--------|
| CPU | 压测峰 ~6–30%；deadlock_guard @90% | Gateway/Worker 主要为 IO 等待；CPU 低于 Python |
| 内存 | 单容器 ~250–640 MiB；b64 并发 inflate | Gateway asset 内存存 PNG；10 并发 ~11MB+ 图缓存 |
| 带宽 | b64 同步时 **Panda 上行** 瓶颈（30Mbps） | ChatGPT 拉图 + 用户看图（若未 WebP/R2） |

---

## 5. 前端 UI 性能

| 维度 | gptimage web (`:8012` / 静态) | TNexus web (`:9000`) |
|------|-------------------------------|----------------------|
| 框架 | Next.js 16 + shadcn/Radix | Next.js 16 + framer-motion |
| 生图进度 | 依赖实现 | `GET /api/jobs/{id}/status` **轻量轮询**（无 b64） |
| 结果预览 | 常内联 `data:image/png;base64` | `/api/images/thumb/{id}`（应用 WebP 后更小） |
| 历史列表 | 可能内联大 payload | thumb API；job 列表不嵌 MB 级字段 |
| 点击/查询 | 多页面后缓存命中变快（`25-frontend-performance-plan`） | 静态 UI + API 分离；`phase_timings_ms` 可观测 |

**TNexus Studio 优化（2026-07-31）**：轮询不再每 2s 拉全量 job+b64；左侧列表走 thumb；计时器 500ms tick。

---

## 6. 选型建议

| 需求 | 选 |
|------|-----|
| 外部 OpenAI 兼容 API + API Key + 异步 url + edits（含 mask/多图） | **Python :8012** |
| 外部 API 基础文生图 + 单图 base64 edits | **TNexus gateway `:8014`** |
| 导演台 / casting 并行 / 号池 UI / 运维 account-ops | **TNexus** |
| 省 Panda 对用户出口带宽 | TNexus + **WebP/AVIF thumb** + **R2**（[36](36-image-delivery-bandwidth-strategy.md)） |
| 彻底下线 :8012 | **未就绪**（~50%；缺调度/背压/对话生图/mask edits） |

---

## 7. 复测命令

```bash
# TNexus Studio 并行
python scripts/test_parallel_casting.py 1 10 20

# Python pipeline（Panda 上 gptimage 目录）
python scripts/_tmp_run_conc10_phases.py

# 资源快照
docker stats --no-stream
```
