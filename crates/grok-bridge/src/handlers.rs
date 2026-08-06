//! axum 端点：`/health`、`/v1/sign`、`/v1/fetch`、`/v1/websocket`（对齐 Python bridge 协议）。

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth;
use crate::js;
use crate::session::{apply_cookies, ensure_navigated, wait_ready, Session, SessionPool};
use url::Url;

/// 允许的目标 host（对齐 Python `ALLOWED_HOSTS`）。
const ALLOWED_HOSTS: [&str; 3] = ["grok.com", "www.grok.com", "assets.grok.com"];

/// bridge 共享状态。
#[derive(Clone)]
pub struct BridgeState {
    pub pool: Arc<SessionPool>,
    /// 鉴权 key（空 = 未配置）。
    pub key: Arc<String>,
}

/// 构建路由（`key` 缺省从 env `GROK_BRIDGE_KEY` 读取）。
pub fn build_router(state: BridgeState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/sign", post(sign))
        .route("/v1/fetch", post(fetch))
        .route("/v1/websocket", post(websocket))
        .with_state(state)
}

/// `GET /health`：不鉴权。
async fn health(State(state): State<BridgeState>) -> impl IntoResponse {
    let sessions = state.pool.session_count().await;
    Json(json!({ "status": "ok", "sessions": sessions }))
}

/// 鉴权检查：非 /health 必须带 `Bearer <key>`；key 未配置 → 一律 401。
#[allow(clippy::result_large_err)] // axum Response 本身即大类型（128B），此处不可避免
fn require_auth(headers: &HeaderMap, state: &BridgeState) -> Result<(), axum::response::Response> {
    let key = state.key.as_str();
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if auth::authorized(header, key) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response())
    }
}

fn bad_request(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
}

fn upstream_error(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_GATEWAY, Json(json!({ "error": msg }))).into_response()
}

// ── /v1/sign ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SignRequest {
    #[serde(default)]
    method: Option<String>,
    path: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    session_key: Option<String>,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    timeout_ms: Option<i64>,
    #[serde(default)]
    light_bootstrap: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignResponse {
    statsig_id: String,
    path: String,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signer_module_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cookie: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cookie_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_cf_clearance: Option<bool>,
}

async fn sign(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(payload): Json<SignRequest>,
) -> axum::response::Response {
    if let Err(resp) = require_auth(&headers, &state) {
        return resp;
    }
    let method = payload
        .method
        .unwrap_or_else(|| "POST".into())
        .to_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return bad_request("invalid method");
    }
    let path = payload.path.unwrap_or_default().trim().to_string();
    if !path.starts_with("/rest/") {
        return bad_request("invalid path");
    }
    let timeout_ms = bounded(payload.timeout_ms, 120_000);
    let cookie = payload.cookie.unwrap_or_default();
    let session_key = payload.session_key.unwrap_or_else(|| "sign-only".into());
    let session_key = &session_key[..session_key.len().min(128)];
    let user_agent = payload.user_agent.unwrap_or_default();
    let proxy_url = payload.proxy_url.unwrap_or_default();
    // light_bootstrap：缺省 true（复用已过 CF 的 cookie 直接导航，与 Python 版生产主路径一致）。
    // 会话键派生已含 proxy；单 Chrome 实例不应用代理（如实标注：代理注入为后续项）。
    let _ = (proxy_url, payload.light_bootstrap);

    let session = match acquire_for(&state, session_key, &user_agent, &cookie).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    // 确保 grok.com 就绪（light_bootstrap 语义：已过 CF 的 cookie 直接导航）。
    if let Err(resp) = prepare_grok_page(&session, &cookie).await {
        return resp;
    }

    let expression = js::sign_script();
    let cfg = json!({
        "path": path,
        "method": method,
        "timeoutMs": timeout_ms,
        "signerModuleId": js::signer_module_id(),
    });
    // 注入 cfg 到表达式：CDP evaluate 无 arguments 通道，把 cfg 拼进表达式作用域。
    let expression = inject_cfg(expression, &cfg);
    let result = match session.client.evaluate(&expression, true).await {
        Ok(v) => v,
        Err(e) => return upstream_error(&format!("browser sign failed: {e}")),
    };

    let Some(statsig_id) = result.get("statsigId").and_then(Value::as_str) else {
        let detail = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("no statsigId in result");
        return upstream_error(&format!("statsig signature unavailable: {detail}"));
    };
    let statsig_id = statsig_id.trim().to_string();
    if statsig_id.is_empty() || statsig_id.starts_with("eDA6") || statsig_id.starts_with("x0:") {
        return upstream_error("statsig signature unavailable (fallback id)");
    }

    // 导出 cookie jar（对齐 Python 版 sign 响应）。
    let mut resp = SignResponse {
        statsig_id,
        path: path.clone(),
        method: method.clone(),
        source: result
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string),
        signer_module_id: result.get("signerModuleId").and_then(Value::as_u64),
        cookie: None,
        cookie_names: None,
        has_cf_clearance: None,
    };
    match session.client.get_cookies().await {
        Ok(cookies) if !cookies.is_empty() => {
            let names: Vec<String> = cookies.iter().map(|c| c.name.clone()).collect();
            let joined = cookies
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>()
                .join("; ");
            resp.cookie = Some(joined);
            resp.cookie_names = Some(names.clone());
            resp.has_cf_clearance = Some(names.iter().any(|n| n == "cf_clearance"));
        }
        _ => {}
    }
    (StatusCode::OK, Json(resp)).into_response()
}

