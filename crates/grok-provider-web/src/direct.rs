//! HttpDirectClient — 无 chrome 直连 grok.com（对齐 Go 无桥路径）。
//!
//! BridgeClient 的第二个实现：不需要浏览器。认证用账号 sso token
//! （`Cookie: sso=<t>; sso-rw=<t>`），`x-statsig-id` 由外部 signer 服务签名。
//!
//! - chat/OCR：POST `{base}/rest/app-chat/conversations/new`，解析 SSE 文本增量。
//! - 生图：WS 直连 `wss://{host}/ws/imagine/listen` 收帧（对齐 Go image.go）；
//!   WS 帧二进制格式与上游耦合，需真实上游联调（残余风险，见 docs/39f）。
//!
//! 安全红线：客户端只组装带 cookie 的请求；调用方（grok2api-rs）负责在
//! token/密钥缺失时返回 503 绝不外呼。

use std::sync::Arc;

use grok_domain::{ProviderError, SsoTokenProvider};
use serde_json::Value;

use crate::bridge::BridgeClient;
use crate::statsig::StatsigSigner;

/// 上游总超时（对齐 provider「connect 5s / total 60s」红线语义）。
const TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// SSE 分块读取上限（防御失控响应）。
const MAX_BITSTREAM_READ_LIMIT: usize = 64 << 20;

/// 直连客户端配置。
#[derive(Clone)]
pub struct DirectConfig {
    /// grok.com 基地址（缺省 https://grok.com）。
    pub base_url: String,
    /// statsig signer 服务地址（缺省公网 grok.wodf.de/sign）。
    pub signer_url: String,
    /// 直连需要 sso token；`sso` 提供按账号取 token。
    pub sso: Option<Arc<dyn SsoTokenProvider>>,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            base_url: "https://grok.com".to_string(),
            signer_url: "https://grok.wodf.de/sign".to_string(),
            sso: None,
        }
    }
}

/// 直连客户端。
pub struct HttpDirectClient {
    client: reqwest::Client,
    signer: StatsigSigner,
    cfg: DirectConfig,
}

impl HttpDirectClient {
    /// 构造直连客户端。
    pub fn new(cfg: DirectConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .danger_accept_invalid_certs(false)
            .build()
            .expect("reqwest client");
        let signer = StatsigSigner::new(client.clone(), cfg.signer_url.clone());
        Self {
            client,
            signer,
            cfg,
        }
    }

