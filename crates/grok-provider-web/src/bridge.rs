//! browser-bridge HTTP 客户端（docs/39d §8，39 主文档 §4.3）。
//!
//! 协议对齐 Go `browser_bridge.go`：`POST /v1/fetch`（浏览器内 fetch，自动附加
//! x-statsig-id）→ `{status, headers, body(base64), error}`。
//!
//! G1 允许 mock：`BridgeClient` trait 抽象了「从可注入 client 构造」，测试用
//! [`MockBridgeClient`] 直插，不发起真实 HTTP。地址取 `GROK2API_BROWSER_BRIDGE_URL`
//! 环境变量，默认 `http://browser-bridge:8192`；鉴权 key 取 `GROK2API_BRIDGE_KEY`。

use reqwest::{Client, Method};
use serde_json::Value;

use base64::Engine;
use grok_domain::ProviderError;

/// bridge 基地址。
pub fn default_bridge_url() -> String {
    std::env::var("GROK2API_BROWSER_BRIDGE_URL")
        .unwrap_or_else(|_| "http://browser-bridge:8192".to_string())
}

/// 上游 chat 端点（对齐 Go `cfg.BaseURL + /rest/app-chat/conversations/new`）。
pub fn default_chat_url() -> String {
    std::env::var("GROK2API_WEB_CHAT_URL")
        .unwrap_or_else(|_| "https://grok.com/rest/app-chat/conversations/new".to_string())
}

