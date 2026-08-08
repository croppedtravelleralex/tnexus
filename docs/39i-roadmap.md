# 39i — Grok 子系统 Roadmap（多路侦察汇总 2026-08-08）

> 来源：5 路只读侦察（后端体检/前端部署/签名链路/文档对照/Go 全貌）+ 全程攻坚记录。
> 结论先行：**技术层已完备可切流，卡点是外部账号池**；后续工作分「切流放行 / 深化 / 远期」三层。

---

## 1. 当前状态总览（五维健康度）

| 维度 | 状态 | 依据 |
|------|------|------|
| **后端代码** | ✅ A 级（~3.2 万行 / 19 crate / 测试 230+ 全绿） | 全 crate 编译 + clippy 0 + CI 绿；grok2api-rs 32 / provider-web 48 / signer 13 单测 |
| **纯 HTTP 签名器** | ✅ 打通（唯一技术突破点） | bundle 执行 → 94 字符签名 → GET 200 实证；meta 实时注入 + stub 修复后任意 meta 稳定 |
| **Admin 管理面** | 🟡 B-（路由全接、数据面部分占位） | 8 域接线 + import + 登录；**audits/dashboard/media 数据源仍内存占位（TODO）**；缺 Go 侧 egress/imagepipeline 两域 |
| **前端** | 🟡 B（页面全建、数据面依赖后端） | 7 页 + 10 组件 + 独立 grokApi；静态导出已过；未接线入口已清点 |
| **上线链路** | 🟡 B（部署脚本/CI 全绿，**流量未开**） | compose/nginx/CI 就绪；media 501/videos 500 待收尾；3 个 sidecar 任务（quota_refresh/dispatch_probe/pin_sync）标注 TODO |

**当前唯一硬阻塞**：grokImage 账号池（687 个实测）被 grok 批量风控禁言发消息——POST 全 403。与代码无关。

## 2. Roadmap

### 近期（账号就绪后 1-2 天，切流放行）
- [ ] **R1 全链路验证**：新账号 → 本地签名器（实时 meta）→ POST → SSE 流 → OCR 图片上传 → 生图
- [ ] **R2 签名器产品化**：rquickjs 集成（同步 SHA-256 polyfill，替代 node vm）；`GROK2API_SIGNER_MODE=local`
      开箱即用；meta 实时抓取进 MetaCache（direct.rs 或 signer.rs 统一）
- [ ] **R3 Admin 数据面补真**：audits/dashboard/media 接 PG（grok_request_audits/grok_media_assets）；
      media get / size-summary 从 501 → 200；videos 轮询上游
- [ ] **R4 三个 sidecar 任务**：web_quota_refresh / web_dispatch_probe / pin_sync——Rust 实现或 Go sidecar
      声明（决策点：全部 Rust 化是既定方向）
- [ ] **R5 上线执行**：Panda 部署（deploy.sh pull+up）、nginx /grok/v1/ 反代、.env 全量、探针 24h、
      shadow 真实数据对比（dispatch diff <5%）、切流回滚预案

### 中期（1-2 周，深化与韧性）
- [ ] **M1 账号生命周期闭环**：凭据 refresh（sso 过期自动刷新）、风控检测（403/禁言自动下线）、
      账号分级（web/build/console 三池对齐 Go 四池探针）
- [ ] **M2 生图直连**：direct_imagine_ws 真实联调（WS 收帧）、image-pipeline 接线（当前 0 测试最弱 crate）
- [ ] **M3 Admin 补域**：egress（出口管理/代理池）+ imagepipeline（生图流水线视图）两域对齐 Go
- [ ] **M4 前端深化**：grok 对话页完整体验（流式渲染/会话历史/多模型切换）、OCR 面板批量、
      号池页配额列批量端点（当前 QUOTA_FETCH_LIMIT=20 截断）
- [ ] **M5 运行验收**：shadow 真实数据 P50/P95/P99、探针稳定性 24h×7、额度恢复链路真跑

### 远期（1 月+，演进）
- [ ] **F1 签名器自维护**：chunk 自动重提取（上游演化检测：meta/签名格式校验 → 告警 + 半自动重生成）；
      反混淆流水线文档化
- [ ] **F2 前端 Rust 化决策**（Dioxus/Yew 评估）——激进，用户拍板；或维持 Next.js 静态导出 + 独立部署
- [ ] **F3 多租户/额度网关**：client_keys 计费、模型路由定价、用量报表（对齐新-api 的商业模式面）
- [ ] **F4 弹性出口**：webshare 池重新评估（当前全被 grok 拉黑）；住宅/机房分层、账号→出口亲和
- [ ] **F5 grok-bridge 命运**：已写好 1942 行但被否决——转为「备胎」（CDP 兜底文档化）或删除减负

## 3. 可改进方向（按 ROI）

| 方向 | 现状 | 改进 | 收益 |
|------|------|------|------|
| **签名器产品化** | node vm 验证版 | rquickjs 内嵌 + meta 实时注入 | local 模式开箱即用，无 node 依赖 |
| **Admin 数据面** | 内存占位（3 处 TODO） | PG store 补真 | 管理台从演示→可用 |
| **测试覆盖** | 230+（grok-pool 集成测试在 tests/） | image-pipeline 0 测试、grok-accountsync/grok-audit 0 单测 | 最弱三 crate 补测 |
| **文档-代码一致性** | 39g 计数滞后 | 39g 刷新（31/68 需核对） | 审计可信 |
| **部署演练** | CI 绿但从未 Panda 实跑 | 一次真实部署演练（含 rollback） | 上线零惊吓 |
| **CI 时长** | build 33m | 缓存分层（sccache/GH 缓存） | 迭代加速 |
| **错误可观测** | tasks.rs 日志提示 | 探针/错误指标进 grok-audit | 风控提前预警 |
| **安全** | Bearer JWT 双体系 | 统一登录（会话自动换 token）| 管理台 UX |

## 4. 最终实现目标（Done 定义）

> **grok2api 完整 Rust 化替代 Go 生产，纯 HTTP 无桥，管理台可用，账号生命周期自洽。**

1. **替代**：`/v1` 对话/OCR/生图 + `/admin` 管理面 1:1 覆盖 Go 生产功能（含 egress/imagepipeline 两域），
   Panda 切流后 Go 进程可停
2. **纯 HTTP**：签名器本地化（无 browser-bridge、无外部 signer、无 node 运行时——rquickjs 内嵌）；
   代理分层（本地出口 + 账号出口亲和）
3. **账号自洽**：凭据 refresh、风控自动下线、额度恢复、三池探针——**不再需要人工换号**
4. **管理台**：号池导入/编辑/额度/审计/仪表盘/设置全数据面真实（PG 直读），登录统一
5. **韧性**：探针 24h 稳定、dispatch diff <5%、media/videos 全通、CI 全绿、部署回滚 <5min

**退出条件（可量化）**：新账号到货 → R1 全链路绿 → R5 切流 → M5 运行验收 1 周 → 远期 F1-F5 渐进。

## 5. 风险登记

| 风险 | 等级 | 缓解 |
|------|------|------|
| 账号池整体风控持续 | 🔴 高 | R1 新账号验证；M1 风控自动下线；F4 出口分层 |
| 上游前端演化（签名器 moduleId/meta 变） | 🟡 中 | F1 自维护流水线；签名格式校验告警 |
| sidecar 任务缺口（quota_refresh 等） | 🟡 中 | R4 排期；期间手动运维兜底 |
| grok-bridge 死代码（1942 行未用） | 🟢 低 | F5 决策（删/留） |
| CI 时长膨胀 | 🟢 低 | 缓存分层 |