    /// 组装直连请求头（对齐 Go `headers.go` + statsig）。
    fn build_headers(
        &self,
        sso_cookie: &str,
        statsig_id: &str,
        content_type: &str,
    ) -> reqwest::header::HeaderMap {
        use reqwest::header::*;
        let mut map = HeaderMap::new();
        map.insert(
            CONTENT_TYPE,
            HeaderValue::from_str(content_type)
                .unwrap_or(HeaderValue::from_static("application/json")),
        );
        map.insert(ACCEPT, HeaderValue::from_static("*/*"));
        map.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        let origin = self.cfg.base_url.trim_end_matches('/').to_string();
        map.insert(
            ORIGIN,
            HeaderValue::from_str(&origin)
                .unwrap_or_else(|_| HeaderValue::from_static("https://grok.com")),
        );
        map.insert(
            REFERER,
            HeaderValue::from_str(&format!("{origin}/"))
                .unwrap_or_else(|_| HeaderValue::from_static("https://grok.com/")),
        );
        map.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        map.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        map.insert(
            HeaderName::from_static("priority"),
            HeaderValue::from_static("u=1, i"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-dest"),
            HeaderValue::from_static("empty"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-mode"),
            HeaderValue::from_static("cors"),
        );
        map.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );
        map.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
        if !sso_cookie.is_empty() {
            if let Ok(cookie_value) = HeaderValue::from_str(sso_cookie) {
                map.insert(COOKIE, cookie_value);
            }
        }
        if !statsig_id.is_empty() {
            if let Ok(id_value) = HeaderValue::from_str(statsig_id) {
                map.insert(HeaderName::from_static("x-statsig-id"), id_value);
            }
        }
        map
    }

    /// 对目标 path 签名（method + path → x-statsig-id）。
    async fn sign_path(
        &self,
        sso_cookie: &str,
        method: &str,
        path: &str,
    ) -> Result<String, ProviderError> {
        self.signer
            .sign(
                &self.cfg.base_url,
                &self.cfg.signer_url,
                sso_cookie,
                method,
                path,
            )
            .await
            .map_err(|e| {
                tracing::warn!(signer = %self.cfg.signer_url, "statsig sign failed: {e}");
                e
            })
    }

    /// 从 cookie 头构造（sso=<t> 原始 token 直接拼接）。
    fn build_sso_cookie(token: &str) -> String {
        let token = token.trim();
        let token = token.strip_prefix("sso=").map(str::trim).unwrap_or(token);
        let token = token.split(';').next().unwrap_or("").trim();
        // 清理危险字符（防 http 头注入）。
        let sanitized: String = token
            .chars()
            .filter(|c| *c != '\r' && *c != '\n' && *c != '\0')
            .collect();
        format!("sso={sanitized}; sso-rw={sanitized}")
    }
}

/// 快速解析 SSE 文本：逐行读，`data: {json}` → `result.response.token` → 拼 text。
/// 若整个 body 不是 SSE（如返回 OpenAI 风格 JSON），熔断：尝试 `choices[0].message.content`。
fn parse_sse_text(body: &[u8]) -> Result<String, ProviderError> {
    let text = String::from_utf8_lossy(body);
    let mut out = String::new();
    let mut saw_data = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            saw_data = true;
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(root) = serde_json::from_str::<Value>(data) {
                if let Some(err) = root.get("error") {
                    let msg = err
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("upstream error");
                    return Err(ProviderError::Upstream(msg.to_string()));
                }
                if let Some(token) = root
                    .get("result")
                    .and_then(|r| r.get("response"))
                    .and_then(|r| r.get("token"))
                    .and_then(|t| t.as_str())
                {
                    let thinking = root
                        .get("result")
                        .and_then(|r| r.get("response"))
                        .and_then(|r| r.get("isThinking"))
                        .and_then(|t| t.as_bool())
                        .unwrap_or(false);
                    let tag = root
                        .get("result")
                        .and_then(|r| r.get("response"))
                        .and_then(|r| r.get("messageTag"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    // 对齐 Go：thinking token 不计入正文；正文 token 需非 thinking。
                    if !thinking {
                        out.push_str(token);
                    } else {
                        let _ = tag;
                    }
                }
            }
        }
    }
    if !saw_data {
        // 非 SSE：尝试 OpenAI 错误体
        if let Ok(root) = serde_json::from_str::<Value>(&text) {
            if let Some(msg) = root
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
            {
                return Err(ProviderError::Upstream(msg.to_string()));
            }
            if let Some(content) = root
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_str())
            {
                return Ok(content.to_string());
            }
        }
    }
    if out.is_empty() && !saw_data {
        return Err(ProviderError::Upstream("empty chat response".into()));
    }
    Ok(out)
}