/// 从会话上下文派生稳定 sessionKey（sha256 前 16 字节 hex，对齐 Go `browserSessionKey`）。
pub fn derive_session_key(cookie: &str, user_agent: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(cookie.as_bytes());
    hasher.update([0u8]);
    hasher.update(user_agent.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// browser-bridge 客户端抽象（便于单测注入 fake）。
///
/// `fetch_bytes`：POST /v1/fetch 下载一张远端 HTTPS 图片。
/// `fetch_chat`：POST /v1/fetch 转发上游 chat payload，返回最终文本。
#[async_trait::async_trait]
pub trait BridgeClient: Send + Sync {
    /// 下载图片二进制（经 bridge）。`image_url` 为 HTTPS URL 或 data URI。
    /// `sso_token`：无 chrome 直连路径用于鉴权；bridge 会话路径忽略。
    async fn fetch_bytes(
        &self,
        image_url: &str,
        sso_token: Option<&str>,
    ) -> Result<Vec<u8>, ProviderError>;

    /// 发送 chat 请求到 bridge，返回上游 SSE 汇总后的文本。
    /// `account_id`：直连模式用于加载 `pure_http_keys`（bridge 模式忽略）。
    async fn fetch_chat(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<String, ProviderError>;

    /// 发送生图（imagine）请求到 bridge，返回上游 JSON（含 data[].url / b64_json）。
    async fn fetch_imagine(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
        account_id: Option<i64>,
    ) -> Result<Value, ProviderError>;
}

/// 基于 `reqwest::Client` 的真实 bridge 客户端（协议对齐 Go `browser_bridge.go`）。
///
/// 会话上下文（sessionKey/cookie/UA）以字段携带：当前 trait 边界未透传账号详情，
/// 默认取 env（`GROK2API_BRIDGE_SESSION_KEY` / `GROK2API_BRIDGE_COOKIE` /
/// `GROK2API_BRIDGE_USER_AGENT`），sessionKey 缺省由 cookie+UA 派生键；后续若需按
/// 账号隔离，用 [`HttpBridgeClient::with_session_context`] 注入。
pub struct HttpBridgeClient {
    client: Client,
    base: String,
    key: String,
    session_key: String,
    cookie: String,
    user_agent: String,
    chat_url: String,
}

impl HttpBridgeClient {
    /// 用默认 `reqwest::Client` 与 env 配置构造。
    pub fn new() -> Self {
        let cookie = std::env::var("GROK2API_BRIDGE_COOKIE").unwrap_or_default();
        let user_agent = std::env::var("GROK2API_BRIDGE_USER_AGENT").unwrap_or_default();
        let session_key = std::env::var("GROK2API_BRIDGE_SESSION_KEY")
            .unwrap_or_else(|_| derive_session_key(&cookie, &user_agent));
        Self {
            client: Client::new(),
            base: default_bridge_url(),
            key: std::env::var("GROK2API_BRIDGE_KEY").unwrap_or_default(),
            session_key,
            cookie,
            user_agent,
            chat_url: default_chat_url(),
        }
    }

    /// 注入客户端与基地址（测试可指向 mock server）。
    pub fn with_client_and_base(client: Client, base: String) -> Self {
        Self {
            client,
            base,
            key: std::env::var("GROK2API_BRIDGE_KEY").unwrap_or_default(),
            session_key: derive_session_key("", ""),
            cookie: String::new(),
            user_agent: String::new(),
            chat_url: default_chat_url(),
        }
    }

    /// 注入会话上下文（按账号隔离时使用）。
    pub fn with_session_context(
        mut self,
        session_key: &str,
        cookie: &str,
        user_agent: &str,
    ) -> Self {
        self.session_key = session_key.to_string();
        self.cookie = cookie.to_string();
        self.user_agent = user_agent.to_string();
        self
    }
}

impl Default for HttpBridgeClient {
    fn default() -> Self {
        Self::new()
    }
}

/// bridge `/v1/fetch` 响应（对齐 Go `browserBridgeResponse`）。
#[derive(serde::Deserialize)]
struct FetchResult {
    #[serde(default)]
    status: u16,
    #[serde(default)]
    body: String,
    #[serde(default)]
    error: String,
}

impl HttpBridgeClient {
    /// 底层 `POST {base}/v1/fetch`，body 为协议 JSON（url/method/headers/body b64）。
    async fn post_fetch(
        &self,
        url: &str,
        method: &str,
        headers: Value,
        raw: &[u8],
    ) -> Result<FetchResult, ProviderError> {
        let payload = serde_json::json!({
            "sessionKey": self.session_key,
            "url": url,
            "method": method,
            "headers": headers,
            "body": base64::engine::general_purpose::STANDARD.encode(raw),
            "cookie": self.cookie,
            "proxyUrl": "",
            "userAgent": self.user_agent,
            "referer": "https://grok.com/",
            "timeoutMs": 60000,
        });
        let endpoint = format!("{}/v1/fetch", self.base.trim_end_matches('/'));
        let mut request = self.client.request(Method::POST, &endpoint).json(&payload);
        if !self.key.is_empty() {
            request = request.bearer_auth(&self.key);
        }
        let resp = request
            .send()
            .await
            .map_err(|e| ProviderError::Bridge(format!("connected to {endpoint}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(ProviderError::Bridge(format!(
                "bridge {status} for {endpoint}: {body}"
            )));
        }
        let value: FetchResult = resp
            .json()
            .await
            .map_err(|e| ProviderError::Bridge(format!("parse bridge response: {e}")))?;
        if !value.error.is_empty() {
            return Err(ProviderError::Bridge(value.error));
        }
        // 对齐 Go：状态码必须在 100..=599（bridge 可能透传上游非 2xx）。
        if !(100..=599).contains(&value.status) {
            return Err(ProviderError::Bridge(format!(
                "bridge returned invalid status {}",
                value.status
            )));
        }
        Ok(value)
    }
}

#[async_trait::async_trait]
impl BridgeClient for HttpBridgeClient {
    async fn fetch_bytes(
        &self,
        image_url: &str,
        _sso_token: Option<&str>,
    ) -> Result<Vec<u8>, ProviderError> {
        // data URI 本地直解（不经 bridge）：仅 HTTP(S) 图片走 bridge 下载。
        if let Some(payload) = decode_data_uri(image_url) {
            return Ok(payload);
        }
        let result = self
            .post_fetch(image_url, "GET", serde_json::json!({}), &[])
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&result.body)
            .map_err(|e| ProviderError::Bridge(format!("decode image body: {e}")))?;
        tracing::debug!(len = bytes.len(), url = image_url, "bridge fetched image");
        Ok(bytes)
    }

    async fn fetch_chat(
        &self,
        payload: &Value,
        _sso_token: Option<&str>,
        _account_id: Option<i64>,
    ) -> Result<String, ProviderError> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| ProviderError::Bridge(format!("serialize chat payload: {e}")))?;
        let result = self
            .post_fetch(
                &self.chat_url,
                "POST",
                serde_json::json!({ "content-type": ["application/json"] }),
                &body,
            )
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&result.body)
            .map_err(|e| ProviderError::Bridge(format!("decode chat body: {e}")))?;
        String::from_utf8(bytes)
            .map_err(|e| ProviderError::Bridge(format!("chat body not utf8: {e}")))
    }

    async fn fetch_imagine(
        &self,
        payload: &Value,
        _sso_token: Option<&str>,
        _account_id: Option<i64>,
    ) -> Result<Value, ProviderError> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| ProviderError::Bridge(format!("serialize imagine payload: {e}")))?;
        let result = self
            .post_fetch(
                &self.chat_url,
                "POST",
                serde_json::json!({ "content-type": ["application/json"] }),
                &body,
            )
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&result.body)
            .map_err(|e| ProviderError::Bridge(format!("decode imagine body: {e}")))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| ProviderError::Bridge(format!("parse imagine json: {e}")))
    }
}

