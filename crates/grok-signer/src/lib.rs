//! 本地 JS 签名器执行引擎（rquickjs + vendored quickjs）。
//!
//! 纯 HTTP 无 chrome 签名：执行 grok.com 前端 Turbopack 模块（moduleId 4629918）
//! 的 signer 函数，在本地 JS 引擎中产出 `x-statsig-id`，替代外部 signer 服务。
//!
//! bundle 输出约定：被执行的 JS 将结果写入 `globalThis.__signOut`（字符串），
//! 本 crate 执行后读取该全局属性返回。

use rquickjs::{Context, Runtime};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// 签名执行默认超时（秒）：引擎无内建 timeout，用线程 + channel 脱逃。
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(5);

/// 签名执行错误。
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("js 执行超时（> {0:?}）")]
    Timeout(Duration),
    #[error("js 引擎错误: {0}")]
    Engine(String),
    #[error("bundle 未写出 __signOut（或非字符串）")]
    NoOutput,
}

/// 独立签名 bundle 资产路径：`crates/grok-signer/assets/grok_sign_standalone.js`。
/// 运行时装（std::fs），文件缺失不导致编译失败。
pub fn asset_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("grok_sign_standalone.js")
}

/// 独立 bundle 资产是否就绪。
pub fn has_asset() -> bool {
    std::fs::metadata(asset_path())
        .map(|m| m.is_file())
        .unwrap_or(false)
}

/// 读取独立 bundle 内容（若存在）。
pub fn load_asset() -> Option<String> {
    std::fs::read_to_string(asset_path()).ok()
}

/// 独立 bundle 里的 path/method 占位标记（执行前替换为转义后真实值）。
pub const SIGN_PATH_PLACEHOLDER: &str = "__SIGN_PATH__";
pub const SIGN_METHOD_PLACEHOLDER: &str = "__SIGN_METHOD__";

/// 假 signer bundle（模拟 grok 模块 4629918）：default() 返回 signer 函数，
/// signer(path, method) 返回固定格式 id，写 `globalThis.__signOut`。
/// 用于验证接口约定与单测（fake mode）。
pub const FAKE_SIGNER_BUNDLE: &str = r#"
(function (path, method) {
  var hash = function (s) {
    var h = 5381;
    for (var i = 0; i < s.length; i++) { h = ((h << 5) + h + s.charCodeAt(i)) | 0; }
    return (h >>> 0).toString(36);
  };
  var signer = function (p, m) {
    return 'x0:' + hash(p + '|' + m) + ':' + new Date().getTime().toString(36);
  };
  globalThis.__signOut = signer(path, method);
})(__SIGN_PATH__, __SIGN_METHOD__);
"#;

/// 求值 `js`（5s 超时）。返回值忽略；主要用于引擎自检。
pub fn eval_stdout(js: &str) -> Result<String, SignError> {
    let js = js.to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(run_eval(&js));
    });
    rx.recv_timeout(EXEC_TIMEOUT)
        .map_err(|_| SignError::Timeout(EXEC_TIMEOUT))?
}

/// 执行签名 bundle：预期 JS 将结果写入 `globalThis.__signOut`（字符串）。
/// 5s 超时（死循环被线程脱逃杀掉）。
pub fn execute_signature_bundle(js: &str) -> Result<String, SignError> {
    let js = js.to_string();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(run_signature_bundle(&js));
    });
    rx.recv_timeout(EXEC_TIMEOUT)
        .map_err(|_| SignError::Timeout(EXEC_TIMEOUT))?
}

/// 执行独立签名 bundle（带 path/method）：`__SIGN_PATH__`/`__SIGN_METHOD__`
/// 占位替换为转义后的真实值再 eval。
pub fn execute_standalone_bundle(js: &str, path: &str, method: &str) -> Result<String, SignError> {
    let js = js
        .replace(SIGN_PATH_PLACEHOLDER, &js_string_escape(path))
        .replace(SIGN_METHOD_PLACEHOLDER, &js_string_escape(method));
    execute_signature_bundle(&js)
}

/// 把任意字符串转义为 JS 字符串字面量（嵌入 bundle 前防注入/破坏语法）。
fn js_string_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn run_eval(js: &str) -> Result<String, SignError> {
    let runtime = Runtime::new().map_err(engine_err)?;
    let context = Context::full(&runtime).map_err(engine_err)?;
    context
        .with(|ctx| ctx.eval::<(), _>(js))
        .map_err(engine_err)?;
    Ok(String::new())
}

fn run_signature_bundle(js: &str) -> Result<String, SignError> {
    let runtime = Runtime::new().map_err(engine_err)?;
    let context = Context::full(&runtime).map_err(engine_err)?;
    context.with(|ctx| {
        // eval 阶段错误（语法/运行时 throw）→ 引擎错误。
        if let Err(e) = ctx.eval::<(), _>(js) {
            return Err(engine_err(e));
        }
        let globals = ctx.globals();
        let v = match globals.get::<_, rquickjs::Value>("__signOut") {
            Ok(v) => v,
            Err(_) => return Err(SignError::NoOutput),
        };
        match v.type_of() {
            rquickjs::Type::String => match v.as_string().and_then(|s| s.to_string().ok()) {
                Some(t) => Ok(t),
                None => Err(SignError::NoOutput),
            },
            rquickjs::Type::Undefined | rquickjs::Type::Null => Err(SignError::NoOutput),
            _ => Err(SignError::NoOutput),
        }
    })
}

fn engine_err(e: impl ToString) -> SignError {
    SignError::Engine(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_hello() {
        assert!(eval_stdout("var x = 1 + 1;").is_ok());
    }

    #[test]
    fn fake_bundle_signs() {
        let out = execute_standalone_bundle(FAKE_SIGNER_BUNDLE, "/hello/sign", "POST").unwrap();
        assert!(out.starts_with("x0:"), "got {out}");
    }

    #[test]
    fn placeholder_substitution_escapes_payload() {
        // path 含引号/反斜杠不应破坏 bundle 语法或注入。
        let out = execute_standalone_bundle(FAKE_SIGNER_BUNDLE, "/a'b\\c", "POST").unwrap();
        assert!(out.starts_with("x0:"), "got {out}");
    }

    #[test]
    fn error_js_propagates() {
        let r = execute_signature_bundle("throw new Error('boom')");
        assert!(matches!(r, Err(SignError::Engine(_))));
    }

    #[test]
    fn timeout_kills_infinite_loop() {
        let r = execute_signature_bundle("for(;;){}");
        assert!(matches!(r, Err(SignError::Timeout(_))));
    }

    #[test]
    fn missing_output_is_no_output() {
        let r = execute_signature_bundle("var x = 1;");
        assert!(matches!(r, Err(SignError::NoOutput)));
    }

    #[test]
    fn asset_api_graceful_without_file() {
        // 无资产文件时优雅返回 false/None（不 panic、不编译失败）。
        let _ = has_asset();
        let _ = load_asset();
    }
}