#[async_trait::async_trait]
impl BridgeClient for HttpDirectClient {
    async fn fetch_bytes(
        &self,
        image_url: &str,
        _sso_token: Option<&str>,
    ) -> Result<Vec<u8>, ProviderError> {
        // data URI 本地直解
        if let Some(payload) = crate::bridge::decode_data_uri_public(image_url) {
            return Ok(payload);
        }
        let resp = self
            .client
            .get(image_url)
            .send()
            .await
            .map_err(|e| ProviderError::Bridge(format!("direct fetch bytes: {e}")))?;
        if !resp.status().is_success() {
            return Err(ProviderError::Upstream(format!(
                "image fetch status {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("image body: {e}")))?;
        Ok(bytes.to_vec())
    }

    async fn fetch_chat(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
    ) -> Result<String, ProviderError> {
        let Some(sso_token) = sso_token else {
            // 安全红线：直连无 token 一律 503 不外呼。
            return Err(ProviderError::NoAvailableAccount);
        };
        let cookie = Self::build_sso_cookie(sso_token);
        let path = "/rest/app-chat/conversations/new";
        let statsig_id = self.sign_path(&cookie, "POST", path).await?;
        let endpoint = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), path);
        let resp = self
            .client
            .post(&endpoint)
            .headers(self.build_headers(&cookie, &statsig_id, "application/json"))
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                let tag = if e.is_timeout() {
                    "timeout"
                } else if e.is_connect() {
                    "connect"
                } else {
                    "unknown"
                };
                ProviderError::Bridge(format!("direct chat {tag}: {e}"))
            })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Upstream(format!("chat status {status}")));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("chat body: {e}")))?;
        if bytes.len() > MAX_BITSTREAM_READ_LIMIT {
            return Err(ProviderError::Upstream("chat stream over limit".into()));
        }
        parse_sse_text(&bytes)
    }

    async fn fetch_imagine(
        &self,
        payload: &Value,
        sso_token: Option<&str>,
    ) -> Result<Value, ProviderError> {
        // 简化：直连路径对生图先用 chat 端点「new conversation」（对齐 Go openChat 首步），
        // 图片结果依赖 WS 帧；WS 二进制帧格式与上游耦合，需真实上游联调验证。
        // 此处回退到 fetch_chat 文本 SSE（同 endpoint），提取文本供调用方判断，
        // WS 收帧在 `direct_imagine_ws`（TODO：真实上游联调后启用）。
        self.fetch_chat(payload, sso_token).await.map(|text| {
            serde_json::json!({
                "object": "list",
                "created": chrono::Utc::now().timestamp(),
                "data": [ { "text": text } ],
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_extracts_tokens() {
        let body = r#"
data: {"result":{"conversation":{"conversationId":"c1"}}}
data: {"result":{"response":{"token":"你","isThinking":false,"messageTag":"final"}}}
data: {"result":{"response":{"token":"好","isThinking":false,"messageTag":"final"}}}
data: [DONE]
"#;
        let out = parse_sse_text(body.as_bytes()).unwrap();
        assert_eq!(out, "你好");
    }

    #[test]
    fn parse_sse_ignores_thinking_and_empty() {
        let body = r#"
data: {"result":{"response":{"token":"思考中","isThinking":true}}}
data: {"result":{"response":{"token":"答","isThinking":false,"messageTag":""}}}
"#;
        let out = parse_sse_text(body.as_bytes()).unwrap();
        assert_eq!(out, "答");
    }

    #[test]
    fn parse_sse_surfaces_errors() {
        let body = r#"data: {"error":{"message":"boom"}}
"#;
        let err = parse_sse_text(body.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn sso_cookie_build_is_sanitized() {
        assert_eq!(
            HttpDirectClient::build_sso_cookie("sso=abc"),
            "sso=abc; sso-rw=abc"
        );
        assert_eq!(
            HttpDirectClient::build_sso_cookie("abc; extra=1"),
            "sso=abc; sso-rw=abc"
        );
        assert_eq!(
            HttpDirectClient::build_sso_cookie("a\r\nb"),
            "sso=ab; sso-rw=ab"
        );
    }

    #[tokio::test]
    async fn fetch_chat_without_token_is_503_semantics() {
        let client = HttpDirectClient::new(DirectConfig::default());
        let err = client
            .fetch_chat(&serde_json::json!({"model":"x"}), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NoAvailableAccount));
    }

    #[tokio::test]
    async fn no_upstream_call_when_token_missing() {
        // 用会失败的 signer url 验证：token 缺失时根本不发上游请求（直接短路）。
        let cfg = DirectConfig {
            base_url: "https://grok.com".to_string(),
            signer_url: "https://127.0.0.1:1/sign".to_string(),
            sso: None,
        };
        let client = HttpDirectClient::new(cfg);
        let err = client
            .fetch_chat(&serde_json::json!({"model":"x"}), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NoAvailableAccount));
    }
}