/// 解码 `data:image/<type>;base64,<payload>`。非 data URI 返回 None。
pub fn decode_data_uri_public(value: &str) -> Option<Vec<u8>> {
    decode_data_uri(value)
}

/// 解码 `data:image/<type>;base64,<payload>`。非 data URI 返回 None。
fn decode_data_uri(value: &str) -> Option<Vec<u8>> {
    let rest = value.strip_prefix("data:")?;
    let encoded = rest.split_once(";base64,")?.1;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
}

/// 测试用的内存 fake：`fetch_bytes` 返回预置字节，`fetch_chat` 返回配置文本。
pub struct MockBridgeClient {
    /// 供 fetch_bytes 按 URL 返回的字节。
    pub images: std::collections::HashMap<String, Vec<u8>>,
    /// fetch_chat 统一返回的文本。
    pub chat_text: String,
    /// fetch_chat 收到的 payload（测试断言 golden）。
    pub last_chat_payload: tokio::sync::Mutex<Option<Value>>,
    /// fetch_imagine 统一返回的 JSON（默认空对象，空则无 data → 引擎报错）。
    pub imagine_response: Value,
    /// fetch_imagine 收到的 payload（测试断言 golden）。
    pub last_imagine_payload: tokio::sync::Mutex<Option<Value>>,
}

impl MockBridgeClient {
    pub fn new() -> Self {
        Self {
            images: std::collections::HashMap::new(),
            chat_text: String::new(),
            last_chat_payload: tokio::sync::Mutex::new(None),
            imagine_response: serde_json::Value::Null,
            last_imagine_payload: tokio::sync::Mutex::new(None),
        }
    }
}

impl Default for MockBridgeClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BridgeClient for MockBridgeClient {
    async fn fetch_bytes(
        &self,
        image_url: &str,
        _sso_token: Option<&str>,
    ) -> Result<Vec<u8>, ProviderError> {
        if let Some(b) = decode_data_uri(image_url) {
            return Ok(b);
        }
        self.images
            .get(image_url)
            .cloned()
            .ok_or_else(|| ProviderError::Bridge(format!("mock no bytes for {image_url}")))
    }

    async fn fetch_chat(
        &self,
        payload: &Value,
        _sso_token: Option<&str>,
        _account_id: Option<i64>,
    ) -> Result<String, ProviderError> {
        *self.last_chat_payload.lock().await = Some(payload.clone());
        Ok(self.chat_text.clone())
    }

    async fn fetch_imagine(
        &self,
        payload: &Value,
        _sso_token: Option<&str>,
        _account_id: Option<i64>,
    ) -> Result<Value, ProviderError> {
        *self.last_imagine_payload.lock().await = Some(payload.clone());
        Ok(self.imagine_response.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_decodes_to_bytes() {
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let bytes = decode_data_uri(uri).expect("decode");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn non_data_uri_is_none() {
        assert!(decode_data_uri("https://x.com/a.png").is_none());
    }

    #[test]
    fn session_key_is_stable_and_sensitive_to_input() {
        let a = derive_session_key("c1", "ua");
        let b = derive_session_key("c1", "ua");
        let c = derive_session_key("c2", "ua");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 32, "sha256 前 16 字节 hex");
    }

    #[tokio::test]
    async fn mock_fetch_chat_records_payload() {
        let mut m = MockBridgeClient::new();
        m.chat_text = "ok".to_string();
        let p = serde_json::json!({"model": "grok-chat-fast"});
        let out = m.fetch_chat(&p, None, None).await.unwrap();
        assert_eq!(out, "ok");
        let got = m.last_chat_payload.lock().await;
        assert_eq!(got.as_ref().unwrap()["model"], "grok-chat-fast");
    }

    #[test]
    fn http_client_derives_session_key_when_unset() {
        std::env::remove_var("GROK2API_BRIDGE_SESSION_KEY");
        std::env::remove_var("GROK2API_BRIDGE_COOKIE");
        std::env::remove_var("GROK2API_BRIDGE_USER_AGENT");
        let c = HttpBridgeClient::new();
        assert_eq!(c.session_key.len(), 32);
    }

    #[test]
    fn http_client_with_session_context_sets_fields() {
        let c = HttpBridgeClient::default().with_session_context("k", "cookie", "ua");
        assert_eq!(c.session_key, "k");
        assert_eq!(c.cookie, "cookie");
        assert_eq!(c.user_agent, "ua");
    }
}