// ── /v1/fetch ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FetchRequest {
    #[serde(default)]
    session_key: Option<String>,
    url: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: Option<Value>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    timeout_ms: Option<i64>,
}

async fn fetch(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(payload): Json<FetchRequest>,
) -> axum::response::Response {
    if let Err(resp) = require_auth(&headers, &state) {
        return resp;
    }
    let url = payload.url.unwrap_or_default().trim().to_string();
    if let Err(msg) = validate_target(&url, &["https"]) {
        return bad_request(&msg);
    }
    let method = payload
        .method
        .unwrap_or_else(|| "GET".into())
        .to_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
        return bad_request("invalid method");
    }
    let timeout_ms = bounded(payload.timeout_ms, 120_000);
    let cookie = payload.cookie.unwrap_or_default();
    let session_key = payload.session_key.unwrap_or_default();
    let session_key = &session_key[..session_key.len().min(128)];
    let user_agent = payload.user_agent.unwrap_or_default();
    let referer = payload.referer.unwrap_or_default();
    let _ = payload.proxy_url; // 代理注入为后续项（单 Chrome 实例）。

    let session = match acquire_for(&state, session_key, &user_agent, &cookie).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(resp) = prepare_grok_page(&session, &cookie).await {
        return resp;
    }
    let _ = ensure_navigated(&session, &url).await;

    let expression = inject_cfg(
        js::fetch_script(),
        &json!({
            "url": url,
            "method": method,
            "headers": payload.headers.unwrap_or(Value::Object(Default::default())),
            "body": payload.body.unwrap_or_default(),
            "referer": referer,
            "timeoutMs": timeout_ms,
            "signerModuleId": js::signer_module_id(),
        }),
    );
    let result = match session.client.evaluate(&expression, true).await {
        Ok(v) => v,
        Err(e) => return upstream_error(&format!("browser fetch failed: {e}")),
    };
    if let Some(err) = result.get("error").and_then(Value::as_str) {
        if !err.is_empty() {
            return (StatusCode::BAD_GATEWAY, Json(json!({ "error": err }))).into_response();
        }
    }
    (StatusCode::OK, Json(result)).into_response()
}

// ── /v1/websocket ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WebSocketRequest {
    #[serde(default)]
    session_key: Option<String>,
    url: Option<String>,
    #[serde(default)]
    messages: Option<Vec<Value>>,
    #[serde(default)]
    cookie: Option<String>,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    timeout_ms: Option<i64>,
    #[serde(default)]
    idle_ms: Option<i64>,
    #[serde(default)]
    expected: Option<i64>,
}

