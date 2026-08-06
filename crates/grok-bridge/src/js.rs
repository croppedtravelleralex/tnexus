//! 浏览器内 JS 片段（语言无关资产，逻辑照搬 Python bridge 的
//! `SIGN_SCRIPT` / fetch script / websocket script，但改为 Promise 返回式，
//! 适配 CDP `Runtime.evaluate` + `awaitPromise`）。
//!
//! 维护点：`SIGNER_MODULE_ID`（grok.com Turbopack 签名器模块号）随上游前端演化，
//! 经 `BRIDGE_SIGNER_MODULE_ID` env 覆盖，缺省 4629918（与 Python 版一致）。

/// 缺省 grok.com Turbopack 签名器模块号。
pub const SIGNER_MODULE_ID: &str = "4629918";

/// 解析 env 覆盖的签名器模块号。
pub fn signer_module_id() -> u64 {
    std::env::var("BRIDGE_SIGNER_MODULE_ID")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| SIGNER_MODULE_ID.parse().unwrap())
}

/// 会话就绪探测表达式（Turbopack runtime 加载 + body 子节点 ≥3）。
pub const READY_EXPR: &str =
    "!!(globalThis.__grokBridgeRuntime && document.body && document.body.childNodes && document.body.childNodes.length >= 3)";

/// 签名脚本：等待 runtime → 捕获 fetch 发出的 x-statsig-id → 兜底模块签名器。
/// 返回 `{statsigId?, path, method, source?, signerModuleId?, error?, ...}`。
pub fn sign_script() -> &'static str {
    r#"(async () => {
  const cfg = {path, method, timeoutMs, signerModuleId};
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const looksGood = sig => {
    const value = String(sig || '').trim();
    return value.length > 20 && !value.startsWith('x0:') && !value.startsWith('eDA6');
  };
  const trySign = async (moduleId) => {
    const signerModule = await globalThis.__grokBridgeRuntime.A(moduleId);
    if (!signerModule || typeof signerModule.default !== 'function') {
      throw new Error('module has no default factory');
    }
    const signer = signerModule.default();
    if (typeof signer !== 'function') throw new Error('default() did not return signer');
    const statsigId = await signer(cfg.path, cfg.method);
    if (!looksGood(statsigId)) throw new Error('empty or fallback statsig id');
    globalThis.__grokBridgeSigner = signer;
    globalThis.__grokBridgeSignerModuleId = moduleId;
    return String(statsigId).trim();
  };
  try {
    const deadline = Date.now() + Math.max(8000, (cfg.timeoutMs || 30000) - 2000);
    while (Date.now() < deadline) {
      if (globalThis.__grokBridgeRuntime && document.body && document.body.childNodes.length >= 3) break;
      await sleep(250);
    }
    if (!globalThis.__grokBridgeRuntime) throw new Error('Turbopack runtime unavailable');
    if (!document.body || document.body.childNodes.length < 3) {
      throw new Error('Grok DOM not ready for signer');
    }
    await sleep(2500);

    const captured = [];
    const note = (sig) => { if (looksGood(sig)) captured.push(String(sig).trim()); };
    try {
      const origSet = Headers.prototype.set;
      Headers.prototype.set = function(name, value) {
        if (String(name).toLowerCase() === 'x-statsig-id') note(value);
        return origSet.apply(this, arguments);
      };
      const origAppend = Headers.prototype.append;
      Headers.prototype.append = function(name, value) {
        if (String(name).toLowerCase() === 'x-statsig-id') note(value);
        return origAppend.apply(this, arguments);
      };
    } catch (_) {}
    const origFetch = window.fetch.bind(window);
    window.fetch = async (input, init = {}) => {
      try {
        const headers = init && init.headers;
        if (headers instanceof Headers) note(headers.get('x-statsig-id'));
        else if (headers && typeof headers === 'object') note(headers['x-statsig-id'] || headers['X-Statsig-Id']);
      } catch (_) {}
      return origFetch(input, init);
    };
    for (const path of ['/rest/rate-limits', cfg.path]) {
      try {
        await origFetch(path, {
          method: 'POST',
          credentials: 'include',
          headers: {'content-type': 'application/json'},
          body: JSON.stringify(path === cfg.path
            ? {temporary: true, message: 'ping', modeId: 'fast'}
            : {requestKind: 'CLIENT_STATE_UPDATE'}),
        });
      } catch (_) {}
      if (captured.length) break;
      await sleep(500);
    }
    if (captured.length) {
      return {statsigId: captured[0], path: cfg.path, method: cfg.method, source: 'fetch-capture', signerModuleId: cfg.signerModuleId};
    }

    const preferred = Number(cfg.signerModuleId) || 4629918;
    try {
      const statsigId = await trySign(preferred);
      return {statsigId, path: cfg.path, method: cfg.method, source: 'module', signerModuleId: preferred};
    } catch (error) {
      return {error: String(error && (error.stack || error.message) || error).slice(0, 800), signerModuleId: preferred, source: 'module-failed'};
    }
  } catch (error) {
    return {error: String(error && (error.stack || error.message) || error).slice(0, 1200)};
  }
})()"#
}

