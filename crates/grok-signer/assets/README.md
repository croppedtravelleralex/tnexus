# grok_sign_standalone.js — 纯 HTTP x-statsig-id 签名器

grok.com 前端签名器模块 1645e3（Turbopack moduleId 4629918）的自包含执行产物。

## 用法

```js
const js = fs.readFileSync('grok_sign_standalone.js', 'utf8')
  .replaceAll('__GROK_META__', '<实时抓取的 meta[name^=gr] content>')
  .replaceAll('__SIGN_PATH__', '/rest/app-chat/conversations/new')
  .replaceAll('__SIGN_METHOD__', 'POST');
// 执行后 globalThis.__signOut = Promise<string>（x-statsig-id）
```

## 关键约束（攻坚结论，2026-08-07）

1. **meta 必须实时抓取**：`GET https://grok.com/`（带浏览器 UA）→ 解析
   `<meta name^=gr>` 的 content——**每次页面加载/会话都变**（实测 3 次 3 个值）。
   签名绑定该值，服务端按会话校验。
2. **完整签名 ~94 字符**（base64，70 字节）：`[rand(4) + 0x100 + meta(44) +
   时间戳(4) + 0 + SHA-256(16?) + 3]` 类结构——**必须取完整值**，截断（40 字符）
   导致 403（GET 不校验内容、POST 校验）。
3. **运行环境**：node vm（`crypto.webcrypto.subtle` 提供 SHA-256；document stub 提供
   `querySelectorAll`/`getAttribute`）。rquickjs 集成需要提供同步 SHA-256 + stub。
4. **验证状态**：bundle 签名 + 真实 sso cookie → GET 200 ✅；POST 403 为
   **账号池级风控**（真实浏览器页面自身 POST 也 403、前端拦截 UI 发送）——非签名问题。

## 上游维护

- chunk 来源：grok.com `_next/static/chunks/*.js`（Turbopack），模块 id 1645e3 / wrapper 4629918
- 重生成：参考 /tmp/sandtest/build_node_bundle.py（生成器）+ 解密工具（字符串表 t/n 暴露法）
- 上游演化：moduleId 与混淆每轮变化——失效时按上述路径重提取
