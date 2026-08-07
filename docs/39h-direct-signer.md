# 39h — 纯 HTTP 无 chrome 签名器专项（statsig x-statsig-id 攻坚记录）

> 状态：2026-08-07。框架接线完成（三模式），**真实签名 bundle 反混淆未攻克**——本文件记录全部技术细节，
> 供后续续攻或切换方案时直接引用。**敏感值（密码/密钥/代理凭证）一律不写入本文件。**

---

## 1. 为什么需要签名器

grok.com 的 API（`/rest/app-chat/conversations/new` 等）强制校验 `x-statsig-id` 请求头：

| 场景 | 结果（实测） |
|------|-------------|
| 无 cookie 直连 | 401 `No credentials presented`（CF 层放行，认证层拒绝） |
| 带 cookie、无 x-statsig-id | **403 `Request rejected by anti-bot rules`** |
| 带 cookie + 有效 x-statsig-id | 预期 200（未实测——签名器未就绪） |

即：CF 层可用浏览器 UA + 出口代理过；**应用层反爬靠 x-statsig-id**，无它必 403。

## 2. 签名器是什么

- x-statsig-id 由 **grok.com 前端 Turbopack 模块**生成，非标准 statsig SDK（输入含 path/method）。
- 调用协议（从未混淆 wrapper chunk 解出，已在本地 mini-runtime 验证）：
  ```
  __grokBridgeRuntime.A(4629918)        // 取模块（wrapper）
    .default()                          // 得到 signer 函数
    (path, method)                      // 调用 → x-statsig-id 字符串
  ```
- wrapper 模块 4629918 导出 key2 = signer 函数；**真正实现模块 id = 1645e3**（chunk `1hh54l36z-re3.js`）。

## 3. 外部 signer 服务已死（关键事实）

- Go 生产链路依赖外部 HTTP 签名服务 `https://grok.wodf.de/sign`（缺省，config.yaml `credentialEncryptionKey` 同仓）。
- 2026-08-07 实测：**本地直连 / 本地代理 7897 / Panda 服务器** 三路均返回 Cloudflare managed challenge
  （`Just a moment...` 403，HTML 带 `_cf_chl_opt` 脚本）——外部 signer 全局不可用，Go 侧无 chrome 路径实际已断。
- 结论：**必须本地生成签名**（或恢复可用 signer 源）。

## 4. 本地签名器的两条路

### 4a. 纯 HTTP（用户指定路线）——提取前端模块 + 本地 JS 引擎

已完成的资产与设施：

| 资产 | 位置 | 状态 |
|------|------|------|
| grok.com 首页 HTML（metaContent 源） | /tmp/grok_home.html（本地临时） | ✅ 抓取成功（本地代理 7897 + 浏览器 UA） |
| 101 个前端 chunk | /tmp/grok_chunks/ | ✅ 下载成功 |
| 签名模块 1645e3 源码 | /tmp/mod_1645e3_raw.js | ✅ 提取 |
| Turbopack mini-runtime（本地加载 2453 模块） | 上一轮攻坚 agent 自写 | ✅ 模块加载跑通 |
| 调用协议验证 | 未混淆 wrapper（chunk 34y8rzg_5i0ce.js） | ✅ |
| **可执行 standalone bundle** | crates/grok-signer/assets/grok_sign_standalone.js | ❌ **未产出** |

**硬墙**：模块 1645e3 **重度混淆**（属性名动态拼接，如 `"child"+"Nodes"`），调用时要求完整浏览器运行时状态
（document/window/navigator/childNodes 链）。两轮 agent 攻坚（手工 DOM stub、jsdom + 真实首页、
Proxy 属性追踪、反混淆）均在调用处崩 `reading 'childNodes'`/navigator 缺失——**node 裸环境复现不了完整浏览器状态**。

**脆弱性**：即使反混淆成功，签名模块随 grok.com 前端发版演化（moduleId 会变、混淆会换），每次都要重新提取。

### 4b. Rust CDP（本地 Chrome，备用但可靠）

- grok-bridge crate（~1500 行，已提交）实现自写 CDP 客户端 + 浏览器内执行同一段 JS 签名
  （`sign_script()` 逻辑照搬 Python bridge，moduleId 4629918，env `BRIDGE_SIGNER_MODULE_ID` 可覆盖）。
- Chrome 是**本地组件**（非外部服务），全 Rust 内部化；对上游演化免疫（浏览器里永远能执行前端 JS）。
- 字面意义上"有浏览器"——与用户"纯 http 无浏览器"要求冲突，故当前未启用。

## 5. 已落地的本地签名器框架（GROK2API_SIGNER_MODE）

**提交 `770136c`**（与 4a 的 bundle 无关，框架先行）：

