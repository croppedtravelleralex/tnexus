"""Playwright 探测共享常量（免 curl_cffi / canary 全量导入）。"""
from __future__ import annotations

import os

UA = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36"
)
PROXY = os.environ.get("GROK_LOCAL_PROXY", "http://127.0.0.1:7897")

TURBOPACK_HOOK = r"""
(() => {
  const queue = [];
  const nativePush = Array.prototype.push;
  const wrap = entry => {
    if (!Array.isArray(entry) || typeof entry[entry.length - 1] !== 'function') return entry;
    const original = entry[entry.length - 1];
    entry[entry.length - 1] = function(...args) {
      if (args[0]) globalThis.__grokBridgeRuntime = args[0];
      return original.apply(this, args);
    };
    return entry;
  };
  queue.push = function(...entries) { return nativePush.apply(this, entries.map(wrap)); };
  globalThis.TURBOPACK = queue;
})();
"""

CAPTURE_HOOK = r"""
(() => {
  const captured = [];
  globalThis.__grokCapturedSigs = captured;
  const grab = (headers, url, method) => {
    if (!headers) return;
    let sig = '';
    try {
      if (typeof Headers !== 'undefined' && headers instanceof Headers) sig = headers.get('x-statsig-id') || '';
      else if (Array.isArray(headers)) {
        const hit = headers.find(h => String(h[0]).toLowerCase() === 'x-statsig-id');
        sig = hit ? String(hit[1]) : '';
      } else if (typeof headers === 'object') {
        for (const [k, v] of Object.entries(headers)) {
          if (String(k).toLowerCase() === 'x-statsig-id') { sig = String(v); break; }
        }
      }
    } catch (_) {}
    if (!sig) return;
    try {
      captured.push({ t: Date.now(), url: String(url || ''), method: String(method || ''), sig });
      if (captured.length > 200) captured.splice(0, captured.length - 200);
    } catch (_) {}
  };
  const origFetch = globalThis.fetch;
  if (typeof origFetch === 'function') {
    globalThis.fetch = function(input, init) {
      try {
        const url = typeof input === 'string' ? input : (input && input.url) || '';
        const method = (init && init.method) || 'GET';
        const h = (init && init.headers) || (input && input.headers);
        grab(h, url, method);
      } catch (_) {}
      return origFetch.apply(this, arguments);
    };
  }
})();
"""


def chat_payload(message: str, *, enable_image: bool = False) -> dict:
    return {
        "collectionIds": [],
        "disabledConnectorIds": [],
        "deviceEnvInfo": {
            "darkModeEnabled": False,
            "devicePixelRatio": 2,
            "screenHeight": 1328,
            "screenWidth": 2056,
            "viewportHeight": 1083,
            "viewportWidth": 2056,
        },
        "disableMemory": True,
        "disableSearch": False,
        "disableSelfHarmShortCircuit": False,
        "disableTextFollowUps": False,
        "enableImageGeneration": enable_image,
        "enableImageStreaming": enable_image,
        "enableSideBySide": True,
        "fileAttachments": [],
        "forceConcise": False,
        "forceSideBySide": False,
        "imageAttachments": [],
        "imageGenerationCount": 2 if enable_image else 0,
        "isAsyncChat": False,
        "message": message,
        "modeId": "fast",
        "responseMetadata": {},
        "returnImageBytes": False,
        "returnRawGrokInXaiRequest": False,
        "sendFinalMetadata": True,
        "temporary": True,
    }
