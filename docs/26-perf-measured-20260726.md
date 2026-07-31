# 26 — 性能实测与重写收益预估（2026-07-26）

采集方式：panda 只读 —— `/proc/<pid>/{status,stat,smaps_rollup,task/*/stat}`、cgroup `cpu.stat`/`memory.current`、
`docker inspect|stats`、`logs.jsonl` 统计、runlog 读取。
**未编译、未写文件、未重启服务、未发送压测或生图流量。**

本文取代 [13-perf-baseline-compare.md](13-perf-baseline-compare.md) 中的全部预估数字 ——
那份文档的 4 行 Rust 收益是拍脑袋（表头自标「预估」，MVP/全量两列空白），
且其 Python CPU 基线偏了**两个数量级**。

---

## 0. 四类数据的严格区分

本文所有数字标注来源类别，不混用：

| 类别 | 含义 |
|------|------|
| **【实测】** | 本次在 panda 上现采，可复现（见 §6） |
| **【文档声称】** | 旧文档写的，本次已证伪或无法证实 |
| **【公开基准】** | 第三方 benchmark，附出处 |
| **【推断】** | 从实测外推，明确标注未验证部分 |

---

## 1. 实测基线

### 1.1 进程级

| 指标 | Python `chatgpt2api-local` | Rust `:8013` | 采集 |
|------|---------------------------|-------------|------|
| RSS | **229,240 KB（224 MB）** | 5,296 KB（**私有仅 432 KB**） | `/proc/<pid>/status` VmRSS |
| Pss（去共享） | 228,233 KB | 3,053 KB | `smaps_rollup` |
| Private_Dirty | 222,180 KB | **368 KB** | `smaps_rollup` |
| VmSize | 1,368,060 KB | 144,528 KB | `/proc/<pid>/status` |
| VmPeak | **1,580,564 KB** | 209,920 KB | 同上 |
| Swap | 0 KB | 2,528 KB（被换出，说明长期空闲） | `smaps_rollup` |
| OS 线程 | **30** | **3** | `ls /proc/<pid>/task \| wc -l` |
| 实时 CPU（10s 采样） | **98.5%** | ~0% | `task/*/stat` utime+stime 差分 |
| 生命周期累计 CPU | 17,741s user + 368s sys | **0.19s user + 0.14s sys** | `/proc/<pid>/stat` 字段 14/15 |
| 运行时长 | 30,853s（8h35m） | 258,074s（**2d23h**） | `ps -o etimes` |
| 平均 CPU 占用 | **58.6%** | **0.00013%** | 累计 ÷ 时长 |
| 自愿上下文切换 | **709,436** | **27** | `/proc/<pid>/status` |
| 非自愿上下文切换 | 22,522 | 25 | 同上 |

### 1.2 容器 / 部署级

| 指标 | 值 | 采集 |
|------|-----|------|
| 容器镜像 `chatgpt2api:local` | **960 MB** | `docker images` |
| 容器内 `.venv` | **105 MB** / 82 个依赖包 | `du -sh /app/.venv` |
| Rust 部署产物 | 单 ELF **7.2 MB**（未 strip） | `ls -la bin/` |
| cgroup CPU 累计 | 18,112s（user 17,744 / sys 369） | `cpu.stat` |
| cgroup 内存 | 295,514,112 B / 上限 1,610,612,736 B | `memory.current` / `memory.max` |
| **CPU 节流** | **仅 4 次 / 309,734 周期** | `cpu.stat nr_throttled` |

节流几乎为 0 说明：**Python 不是被 cgroup 限死的，是自己吃满了一个核。**

### 1.3 关键校正

Rust 进程 3 天累计只用了 **0.33 秒** CPU，且 RSS 中 4,864KB 是 mmap 的二进制（RssFile），
真实私有内存只有 **432 KB**。

> **它的「低资源」不是重写效率的证明，只是空闲进程的证明。**
> 内存比 224MB : 5.2MB ≈ 43:1 **不可作为重写收益引用** ——
> Rust 只跑 7 个端点、单账号、无后台循环，且真正干活的 Python helper 另占 11.8MB。