async fn websocket(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(payload): Json<WebSocketRequest>,
) -> axum::response::Response {
    if let Err(resp) = require_auth(&headers, &state) {
        return resp;
    }
    let url = payload.url.unwrap_or_default().trim().to_string();
    if let Err(msg) = validate_target(&url, &["wss"]) {
        return bad_request(&msg);
    }
    let timeout_ms = bounded(payload.timeout_ms, 180_000);
    let idle_ms = payload.idle_ms.unwrap_or(5000).clamp(500, 30_000);
    let expected = payload.expected.unwrap_or(1).clamp(1, 10);
    let cookie = payload.cookie.unwrap_or_default();
    let session_key = payload.session_key.unwrap_or_default();
    let session_key = &session_key[..session_key.len().min(128)];
    let user_agent = payload.user_agent.unwrap_or_default();
    let referer = payload
        .referer
        .unwrap_or_else(|| "https://grok.com/imagine".into());
    let _ = payload.proxy_url; // 代理注入为后续项（单 Chrome 实例）。

    let session = match acquire_for(&state, session_key, &user_agent, &cookie).await {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(resp) = prepare_grok_page(&session, &cookie).await {
        return resp;
    }
    let _ = ensure_navigated(&session, &referer).await;

    let expression = inject_cfg(
        js::websocket_script(),
        &json!({
            "url": url,
            "messages": payload.messages.unwrap_or_default(),
            "timeoutMs": timeout_ms,
            "idleMs": idle_ms,
            "expected": expected,
        }),
    );
    let result = match session.client.evaluate(&expression, true).await {
        Ok(v) => v,
        Err(e) => return upstream_error(&format!("browser websocket failed: {e}")),
    };
    let frames: Vec<String> = result
        .get("frames")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f.as_str())
                .map(|s| base64::engine::general_purpose::STANDARD.encode(s))
                .collect()
        })
        .unwrap_or_default();
    let error = result
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    (
        StatusCode::OK,
        Json(json!({ "frames": frames, "error": error })),
    )
        .into_response()
}

// ── helpers ──────────────────────────────────────────────────────

fn bounded(raw: Option<i64>, max: i64) -> i64 {
    raw.unwrap_or(30_000).clamp(1000, max)
}

/// 校验 URL scheme + host（对齐 Python `validate_target`）。
fn validate_target(raw: &str, schemes: &[&str]) -> Result<(), String> {
    let parsed = Url::parse(raw).map_err(|_| "invalid target".to_string())?;
    if !schemes.contains(&parsed.scheme()) {
        return Err("invalid target".into());
    }
    let host = parsed.host_str().unwrap_or("");
    if !ALLOWED_HOSTS.contains(&host) {
        return Err("invalid target".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("invalid target".into());
    }
    Ok(())
}

/// 取会话（应用 cookie 前先标记导航态，便于首次 navigate）。
async fn acquire_for(
    state: &BridgeState,
    session_key: &str,
    user_agent: &str,
    cookie: &str,
) -> Result<Arc<Session>, axum::response::Response> {
    let session = state
        .pool
        .acquire(session_key, user_agent)
        .await
        .map_err(|e| upstream_error(&format!("session acquire failed: {e}")))?;
    if let Err(e) = apply_cookies(&session, cookie).await {
        return Err(upstream_error(&format!("cookie apply failed: {e}")));
    }
    Ok(session)
}

/// 准备 grok.com 页面（light_bootstrap 语义：仅首次导航 + 轮询 runtime 就绪）。
async fn prepare_grok_page(
    session: &Session,
    _cookie: &str,
) -> Result<(), axum::response::Response> {
    if session.is_navigated() {
        return Ok(());
    }
    session
        .client
        .navigate("https://grok.com/")
        .await
        .map_err(|e| upstream_error(&format!("navigate failed: {e}")))?;
    session.mark_navigated();
    // 轮询 runtime 就绪（非致命：签名脚本内部自己也会等）。
    let _ = wait_ready(session, Duration::from_secs(10)).await;
    Ok(())
}

/// 把 cfg JSON 注入 JS 表达式作用域（CDP evaluate 无参数通道）。
fn inject_cfg(script: &str, cfg: &Value) -> String {
    format!("({{ const cfg = {cfg}; return {script}; }})()")
}
