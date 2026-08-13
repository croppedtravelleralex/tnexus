# Changelog

## 2026-08-13

### 新增：OCR 以按次 $0.01 接入 NewAPI

- `deploy/panda/newapi_tnexus_ocr.sh`（`apply|sync-key|status|rollback`）：注册 `tnexus-ocr` 分组、渠道指向 `grok2api-rs :8000`、模型 `grok-vision-ocr`、`ModelPrice` 按次 0.01。
- 与生图渠道隔离的两点：key 用**静态** `GROK_GATEWAY_AUTH_KEY`（不受每日轮换的 `GATEWAY_AUTH_KEY` JWT 影响）；分组倍率固定 **1.0**，因为 `tnexus` 组是 0.1，复用会让实收变成 $0.001。
- 仅内网暴露，不改 nginx。

### 变更：生图内容审查拒绝改判客户端错误

- 上游因暴力/色情提示词拒绝出图，原先归 `upstream` → 502，会拉低渠道成功率并可能触发 NewAPI 降权。现归 `client` → 400。
- 匹配 `content_policy_violation` / `防护限制` / `missing_reference_image`；`image_instant_limit` 仍归 Gate（有测试防抢占）。

### 修复：Grok 生图选 16:9 仍出 1:1

- **现象**：对话面板选任何画幅比例，出图都是正方形。
- **根因**：`imagine.rs` 按模型名分 Lite / Pro 两条路，Pro 走 WS 带 `aspect_ratio` 字段，**Lite 分支的函数签名根本不接收 `aspect_ratio`**，参数在分叉处被丢弃；Lite 用的是普通对话接口 `conversations/new`，payload 无任何比例字段。生产日志中的「Grok Web Lite 响应结束…」表明线上走的正是 Lite。
- **修复**：`apply_aspect_hint` 在非 1:1 时把画幅要求写进提示词（Lite 唯一可用通道），并把 `aspect_ratio` 贯通到 Lite 分支。
- **注**：Studio 链路（`jobs` 表）比例正常，实测 `2752x1536` 出图 `1678x937`；该表最新记录停在 08-04，说明问题不在 Studio。

### 修复：生图轮询在任务已终结后仍空转到 420s 超时

- **现象**：近 24h gateway 有 14 次 `image poll timeout after 420.0s`，每次跑满 ~105 次轮询、耗时约 7 分钟，全程 `no ids yet`，是当前生图失败的最大来源。
- **根因**：终止判定被 `conversation_has_image_gen_activity` 把守，而该函数只判断会话里**是否存在** image_gen 节点。任务一旦启动该节点就恒为 true，导致失败识别被永久屏蔽；上游 `/backend-api/tasks` 已返回 `status`（completed/failed），代码却只取 `file_ids`、无视状态。
- **修复**：`tasks_all_terminal` 读取任务状态，连续两轮观察到全部终结且无图片 id 时立即失败并带上任务错误文本，不再等满 wall budget。

### 修复：Grok 号池额度探测结果显示「未知」

- **现象**：`POST /rest/rate-limits` 已成功写入 `fast=30/30`，号池页仍全部「未知」。
- **根因**：`GET /admin/accounts/:id/quota` 返回 `{ items: [...] }`，前端当数组取 `[0]` 得到 `undefined`；打开页面还对每行并发打上游。
- **修复**：解包 `items`、优先展示 `fast` 窗口；列表只读库，上游探测改走「同步全部额度」。

### 修复：生产前端把 API 打到 `http://localhost:9000`

- **现象**：打开 `https://tnexus.relai.asia` 提示「无法连接服务器…（http://localhost:9000）」。Panda 上 `tnexus-api` / `tnexus-worker` 实际健康。
- **根因**：静态 UI 构建时 `NEXT_PUBLIC_API_BASE` 被 `web/.env.local` 打成 localhost；浏览器请求用户本机 9000。
- **修复**：非 localhost 主机一律同域请求；`build_push_ghcr.sh` 每次打包 tnexus 都重建 web；`.dockerignore` 排除 `.env.local`。