---

## 2. Python 98.5% CPU 的归因

10 秒实时采样，29 个线程中只有 12 个活跃：

| 线程 | CPU% |
|------|------|
| tid 15–24（**共 10 个**） | 9.0 / 9.1 / 9.1 / 9.2 / 9.3 / 9.8 / 9.8 / 10.3 / 10.8 / 11.5 |
| tid 32 | 0.5 |
| tid 10（主事件循环） | 0.1 |
| **合计** | **98.5%** = 恰好单核跑满 |

10 个线程 CPU 几乎完全相等（9.0–11.5%），全部处于 `State: S` / `wchan: futex_wait_queue`
—— 这是 **GIL 争抢的教科书特征**：10 个线程排队轮流持锁，加起来正好吃满一个核，单线程拿不到超过 1/10。

来源已精确定位：

| 环节 | 位置 |
|------|------|
| 配置 | `/app/config.json`：`image_task_queue.submit_workers = 10`（`submit_workers_max` 也是 10） |
| 线程创建 | `services/image_task_service.py:1059` `_ensure_workers_locked()` |
| 循环体 | `services/image_task_service.py:1199` `_submit_worker_loop` |
| 启动时机 | 线程 starttime 102184615–102184616 = **进程启动瞬间全部创建**，非按需扩容 |

`_submit_worker_loop` **不是**忙轮询 —— 空闲走 `self._condition.wait(timeout=max(0.05, wait_secs))`。
但 `wait_secs` 默认 0.5s、**最小可低至 0.05s**，且每次唤醒都在持锁状态下执行
`_next_submit_task_locked()` + `_warm_account_lease_pool_locked()`。

**10 线程 × 每秒 2–20 次唤醒 × 每次带锁扫描全部 task dict，在 2 vCPU 上叠加成 98.5%。**

> 【推断】此归因为强推断：线程数精确匹配 config 的 10、CPU 精确均分、futex_wait 状态、启动时刻一致。
> **未用 py-spy 抓栈验证** —— 容器内无 py-spy，安装会违反「禁止在 panda 上编译/安装」。

---

## 3. 架构约束（决定天花板）

| 约束 | 数值 | 依据 | 影响 |
|------|------|------|------|
| 物理核 | **2 vCPU** Xeon 8255C @2.5GHz | `nproc` | Python 已吃满 1 核 = 半台机器 |
| Python GIL | 1 个解释器锁 | Python 3.13.14，非 free-threading | **30 线程只能用 1 核**，加线程不提速 |
| uvicorn worker | **1**（无 `--workers`） | `/proc/1/cmdline` | 无多进程，无法绕过 GIL |
| anyio 线程池 | **40**（`total_tokens`） | 容器内实测，anyio 4.13.0 | **「40 的说法成立」**；41 并发即排队 |
| helper 端点同步性 | **8 个路由全部 sync `def`** | `protocol_bridge.py:591,596,605,610,615` + `openai_face.py:165,182,193,242,276,299,366` | 每请求 → `run_in_threadpool` → 占 1 个 GIL 线程 |
| 同账号互斥锁 | `threading.Lock` per email，超时 90s | `protocol_bridge.py:62` | **同账号生图完全串行**，与语言无关 |
| helper 生图 wall | 120s | `protocol_bridge.py:368` | 单请求最长阻塞 120s |
| helper poll timeout | 90s | `protocol_bridge.py:366` | — |
| Rust helper client 超时 | 文本 120s / 生图 **180s** | `helper_client/src/lib.rs:111,205` | Rust 全程 await，不占 CPU |
| Rust 生图信号量 | **3** | `main.rs:423` | **比上述三者都松 → 不是瓶颈，改它无效** |
| Python 生图全局并发 | 10 | `config.json` | — |
| 容器内存上限 | 1.5 GiB，当前用 295 MB | cgroup | **内存不是当前瓶颈，CPU 才是** |
| 宿主内存 | 3,723 MB 总 / 2,139 用 / **swap 已用 954 MB** | `free -m` | 内存紧张真实存在，但 Python 自身 Swap=0 |

