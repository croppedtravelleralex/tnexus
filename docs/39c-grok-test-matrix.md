# 39c — Grok 移植测试矩阵

最后更新：**2026-08-04**  
主文档：[39-grok2api-rust-migration.md](39-grok2api-rust-migration.md) · 路线图：[39a-grok-roadmap.md](39a-grok-roadmap.md)

## 0. 测试层级

| 层级 | 范围 | 命令 |
|------|------|------|
| L0 | 纯逻辑、无 IO | `cargo test -p grok-domain -p grok-pool-index` |
| L1 | PG / Redis 集成 | `cargo test -p grok-storage -- --ignored` |
| L2 | HTTP handler（mock upstream） | `cargo test -p grok-gateway -p grok-admin` |
| L3 | bridge 联调（staging） | `GROK_INTEGRATION=1 cargo test --ignored` |
| L4 | E2E / 门禁 | `./scripts/grok_migration_gate.sh <phase>` |

晋级：未达标 **停层修复**；CF/anti-bot 记 `upstream`，不当作 Rust self bug 过门（与 [18-test-matrix.md](18-test-matrix.md) 一致）。

---

## 1. Go → Rust 单测移植对照表

源仓：`AutoRegister/grokImage/backend/`（111 个 `*_test.go`）。  
Rust 目标 crate 见 [39d-grok-go-rust-map.md](39d-grok-go-rust-map.md)。

### 1.1 P0 必移植（号池 + 网关核心）

| Go 测试文件 | 用例主题 | Rust crate / 模块 | Phase |
|-------------|----------|-------------------|-------|
| `gateway/selector_test.go` | 选号、粘滞、冷却、lease | `grok-gateway::selector` | G3 |
| `gateway/service_test.go` | 推理编排、失败分类 | `grok-gateway::service` | G1 |
| `gateway/failure_test.go` | 错误 class | `grok-gateway::failure` | G1 |
| `gateway/timing_test.go` | 超时语义 | `grok-gateway` | G2 |
| `account/web_pool_test.go` | Web 池 reconcile | `grok-pool::web` | G3 |
| `account/web_pool_probe_test.go` | 探针 L0/L1/L2 | `grok-pool::web_probe` | G3 |
| `account/web_pool_pins_test.go` | dispatch pin | `grok-pool::pins` | G3 |
| `account/four_pool_probe_test.go` | Build 四池 | `grok-pool::build` | G3 |
| `account/web_lane_quota_test.go` | lane 额度 | `grok-pool` | G3 |
| `account/imagine_slots_test.go` | imagine slot | `grok-image-pipeline` | G2 |
| `account/imagine_quota_test.go` | imagine 额度 | `grok-domain` | G2 |
| `account/quota_test.go` | quota 窗口 | `grok-domain` | G1 |
| `account/poolindex/poolindex_test.go` | heap/BTree | `grok-pool-index` | G3 |
| `account/poolindex/web_drr_test.go` | DRR 调度 | `grok-pool-index` | G3 |
| `account/build_probe_monitor_test.go` | Build 探针 | `grok-ops` | G4 |
| `account/build_chat_probe_test.go` | Build chat 探针 | `grok-ops` | G4 |
| `account/conversion_test.go` | Web→Build | `grok-admin::account` | G5 |
| `account/reauth_test.go` | 重认证 | `grok-admin` | G4 |

### 1.2 P1 Web Provider（推理 + OCR + 生图）

| Go 测试文件 | 用例主题 | Rust 模块 | Phase |
|-------------|----------|-----------|-------|
| `provider/web/protocol_test.go` | OpenAI 协议翻译 | `grok-conversation` | G1 |
| `provider/web/quota_test.go` | Web 刷额度 | `grok-provider-web::quota` | G3 |
| `provider/web/chat.go`（逻辑） | `contentTextAndImages`、8 图限制 | `grok-conversation` | G1 |
| `provider/web/image_prompt_test.go` | prompt 扩写 | `grok-provider-web::expand` | G2 |
| `provider/web/statsig_test.go` | Statsig 签名 | `grok-provider-web::statsig` | G1 |
| `provider/web/browser_bridge_test.go` | bridge 客户端 | `grok-egress::bridge` | G1 |
| `provider/web/chrometicket_download_test.go` | ticket 下载 | `grok-chrome-ticket` | G2 |
| `provider/web/sso_build_test.go` | SSO 导入 | `grok-admin::import` | G4 |
| `imagepipeline/scheduler_test.go` | PS/SS 槽位 | `grok-image-pipeline` | G2 |
| `chrometicket/pool_test.go` | 票池 CRUD | `grok-chrome-ticket` | G4 |

### 1.3 P2 Build / Console / 协议

| Go 测试文件 | Rust 模块 | Phase |
|-------------|-----------|-------|
| `provider/cli/adapter_test.go` | `grok-provider-build` | G5 |
| `provider/cli/normalize_test.go` | `grok-provider-build` | G5 |
| `provider/console/console_test.go` | `grok-provider-console` | G5 |
| `provider/conversation/conversation_test.go` | `grok-conversation` | G5 |
| `provider/definition_contract_test.go` | `grok-provider-core` | G0 |

### 1.4 P2 持久化 / HTTP / 运维

