# 36 — 图片交付、带宽与显示格式策略

最后更新：**2026-08-01**

与 [32-independent-deploy.md](../gptimage-gateway-rs/docs/32-independent-deploy.md)（独立部署带宽）、[R2.md](R2.md)（对象存储）、[35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md)（替代进度）同步。

---

## 1. 带宽分层（按链路）

| 阶段 | 流量路径 | 现状 TNexus | 目标 |
|------|----------|-------------|------|
| 上游拉图 | ChatGPT → Panda Gateway | ~1.1MB PNG/张 + ~5KB SSE | **不可消**（estuary 契约要求 Bearer；见 §4） |
| 生图 API 回包 | Gateway → Worker | URL 模式 ~163B；b64 ~1.5MB | **仅 URL**（去掉 b64 默认路径） |
| 展示（列表/预览） | 用户 ↔ 服务 | 常 302 到全尺寸 PNG asset（~1.1MB） | **AVIF/WebP thumb**（§2） |
| 下载 | 用户 ↔ 服务 | PNG `download_url` / asset | **保持 PNG** |
| 持久化（可选） | Panda → R2 | 未启用（`r2:false`） | 上传一次；用户侧走 CF CDN |
| 理想：用户看图 | 用户 ↔ CF ↔ R2/上游 | 经 Panda | **不经 Panda**（§3） |

**结论**：「URL 生图」只省 Worker↔Gateway 回包；**不省** ChatGPT→Panda 拉图。要省 Panda→用户 展示流量，靠 **WebP/AVIF 缩略图**；要省 Panda 出口看图流量，靠 **R2 + 独立图床域** 或 **Edge 302 到可直连的上游 URL**。

---

## 2. 显示格式：AVIF / WebP（下载仍 PNG）

### 2.1 原则

| 用途 | 格式 | 典型体积（1024 原图） |
|------|------|----------------------|
| 列表缩略图 `w=240` | AVIF > WebP > JPEG | ~15–40 KB |
| 预览 `w=512` | 同上 | ~40–80 KB |
| 下载 / 原图 | PNG | ~1.1 MB |

### 2.2 代码现状（2026-08-01）

| 组件 | 状态 |
|------|------|
| `tnexus-storage` | R2 上传时生成 `preview.webp`、`thumb.webp` |
| `GET /api/images/thumb/{id}?w=` | 支持 WebP（`Accept: image/webp`）/ JPEG fallback；**无 AVIF** |
| URL 生图 + `source_url` 为 Gateway asset | thumb 接口 **302 到全 PNG** → **展示未压缩（BUG）** |
| `download_url` | 指向 PNG asset / R2 original |

### 2.3 待办（P1）

1. **修 thumb**：有 `source_url` 时禁止 302 原图；服务端缩放后编码 WebP/AVIF 返回。
2. **加 AVIF**：`encode_thumbnail` 增加 `Accept: image/avif` 分支（如 `ravif`）。
3. **协商顺序**：`avif → webp → jpeg`。
4. **下载分离**：`preview_url`/`thumb_url` 仅 thumb API；`download_url` 仅 PNG。

### 2.4 预期收益（修 thumb 后）

- 历史列表 20 张：~22 MB → **~0.6 MB**（用户↔Panda 展示段）
- **不减少** ChatGPT→Panda 拉图带宽

---

## 3. 目标架构：仅 URL + 独立图床域 + Edge 302

### 3.1 方案描述

```
生图：Gateway SSE + resolve estuary URL → 不 download_image_bytes
      → 返回 signed 元数据 / 短期 token

展示：img.tnexus.example.com（CF 橙云）
      → Edge Worker 或 R2 presigned
      → 302 到 R2 或上游临时 URL
      → 浏览器直连，Panda 不代理 body

缩略图：thumb API 或 R2 预生成 WebP/AVIF
下载：PNG 的 signed URL（R2 original 或一次性 redirect）
```

### 3.2 能否「所有图片不经过服务器」？

