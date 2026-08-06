#!/usr/bin/env node
// grok OCR 探测：真实链路验证（需 :8000 + 账号池 + browser-bridge 就绪）。
// 用法：
//   node scripts/grok_ocr_probe.mjs --url http://127.0.0.1:8000 --key <GATEWAY_AUTH_KEY> --image web/public/icon.png
//   （默认 --text "提取图中文字" --model grok-vision-ocr --stream false）
// 预期：200 + 返回识别文本；503 = 空池（无 grok_web 账号）；401 = key 错误；500/502 = bridge 未就绪。

import { readFileSync } from "node:fs";
import { basename } from "node:path";

function arg(name, def) {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 && process.argv[i + 1] ? process.argv[i + 1] : def;
}

const base = arg("url", "http://127.0.0.1:8000");
const key = arg("key", "");
const image = arg("image", "");
const text = arg("text", "提取图中文字");
const model = arg("model", "grok-vision-ocr");
const stream = arg("stream", "false") === "true";

if (!image) {
  console.error("缺 --image <图片路径>");
  process.exit(2);
}

const bytes = readFileSync(image);
const mime = basename(image).toLowerCase().endsWith(".png") ? "image/png" : "image/jpeg";
const content = [
  { type: "image_url", image_url: { url: `data:${mime};base64,${bytes.toString("base64")}` } },
  { type: "text", text },
];

const resp = await fetch(`${base}/v1/chat/completions`, {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    ...(key ? { Authorization: `Bearer ${key}` } : {}),
  },
  body: JSON.stringify({ model, messages: [{ role: "user", content }], stream }),
});

const body = await resp.text();
console.log(`status=${resp.status}`);
console.log(body.slice(0, 2000));
process.exit(resp.status === 200 ? 0 : 1);