| Go 测试文件 | Rust 模块 | Phase |
|-------------|-----------|-------|
| `persistence/relational/repository_test.go` | `grok-storage` | G0 |
| `persistence/relational/*_test.go`（各 repo） | `grok-storage` | G0–G4 |
| `transport/http/inference/handler_test.go` | `grok-gateway` HTTP | G1 |
| `transport/http/inference/model_list_test.go` | `grok-gateway` | G1 |
| `transport/http/account/handler_test.go` | `grok-admin` | G4 |
| `transport/http/chrometicket/handler_test.go` | `grok-admin` | G4 |
| `transport/http/media/handler_test.go` | `grok-admin` | G4 |
| `transport/http/model/handler_test.go` | `grok-admin` | G4 |
| `egress/manager_test.go` | `grok-egress` | G1 |
| `egress/manager_asset_affinity_test.go` | `grok-egress` | G2 |
| `settings/service_test.go` | `grok-ops::settings` | G4 |
| `media/service_test.go` | `grok-admin::media` | G4 |
| `app/startup_test.go` | `grok-ops::startup` | G4 |
| `config/config_test.go` | `grok2api-rs::config` | G0 |
| `runtime/redis/store_integration_test.go` | `grok-runtime`（若拆） | G3 |

---

## 2. OCR / 识图专项矩阵

| ID | 用例 | 输入 | 期望 | Phase |
|----|------|------|------|-------|
| G-OCR-1 | 单图中文 | data URI PNG | 200 + 含中文 | G1 |
| G-OCR-2 | 单图英文 | HTTPS url | 200 + 含英文 | G1 |
| G-OCR-3 | 无文字图 | 风景图 | 200 + 「无文字」或空 | G1 |
| G-OCR-4 | 9 张图 | 9×url | 400 | G1 |
| G-OCR-5 | 超大图 | >64MiB | 400 | G1 |
| G-OCR-6 | file_id | `input_image.file_id` | 400 明确错误 | G1 |
| G-OCR-7 | payload golden | 固定样图 | `enableImageGeneration=false` | G1 |
| G-OCR-8 | 额度 fast | 前后 quota | remaining −1 | G1 |
| G-OCR-9 | 流式 | stream:true | SSE 完整 | G1 |
| G-OCR-10 | 别名路由 | model=grok-vision-ocr | 等同 fast+禁生图 | G1 |

实现：`crates/grok-gateway/tests/ocr_e2e.rs`（L3/L4）。

---

## 3. 生图矩阵（G2）

| ID | 用例 | 期望 | 严格 |
|----|------|------|------|
| G-IMG-1 | generations 单张 | 200 + url/b64 | self=0 |
| G-IMG-2 | prompt_enhance | PS 阶段 trace 存在 | pipeline segment |
| G-IMG-3 | 10 并发 | ≥8/10 成功 | 剔 upstream |
| G-IMG-4 | media GET | 200 | 硬 |
| G-IMG-5 | worker 联调 | Studio job OK | E2E |

---

## 4. Golden 对比测试

目录：`tests/grok_golden/`

| 文件 | 内容 |
|------|------|
| `chat_ocr_request.json` | OpenAI 多模态请求 |
| `chat_ocr_upstream_payload.json` | 期望上游 body（禁生图） |
| `image_generations_request.json` | 生图请求 |
| `selector_candidates.json` | 选号输入/输出 |

流程：

1. Go 录制（staging）：`tools/export_golden.sh`（待建）
2. Rust：`cargo test -p grok-gateway golden_*`
3. diff 白名单字段：`message`、`temporary`、`deviceEnvInfo`

---

## 5. Shadow 生产对比

双跑：Go `:18000` + Rust `:8000`；同 client key；对比 `request_audits`。

脚本：`scripts/grok_shadow_compare.py`（G6）

输出：`artifacts/grok-shadow/<date>/summary.json`

---

## 6. TNexus Worker 回归

| ID | 检查 | 命令 |
|----|------|------|
| W-1 | `GROK2API_BASE` 切换 | env 指向 Rust 后 job 成功 |
| W-2 | b64 并行 | `scripts/test_b64_parallel_perf.py` |
| W-3 | pipeline 字段 | `job_results` 含阶段 JSON |
| W-4 | 构思仍走 gptimage | director 请求 host ≠ grok base |

---

## 7. CI 集成

`.github/workflows/` 增加 job（或 matrix）：

```yaml
# 片段示意
- run: cargo test -p grok-domain -p grok-storage -p grok-pool-index
- run: cargo test -p grok-gateway -p grok-provider-web
- run: ./scripts/grok_migration_gate.sh g0
```

G1+ 需 `GROK_INTEGRATION=1` + bridge mock 服务（docker-compose fixture）。

---

## 8. 门禁脚本

`scripts/grok_migration_gate.sh`：

| 子命令 | 检查 |
|--------|------|
| `g0` | build + schema + config test |
| `g1` | OCR E2E + gateway tests |
| `g2` | image generations + worker smoke |
| `g3` | pool tests + dispatch diff script |
| `g4` | admin handler tests + task health |
| `g6` | shadow summary 阈值 |

---

## 9. 不做 / 非目标用例

- grok2api 注册机 E2E（外置 AutoRegister）
- Panda 上 `cargo test` / `docker build`（红线）
- 生产 ChatGPT `:8012` 交叉测试

---

## 10. 签字栏

| Phase | 日期 | `grok_migration_gate` | 备注 |
|-------|------|----------------------|------|
| G0 | | ☐ | |
| G1 | | ☐ | |
| G2 | | ☐ | |
| G3 | | ☐ | |
| G4 | | ☐ | |
| G5 | | ☐ | |
| G6 | | ☐ | |
