#!/usr/bin/env python3
"""Patch grok_sign_standalone.js to load module from external chunk file."""
from __future__ import annotations

from pathlib import Path

BUNDLE = Path(__file__).resolve().parents[1] / "crates" / "grok-signer" / "assets" / "grok_sign_standalone.js"
MARK = "// 1645e3 模块 factory"


def main() -> None:
    text = BUNDLE.read_text(encoding="utf-8")
    idx = text.find(MARK)
    if idx < 0:
        raise SystemExit("marker not found")
    head = """// grok_sign_standalone.js —— grok.com 签名器模块 1645e3 自包含执行产物 (node 直接 run)
// 模块源码：同目录 grok_sign_module_1645e3.js（live chunk 1nf91r5--cp6_.js / wrapper 4629918）
// 用法: 替换 __GROK_META__ / __SIGN_PATH__ / __SIGN_METHOD__ 后 node 执行；结果写 globalThis.__signOut

const fs = require('fs');
const vm = require('vm');
const path = require('path');
const nodeCrypto = require('crypto');

const LOG = process.argv[2] || 'access.log';
const MODULE_FILE = path.join(__dirname, 'grok_sign_module_1645e3.js');
const src = fs.readFileSync(MODULE_FILE, 'utf8').replace(/\\s+/g, ' ').trim();

"""
    tail = text[idx:]
    tail = tail.replace(
        "const RET = 'return async(W,n)=>{';",
        """function findReturnAsync(body) {
  for (const pat of ['return async(n,t)=>{', 'return async(W,n)=>{']) {
    const idx = body.indexOf(pat);
    if (idx >= 0) return idx;
  }
  return -1;
}
const retIdx = findReturnAsync(body);
""",
    )
    # remove duplicate retIdx lines if present
    tail = tail.replace("const retIdx = body.indexOf(RET);\n", "")
    tail = tail.replace("const RET = 'return async(W,n)=>{';\n", "")
    if "findReturnAsync" not in tail:
        raise SystemExit("failed to patch return async locator")
    BUNDLE.write_text(head + tail, encoding="utf-8")
    print("patched", BUNDLE)


if __name__ == "__main__":
    main()