---

## 4. 请求耗时实测

### 4.1 Python 生产（`/root/gptimage/data/logs.jsonl`，7,678 行）

| 类型 | 样本 | min | **p50** | p95 | max |
|------|------|-----|---------|-----|-----|
| L0 chat 成功 | **2,735** | 2,931ms | **6,323ms** | 12,049ms | 58,530ms |
| L0 chat 失败 | 1,242 | 0ms | 1,530ms | 3,280ms | 124,018ms |
| L0 nurture 成功 | 815 | 3,913ms | 37,216ms | 50,471ms | 159,598ms |
| risk_audit summarize | 361 | 3,457ms | 7,615ms | 13,210ms | 25,592ms |
| risk_audit ops_rca | 292 | 2,764ms | 10,025ms | 10,192ms | 85,214ms |

**文本成功率 = 2,735 / 3,977 = 68.8%**

### 4.2 阶段级（`request_phase` 事件，实例 `3bde17ac`）

| 阶段 | 累计 elapsed_ms | 本阶段 |
|------|----------------|--------|
| preflight | 0 | 0 |
| auth | 550 | 550 |
| **request_build** | 2,190 | **1,639**（turnstile 2,632B + PoW 607B 求解） |
| upstream_submit | 2,404 | 214 |
| sse_ready | 5,109 | 2,705 |
| cleanup | 7,765 | 2,655 |

**7.77s 中纯本地计算约 1.6s，其余 6.2s 是网络等待。**

### 4.3 Rust MVP runlog

| 文件 | 结果 |
|------|------|
| `rust-mvp-matrix-20260720-194539.json` | 文本 **5/5** ok，6.17–12.72s；生图 2/5 ok，57.2s / 84.2s；3 次 poll timeout |
| `rust-ticket-verify-20260723/summary.json` | **唯一干净成功：29.62s**，b64=1,171,556 |
| `rust-mvp-multi-conc2.json` | **0/4**，全 CF403 或 wall timeout |
| `rust-conc-matrix-out.txt` | conc=1 **0/3**；conc=3 **0/3**；全上游 403/timeout |

**Rust 文本 6.17–12.72s vs Python p50 6.32s / p95 12.05s —— 同一分布内，无可测差异。**

本地 `data/runlogs/` 只有 `README.md` + 一个 **0 字节**的 `rust-conc-matrix-localcap.txt`。

---

## 5. Rust 侧当前实际做了什么

`crates/gateway/src/main.rs` 677 行（含 116 行测试）。**出站 HTTP 目标只有 6 个，全部指向本地 helper**：

| 目标 | 调用点 |
|------|--------|
| `{helper}/health` | `helper_client/src/lib.rs:121` |
| `{helper}/v1/internal/quota/refresh` | `:132` |
| `{helper}/v1/internal/accounts/candidates` | `:155` |
| `{helper}/v1/internal/text` | `:185` |
| `{helper}/v1/internal/text/stream` | `:190` |
| `{helper}/v1/internal/image` | `:203` |

**零个指向 ChatGPT。** 所有上游调用仍由 Python 的 `curl_cffi` 发出。

| 工作 | 有无 | 说明 |
|------|------|------|
| 入站 JSON 解析 | 有 | 请求体 350–1,044 B |
| 出站 JSON 序列化 | 有 | `protocol/src/lib.rs:108,124` |
| **SSE 解析** | **无** | `main.rs:342` 只 `bytes_stream().map(...)` —— **纯字节透传，不解析事件** |
| **PoW / turnstile / 加密** | **无** | 全在 Python（生产日志 `turnstile_solved_len:2592` / `proof_solved_len:647`） |
| **b64 图片处理** | 几乎无 | 1.17MB 字符串只做一次长度检查（`main.rs:492`）后原样塞进 JSON |
| 认证 | 有 | argon2 + JWT，仅登录时 |
| 并发控制 | 有 | `Semaphore(3)` + `Mutex<HashMap>` |

