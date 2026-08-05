# 41 — Compute 向优化：价值与收益上限

最后更新：**2026-08-04**

> 前提：Gateway 已 `DATA_PLANE=upstream`（Rust 发 TLS + SSE + PoW/turnstile + 下载）。
> 对照实测见 [26-perf-measured-20260726.md](26-perf-measured-20260726.md) §11。

## 1. 一句话结论

| 优化类型 | 对 E2E 墙钟 | 对 CPU/内存能效 | 建议 |
|----------|-------------|-----------------|------|
| **关停 Python `:8012` + 去掉 submit_worker** | 0% | **极大**（空闲 −85%~−95% CPU，−78% 内存已部分验证） | **P0**，停服路径 |
| **会话复用 / 减少 bootstrap** | **1–3%** | 中 | P1 |
| **PoW / turnstile 算法与并行** | **0.5–2%** | 低–中 | P2（已在 Rust，微优化） |
| **SSE 解析 / 零拷贝 / simd b64** | **≈0%** 墙钟 | 低（降低负载期 CPU 尖峰） | P3，边际 |
| **JSON 加速** | **&lt;0.1%** | 可忽略 | 不做 |

**Compute 向微优化无法把 37s 生图变成 20s** —— 墙钟里 **~85–95% 是上游 SSE 与网络**（§11 + 下文分解）。  
Rust 重写的「能效比」主要体现在 **内存、线程、空闲寄生虫、并行 batch**，不是再抠 10% 的 PoW。

---

## 2. 时间预算分解（单张 b64，TNexus `:8014`）

来源：`artifacts/b64_parallel_perf/results.json`（10 路并行，TNexus 侧有 `_tnexus_pipeline` 分段）。

| 阶段 | 典型范围 | 占 upstream_wall | 可计算优化？ |
|------|----------|------------------|--------------|
| `bootstrap_ms` | 0.1–1.6s | ~2–4% | 连接池、会话复用 |
| `requirements_ms` | 0.2–1.7s | ~2–5% | PoW/turnstile（已在 `crates/upstream`） |
| `prepare_ms` | 0.13–0.27s | &lt;1% | 小 |
| **`sse_ms`** | **20–46s** | **~80–95%** | **网络 + 上游生成，非本地 CPU** |
| `resolve_url_ms` | 0.4–3.6s | ~1–8% | 偶发 poll，多数网络 |
| `download_ms` | 0.3–0.8s | ~1–2% | 网络 |
| gateway b64 编码 + JSON | &lt;0.05s | &lt;0.2% | simd base64 |

**upstream_wall ≈ gateway_wall**（差 &lt;50ms），说明网关侧无 200s 级隐藏排队（admin 重试已 cap）。

以 **p50 ≈ 38s** 为例：

- **本地可优化段合计**：约 **2–4s**（bootstrap + requirements + prepare + download + 编码）
- **不可压缩段（等上游）**：约 **34–36s**

由 Amdahl：即便本地段 **全部减半**，E2E 仅 **−2.5% ~ −5%**；这与 §7「优化 0.01% 段整体最多 0.01%」同族，只是 upstream 已迁到 Rust 后本地段略大。

---

## 3. 分项优化：收益上限（估算）

### 3.1 关停 Python + 搬调度（架构，非 micro-opt）

| 项 | 现状 | 收益 | 置信度 |
|----|------|------|--------|
| 空闲 CPU 98.5% → 5–15% | submit_worker ×10 + GIL | **−85%~−95% 整机空闲 CPU** | 高（§8） |
| 容器内存 729 MB → ~160 MB | 停 `chatgpt2api-local` | **再 −~570 MB** | 高（§11） |
| E2E | — | **0%** | 实测 |

这是 **唯一** 能让「CPU 能效比」出现数量级差异的项。

### 3.2 会话 / 连接复用（`bootstrap_ms`）

| 假设 | 节省 | E2E（38s 基准） |
|------|------|-----------------|
| 同账号 2 分钟内复用 TLS + cookie | 0.3–1.0s / 次 | **0.8–2.6%** |
| 10 路并行不同账号 | 收益摊薄 | batch 墙钟 **1–3%** |

实现：`upstream::Client` 长连接池、按账号缓存 bootstrap 状态（注意 CF 与会话过期）。

### 3.3 PoW + Turnstile（`requirements_ms`）

已在 Rust：`crates/upstream/src/pow.rs`、`turnstile.rs`、`requirements.rs`。