| 组件 | 说明 |
|------|------|
| crates/grok-signer（新 crate） | **rquickjs 0.9**（vendored quickjs）JS 引擎壳；接口 `execute_signature_bundle(js) -> Result<String, SignError>`；5s 线程脱逃超时；输出约定 `globalThis.__signOut`；内置假 signer bundle 单测 |
| crates/grok-provider-web/src/signer.rs | `SignerTrait` + 三实现：`RemoteSigner`（外部 wodf.de，现状默认）、`LocalSigner`（本地 bundle）、`FakeSigner`（仅测试） |
| grok2api-rs config | `GROK2API_SIGNER_MODE=local\|fake\|remote`（缺省 remote）；local 模式无 asset → **503 不外呼**（安全红线） |
| asset 约定 | 放 `crates/grok-signer/assets/grok_sign_standalone.js`，占位符 `__SIGN_PATH__`/`__SIGN_METHOD__` 运行时替换，结果写 `__signOut` |

**Windows 构建注意**：本机 host 是 windows-gnu 且 PATH 混入 QT 旧 mingw（gcc 7.3）会破坏链接——
构建必须 `PATH=/c/software/msys2/ucrt64/bin:$PATH` 前缀；CI（Linux）无此问题。

## 6. 代理链路探索结论（实测）

| 出口 | grok.com 结果 |
|------|--------------|
| webshare 20 住宅节点（`C:/Users/Lenovo/Downloads/Webshare 20 proxies.txt`，格式 `ip:port:user:pass`） | **全部被 grok CF 拉黑**（TCP 10054 重置，首页与 /rest/* 均拒）——这批节点对 grok 无效 |
| 本地代理 `127.0.0.1:7897`（系统代理/Clash） | ✅ 首页 200（带浏览器 UA）；API 可达（401/403 属应用层判定） |
| 本地直连（无代理） | ❌ 连不通（被墙） |

**结论**：meta 抓取/签名请求走 `GROK_LOCAL_PROXY`（本地出口）；上游 API 请求走 webshare 池（账号出口隔离）——
但 webshare 现网被 grok 拉黑，**当前有效拓扑是全部走 7897**（单出口，多账号风控风险需关注）。

## 7b. 攻坚突破（2026-08-07 晚）

- **签名器模块 1645e3 成功执行并产出合法签名**（node vm）：自包含（obfuscator.io 字符串表 + RC4 内嵌），
  只需标准 window API（TextEncoder/Uint8Array/Date/Uint32Array/crypto.subtle/RTCPeerConnection/getComputedStyle）
- **算法解密**（字符串表 t() 暴露法）：签名 = `[rand(4) + 0x100 + meta(44) + ts(4) + 0 + SHA-256(16) + 3]` 类
  结构 base64 → **~94 字符**；`.r-11220`/`F`/`Z` 是动画烟雾弹（页面实测 count=0）
- **meta 每会话动态**（实测 3 次 3 个值）：`GET https://grok.com/` 实时抓 `[name^=gr]` content 注入——必须
  每次签名前抓取
- **签名有效性实证**：bundle 签名 + 真实 sso cookie → GET 200 ✅
- **POST 403 定位于账号池级风控**：真实浏览器页面自身 POST 也 403（headless+非 headless）、
  前端 UI 拦截发送（无请求无 WS 帧）、多账号（304/86-92）/多 IP（英/日/本地）/多 body（Go schema/前端
  schema/modeId 变体）/全套 cookie（cf_clearance/device）——POST 全 403；GET 全 200
- **资产落地**：crates/grok-signer/assets/grok_sign_standalone.js（模板版，__GROK_META__ 运行时注入）
- **后续**：验证 POST 全链路需要**能正常发消息的账号**（现池被禁）；或确认 Panda 生产（wodf.de 时代）
  的账号池状态

## 7. 当前状态与选项

- **可跑链路**：号池（Panda PG 671 账号）→ 凭据解密 → meta 抓取 →（签名 ✗）→ chat。
- **三选项**：
  - A 续攻反混淆（派专家，成本高、脆弱）
  - B 启用 grok-bridge CDP 本地 Chrome 签名（当天可全链路，非字面"纯 http"）
  - C 等可用 signer 源（wodf.de 恢复或换源）

## 8. 相关 env 清单（grok2api-rs 直连）

```
GROK2API_DIRECT=1              # 直连模式（缺省开；显式 0 回退 bridge）
GROK_DATABASE_URL              # PG（号池 + 凭据）
GROK_CREDENTIAL_KEY            # base64 32B AES-GCM（解 grok_credentials.encrypted_primary）
GROK2API_PROXY_FILE            # webshare 代理文件（ip:port:user:pass 每行）
GROK2API_PROXY_LIST            # 内联代理列表（逗号分隔）
GROK_LOCAL_PROXY               # 本地出口代理（meta/签名走它；如 http://127.0.0.1:7897）
GROK2API_SIGNER_URL            # 外部 signer（缺省 https://grok.wodf.de/sign，当前已死）
GROK2API_SIGNER_MODE           # remote|local|fake（local 需 asset）
GROK_GATEWAY_AUTH_KEY          # /v1 鉴权
GROK2API_UPSTREAM_TIMEOUT_MS   # 上游总超时（缺省 60000；代理链路建议 120000）
```