| 环节 | 能否绕过 Panda 字节代理 | 条件 |
|------|------------------------|------|
| ChatGPT → 拿到图 | ❌ 总要有一次 | Gateway 用 Bearer 拉图或解析 URL；**无法零字节** |
| Panda → R2 上传 | ❌ 至少一次上行 | 除非改 Worker 流式直传 R2（仍过 Panda 网卡） |
| 用户看 WebP 缩略图 | ✅ | R2 + `img.*` 自定义域；CF CDN 出 R2 **$0 出站** |
| 用户下 PNG | ✅ | R2 presigned GET；不经 Panda |
| Edge 302 到 **ChatGPT estuary URL** | ⚠️ | 契约要求 **Bearer**（`build_estuary_download_headers`）；裸 URL 浏览器 fetch **应 403**；须 **生产 curl 探针** 验证 |
| Edge 302 到 **R2** | ✅ | 推荐；不暴露 Plus token |

**严格结论**：

- **不能**做到「生图流程零字节经过 Panda」（ChatGPT 侧总要有一次会话 + 拉图或等价操作）。
- **可以**做到「用户侧展示与下载零字节经过 Panda」（R2 + 独立图床域 + signed URL + Edge 302，**不** stream 代理）。
- **不能**在未验证前假设 estuary URL 可浏览器直连。

### 3.3 与 Python `:8012` 异步 `url` 模式对比

| | Python 生产（异步 url） | TNexus 目标架构 |
|--|-------------------------|-----------------|
| 生图回包 | ~0.7KB（上游 CDN URL） | ~0.2KB（signed 引用） |
| 拉图发生处 | 客户端 / 异步任务拉上游 | 可推迟到首次访问或 R2 上传时 |
| Panda 带宽 | 异步队列 + url 时较轻 | R2 上线后用户侧最轻 |

### 3.4 实施顺序

1. 去掉默认 b64；Worker/API 仅 URL。
2. 修 thumb WebP/AVIF；禁止 302 全图。
3. 启用 R2；Gateway **直传 R2**（避免 Worker 二次拉 Gateway）。
4. `img.` 子域 + presigned URL；CF 缓存规则（thumb 长缓存，PNG 短 TTL）。
5. （可选）Edge Worker：校验 TNexus session cookie → 302 R2；**不** `fetch()` 代理 body。
6. estuary 裸 URL 探针；仅当无 Bearer 200 时才考虑上游 302。

---

## 4. Estuary 契约（已查清，非猜测）

来源：`crates/protocol/src/image_contract.rs`、`crates/upstream/src/estuary.rs`、`docs/00-contract.md`。

1. `GET /backend-api/files/{id}/download` → JSON `download_url`
2. `GET download_url` **必须** `Authorization: Bearer {access_token}`
3. 负例：无 Bearer → **必须失败**（测试 M-I5）

因此：**不能把带会话的 estuary URL 直接给浏览器**（除非实测 SAS 型 URL 可无 Header 访问，需探针）。

---

## 5. 配置清单（Panda）

```bash
# 显示 / 存储
R2_ACCOUNT_ID=...
R2_ACCESS_KEY_ID=...
R2_SECRET_ACCESS_KEY=...
R2_BUCKET=tnexus-assets
PRESIGN_TTL_SECS=3600

# Gateway
IMAGE_RESPONSE_FORMAT=url          # Worker 侧（计划）
GATEWAY_SKIP_UPSTREAM_DOWNLOAD=0   # 探针通过后考虑 1
PIPELINE_EVENTS_FILE=/data/pool/pipeline_events.ndjson

# 图床域（计划）
# ASSET_PUBLIC_BASE=https://img.tnexus.relai.asia
```

---

## 6. 相关文档

| 文档 | 内容 |
|------|------|
| [R2.md](R2.md) | R2 配置与定价 |
| [35-tnexus-gptimage-gap.md](35-tnexus-gptimage-gap.md) | 替代 Python 差距 |
| [HANDOFF.md](../HANDOFF.md) | 部署与已知问题 |
| gptimage `docs/07-account-pool-performance-upgrade.md` | Python 侧 URL 优先 / b64 窗口 |
| gptimage `docs/32-independent-deploy.md` | Panda 30Mbps 与 R2 评估 |