| 假设 | 节省 | E2E |
|------|------|-----|
| PoW 求解 2× 快（算法/并行核） | 0.3–0.8s | **0.8–2%** |
| Turnstile VM 常数项优化 | 0.1–0.3s | **&lt;1%** |
| 缓存 requirements（同会话） | 0.5–1.5s | **1–4%**（需验证上游是否允许） |

文本链路实测（§4.2）：`request_build` 含 turnstile+PoW **~1.6s**；生图 requirements 通常 **0.5–1.7s**。

**风险**：PoW 优化不能牺牲 CF 通过率；失败重试会 **净增** E2E（§9 ①）。

### 3.4 SSE 解析与流处理（`sse_ms` 时段的 CPU）

`sse_ms` 墙钟长，但 **CPU 主要在读 socket**；解析器优化：

- 减少拷贝、`Bytes` 复用、simd 扫行
- **不缩短** 上游生成时间 → **E2E ≈ 0%**
- 可能降低 10 路并行时 CPU 峰值（66%→50% 量级猜测，**未实测**）

### 3.5 Base64 / JSON（响应路径）

单张 ~1MB PNG → ~1.3MB base64 字符串：

| 操作 | Python 参考 | Rust 现状 | 再优化上限 |
|------|-------------|-----------|------------|
| b64 encode | ~几十 ms | `BASE64.encode` 一次 | **&lt;20ms** |
| JSON 序列化 | — | serde_json | **&lt;10ms** |

对 38s E2E：**&lt;0.5%**。**不值得**单独立项。

### 3.6 并行与调度（已做部分）

| 已上线 | 效果（§11） |
|--------|-------------|
| `IMAGE_ADMIN_RETRY_MAX=3` | 消除 200s+ 尾流 |
| `IMAGE_GLOBAL_CONCURRENCY=10`、per-account inflight=1 | 10 路 batch 54.5s vs 62.8s |
| admin 跳过 `dispatch_gate` | Studio 10 路不 429 |
| worker `IMAGE_PARALLEL_CONCURRENCY=8` | 软限流 |

进一步 compute 优化对 **batch 墙钟** 的帮助：主要靠 **更少 CPU 争用** 换 **略高并发**，在 **2 vCPU** 上上限明显。

---

## 4. 推荐优先级（性能调优路线图）

| 优先级 | 动作 | 预期收益 | 成本 |
|--------|------|----------|------|
| **P0** | 灰度 → 关停 `:8012` | 空闲 CPU −85%+、内存 −570MB+ | 运维/切流 |
| **P1** | 按账号会话复用、HTTP 连接池 | E2E **1–3%**，batch **1–3%** | 中 |
| **P1** | 空闲 CPU 验收脚本（5min idle vs 负载） | 澄清「能效比」口径 | 低 |
| **P2** | requirements 缓存 / PoW 并行 | E2E **0.5–2%** | 中；需 AB CF 率 |
| **P3** | SSE 零拷贝、simd b64 | E2E **≈0%**；CPU 尖峰略降 | 中 |
| **不做** | 替换 JSON 库、手写序列化 | &lt;0.1% | — |

---

## 5. 与「Rust 重写」预期的关系

| 用户预期 | 文档原话 | 实测 |
|----------|----------|------|
| CPU 降一半+ | §8 稳态 −85%~−95%（**停 Python**） | 负载 docker% 接近；proc CPU **−19%~−62%** |
| 生图更快 | §8 E2E **0%** | p50 持平，p99 TNexus 更好 |
| 内存大降 | §8 −82%~−93% | 容器 **−78%**（8012 仍在） |

**Compute 向微优化**是在「已经等上游 35s」的前提下抠 **2–4s 本地段**；  
**架构级 Rust 收益**是 **去掉 Python 寄生虫 + 4.5× 内存**，已在 §11 验证方向。

---

## 6. 复现与扩展

```bash
# 并行 b64 + 资源曲线
python3 scripts/test_b64_parallel_perf.py --auth-8012 ... --auth-8014 ... -n 10
python3 scripts/plot_b64_parallel_perf.py /tmp/b64_parallel_perf/results.json

# 串行 + 进程 CPU
python3 scripts/test_b64_chain_perf.py --auth-8012 ... --auth-8014 ... -n 2
```

扩展实验（未做）：

- 同账号连续 N 张 → 量化 bootstrap 复用收益
- `perf record` / `tokio-console` 在 SSE 段采样 → 验证解析器 CPU 占比
- 关停 `:8012` 后 5min idle CPU 对照