/// 浏览器内 fetch：/rest/* 自动附加 x-statsig-id；body 为 base64。
/// 返回 `{status, headers, body(b64), error?}`。
pub fn fetch_script() -> &'static str {
    r#"(async () => {
  const cfg = {url, method, headers, body, referer, timeoutMs, signerModuleId};
  try {
    const headers = new Headers();
    for (const [name, values] of Object.entries(cfg.headers || {})) {
      for (const value of values) headers.append(name, value);
    }
    const target = new URL(cfg.url);
    if ((target.hostname === 'grok.com' || target.hostname === 'www.grok.com') && target.pathname.startsWith('/rest/')) {
      let signature = '';
      try {
        if (!globalThis.__grokBridgeRuntime) throw new Error('Turbopack runtime unavailable');
        if (!globalThis.__grokBridgeSigner) {
          const signerModule = await globalThis.__grokBridgeRuntime.A(cfg.signerModuleId);
          globalThis.__grokBridgeSigner = signerModule.default();
        }
        signature = await globalThis.__grokBridgeSigner(target.pathname, cfg.method);
      } catch (signerError) {
        signature = btoa(`x0:${signerError}`);
      }
      headers.set('x-statsig-id', signature);
    }
    const init = {method: cfg.method, headers, credentials: 'include', cache: 'no-store'};
    if (cfg.referer) init.referrer = cfg.referer;
    if (cfg.body && !['GET', 'HEAD'].includes(cfg.method)) {
      const binary = atob(cfg.body), bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      init.body = bytes;
    }
    const result = await fetch(cfg.url, init);
    const blob = await result.blob();
    const reader = new FileReader();
    const dataUrl = await new Promise((resolve, reject) => {
      reader.onerror = () => reject(new Error('response encoding failed'));
      reader.onload = () => resolve(String(reader.result));
      reader.readAsDataURL(blob);
    });
    const headersOut = {};
    result.headers.forEach((value, name) => { headersOut[name] = [value]; });
    return {status: result.status, headers: headersOut, body: String(dataUrl).split(',', 2)[1] || ''};
  } catch (error) {
    return {error: String(error && (error.stack || error.message) || error).slice(0, 1200)};
  }
})()"#
}

/// 浏览器内 WebSocket：发送 messages、收集 frames、completed 计数、idle 收尾。
/// 返回 `{frames: [string], error?}`（frame 为文本，服务端再 b64 编码）。
pub fn websocket_script() -> &'static str {
    r#"(async () => {
  const cfg = {url, messages, timeoutMs, idleMs, expected};
  return await new Promise((resolve) => {
    let frames = [], completed = new Set(), finished = false, idleTimer = null;
    const finish = error => {
      if (finished) return;
      finished = true;
      clearTimeout(timeoutTimer); clearTimeout(idleTimer);
      try { socket.close(); } catch (_) {}
      resolve({frames, error: error || ''});
    };
    const scheduleFinish = () => {
      clearTimeout(idleTimer);
      if (completed.size >= cfg.expected) idleTimer = setTimeout(() => finish(''), cfg.idleMs);
    };
    const timeoutTimer = setTimeout(() => finish('browser websocket timeout'), cfg.timeoutMs);
    const socket = new WebSocket(cfg.url);
    socket.onopen = () => { for (const message of cfg.messages || []) socket.send(JSON.stringify(message)); };
    socket.onerror = () => finish('browser websocket error');
    socket.onclose = event => { if (!finished && completed.size < cfg.expected) finish('browser websocket closed: ' + event.code); };
    socket.onmessage = event => {
      const value = String(event.data); frames.push(value);
      try {
        const parsed = JSON.parse(value);
        if (parsed.type === 'error') return finish('upstream websocket error');
        if (parsed.current_status === 'completed' || parsed.currentStatus === 'completed') {
          completed.add(String(parsed.image_id || parsed.imageId || parsed.id || completed.size));
        }
      } catch (_) {}
      scheduleFinish();
    };
  });
})()"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_self_contained() {
        for s in [sign_script(), fetch_script(), websocket_script()] {
            assert!(s.contains("(async () =>"), "async IIFE");
            assert!(s.ends_with("})()"), "returns promise value");
        }
    }

    #[test]
    fn signer_module_id_parses_env_or_default() {
        std::env::remove_var("BRIDGE_SIGNER_MODULE_ID");
        assert_eq!(signer_module_id(), 4629918);
        std::env::set_var("BRIDGE_SIGNER_MODULE_ID", "123");
        assert_eq!(signer_module_id(), 123);
        std::env::remove_var("BRIDGE_SIGNER_MODULE_ID");
    }
}