**结论：Rust 网关是纯 HTTP 反向代理 + 认证层 + 信号量。每请求 CPU 工作 <1ms
—— 与实测「3 天累计 0.33 秒」完全一致。**

---

## 6. 公开基准（第三方）

| 来源 | 指标 | Python | Rust | 倍数 |
|------|------|--------|------|------|
| TechEmpower R23 社区分析（Fortunes） | RPS | Django 32,651 | Actix 320,144 | **~9.8×** |
| Markaicode 2025 框架对比 | axum 饱和吞吐 / 空闲 RAM | — | axum 0.8 ~18,000 rps，空闲 60–90 MB | — |
| FastAPI 官方 benchmark 页 | 定位 | 仅次于 Starlette | — | 官方自述：基准不测特性 |
| luke.hsiao.dev axum vs FastAPI | 带 Postgres 真实对比 | 基线 | — | **仅快 ~6%**，作者判「可忽略」 |
| Muller Digital 容器实测 | 内存 / CPU | 基线 | −90% / −75% | 10× / 4×（单一计算密集负载，轶事级） |
| jonvet.com | 单线程耗时 | 100% | 40% | **2.5×** |
| fastapi Discussion #7320 | 开发者自述 | 基线 | 单线程 20× 吞吐 | 20×（作者自承其他库也有贡献） |

> ⚠️ 搜索中出现的「axum 6.8M RPS / FastAPI 24,800 RPS」（约 250×）来自 tech-insider.org，
> 该文把 R23 日期写成 2026-01（实际 2025-02/03），数字与 TechEmpower 社区实测差 20 倍，
> **判定不可靠，不予采信**。

**可采信区间**：I/O 密集 Web 服务，Rust 吞吐 2.5×–10×，内存 3×–10×；带真实 DB/网络可低至 1.06×。

---

## 7. 当前架构：开销下降 **0%**，实为净增

这是本文最重要的结论。Rust 的出站 100% 是本地 Python helper，数据面
（curl_cffi TLS 指纹、turnstile、PoW、SSE 事件解析、b64 收集）**全部仍在 Python 内**。

| `13` 承诺 | 可否实现 | 原因 |
|-----------|---------|------|
| RSS −50%~−70% | **不可能** | Rust 5.2MB 是省了，但 Python helper(10.9MB) + 生产(224MB) 一个没少。**系统总 RSS 反而 +5.2MB** |
| CPU −30%~−50% | **不可能** | 98.5% 全产生在 Python 的 submit worker 里，`crates/` 里没有任何 task queue 实现。Rust 只多加一次 JSON 往返，是**净增 CPU** |
| 同机并发 ×2–3 | **不可能** | 上限由三个 Python 约束串联决定：anyio 池 40 × `_lock_for_email` 串行 × GIL 单核。Rust `Semaphore(3)` 比三者都松，**改它无效** |
| 生图 E2E +0%~+15% | **指标无意义** | E2E 43–111s，Rust 参与 <1ms。由上游 + CF 决定，与网关语言关系为**零** |
| 吞吐 ×2.5–10（公开基准） | **不适用** | 基准测框架自身处理 HTTP 的能力。本系统每请求 6–110s 全在等上游，框架占比 <0.01%。**Amdahl：优化占比 0.01% 的部分，整体最多提升 0.01%** |

---

## 8. 完全重写后（Python 归零）的分维度预估

前提：`image_task_service` 2,456 行 + `account_service` + `openai_backend_api` 全部搬到 Rust，
**并关停 `chatgpt2api-local` 容器**。只重写 face 不搬调度，下表全部收益为 0。

| 维度 | Python 实测 | Rust 预估 | 下降比例 | 置信度 |
|------|------------|----------|---------|--------|
| 常驻 RSS | 224 MB | 15–40 MB | **−82% ~ −93%** | 实测锚定（Rust 私有 432KB ↔ axum 空闲 60–90MB 上界） |
| 峰值 VmPeak | 1,544 MB | 80–200 MB | **−87% ~ −95%** | 推断 |
| **稳态 CPU** | **98.5%** | 5–15% | **−85% ~ −95%** | **实测锚定** —— 98.5% 中绝大部分是 GIL 争抢的纯浪费，tokio 单 timer 可完全消除 |
| 上下文切换 | 23 次/秒 | <0.1 次/秒 | **−99%+** | 实测（709,436 vs 27，差 4 个数量级） |
| OS 线程 | 30 | 4–8（= 核数） | −73% ~ −87% | 推断 |
| 每请求网关 CPU | ~1.6s（含 PoW/turnstile） | 0.3–0.8s | **−50% ~ −80%** | 外推 —— PoW 是纯算力，Rust 2–5× |
| **同机并发上限** | **40**（anyio 池） | 数百 | **×5 ~ ×15** | 推断；新上限变为账号配额与 CF 风控 |
| 容器镜像 | 960 MB | 25–40 MB（distroless） | **−96% ~ −97%** | 实测锚定（ELF 7.2MB） |
| 部署产物 | 105MB venv + 82 依赖 | 单 ELF 7–15 MB | −86% ~ −93% | 实测 |
| 冷启动 | 未测（82 包 import） | 毫秒级 | 估 −90%+ | **未实测**，仅标注 |
| **E2E 延迟** | 43–111s（图）/ 6.3s（文本） | **同上** | **0%** | 实测 —— 99% 是上游等待，Amdahl 封顶 |

---

## 9. 三条必须说清的风险

**① 最大不确定项是 TLS 指纹。** 当前 runlog 里 Rust 侧 `rust-conc-matrix-out.txt` 0/6、
`rust-mvp-multi-conc2.json` 0/4，全挂在 CF 403。若 `wreq` 的 JA3/JA4 不能等效复现 curl_cffi，
成功率下降会**完全抵消**上表所有收益。这正是「**self=0 优先于变快**」这条原则要防的事。

**② `_lock_for_email` 的同账号串行是业务约束不是技术约束**（同账号并发会被上游风控），
换语言改不掉。所以「并发 ×5–15」的分母是**跨账号**并发。

**③ CPU 那 −85%~−95% 的前提是把 `image_task_service.py` 2,456 行搬过来。**
只重写 face 不搬调度，这项收益是 0 —— 这正是当前状态。

---

## 10. 复现方式

```bash
# 进程级（Python）
ssh panda 'docker exec chatgpt2api-local sh -c "
  cat /proc/1/status | grep -E \"VmRSS|VmPeak|VmSize|Threads|voluntary\";
  ls /proc/1/task | wc -l; cat /proc/1/smaps_rollup"'

# CPU 归因：10s 采样各线程
ssh panda 'docker exec chatgpt2api-local sh -c "
  for t in /proc/1/task/*; do echo \$(basename \$t) \$(awk \"{print \\\$14+\\\$15}\" \$t/stat); done" '
# 隔 10s 再跑一次做差分，除以 (10 * 100) 得 CPU%

# anyio 池大小
ssh panda 'docker exec chatgpt2api-local /app/.venv/bin/python3 -c "
  import anyio.to_thread; print(anyio.to_thread.current_default_thread_limiter().total_tokens)"'

# submit_workers
ssh panda 'docker exec chatgpt2api-local /app/.venv/bin/python3 -c "
  import json; print(json.load(open(\"/app/config.json\"))[\"image_task_queue\"][\"submit_workers\"])"'

# 请求耗时分布
ssh panda 'docker exec chatgpt2api-local /app/.venv/bin/python3 -c "
  import json,statistics as s
  d=[json.loads(l) for l in open(\"/app/data/logs.jsonl\")]
  ok=[x[\"duration_ms\"] for x in d if x.get(\"ok\") and x.get(\"scene\")==\"chat\"]
  ok.sort(); print(len(ok), ok[len(ok)//2], ok[int(len(ok)*.95)])"'

# Rust 侧出站目标（应只有 HELPER_URL）
grep -rn 'reqwest::Client\|\.get(\|\.post(' crates/ --include=*.rs | grep -v test
```
