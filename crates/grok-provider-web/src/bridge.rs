//! browser-bridge HTTP 客户端（docs/39d §8，39 主文档 §4.3）。
//!
//! browser-bridge 是 Rust 化 **不迁移** 的侧车（`browser-bridge/app.py`），这里只做
//! HTTP 封装：`post /v1/fetch` 下载图片字节 / 转发 chat 请求并回文本。
//!
//! G1 允许 mock：`BridgeClient` trait 抽象了「从可注入 client 构造」，测试用
//! [`MockBridgeClient`] 直插，不发起真实 HTTP。地址取 `GROK2API_BROWSER_BRIDGE_URL`
//! 环境变量，默认 `http://browser-bridge:8192`。

use reqwest::{Client, Method};
use serde_json::Value;

use crate::error::ProviderError;

/// bridge 基地址。
pub fn default_bridge_url() -> String {
    std::env::var("GROK2API_BROWSER_BRIDGE_URL")
        .unwrap_or_else(|_| "http://browser-bridge:8192".to_string())
}

/// browser-bridge 客户端抽象（便于单测注入 fake）。
///
/// `fetch_bytes`：POST /v1/fetch 下载一张远端 HTTPS 图片。
/// `fetch_chat`：POST /v1/fetch 转发上游 chat payload，返回最终文本。
#[async_trait::async_trait]
pub trait BridgeClient: Send + Sync {
    /// 下载图片二进制（经 bridge）。`image_url` 为 HTTPS URL 或 data URI。
    async fn fetch_bytes(&self, image_url: &str) -> Result<Vec<u8>, ProviderError>;

    /// 发送 chat 请求到 bridge，返回上游 SSE 汇总后的文本。
    async fn fetch_chat(&self, payload: &Value) -> Result<String, ProviderError>;
}

/// 基于 `reqwest::Client` 的真实 bridge 客户端。
pub struct HttpBridgeClient {
    client: Client,
    base: String,
}

impl HttpBridgeClient {
    /// 用默认 `reqwest::Client` 与 `GROK2API_BROWSER_BRIDGE_URL` 构造。
    pub fn new() -> Self {
        Self::with_client_and_base(Client::new(), default_bridge_url())
    }

    /// 注入客户端与基地址（测试可指向 mock server）。
    pub fn with_client_and_base(client: Client, base: String) -> Self {
        Self { client, base }
    }
}

impl Default for HttpBridgeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpBridgeClient {
    /// 底层 `POST {base}/v1/fetch`，body 为给定 JSON，返回响应文本。
    async fn post_fetch(&self, body: Value) -> Result<reqwest::Response, ProviderError> {
        let url = format!("{}/v1/fetch", self.base.trim_end_matches('/'));
        let resp = self
            .client
            .request(Method::POST, &url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Bridge(format!("connected to {url}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(ProviderError::Bridge(format!(
                "bridge {status} for {url}: {body}"
            )));
        }
        Ok(resp)
    }
}

#[async_trait::async_trait]
impl BridgeClient for HttpBridgeClient {
    async fn fetch_bytes(&self, image_url: &str) -> Result<Vec<u8>, ProviderError> {
        // data URI 本地直解（不经 bridge）：仅 HTTP(S) 图片走 bridge 下载。
        if let Some(payload) = decode_data_uri(image_url) {
            return Ok(payload);
        }
        let body = serde_json::json!({
            "action": "download_image",
            "url": image_url,
        });
        let resp = self.post_fetch(body).await?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Bridge(format!("read image bytes: {e}")))?;
        tracing::debug!(len = bytes.len(), url = image_url, "bridge fetched image");
        Ok(bytes.to_vec())
    }

    async fn fetch_chat(&self, payload: &Value) -> Result<String, ProviderError> {
        let body = serde_json::json!({
            "action": "chat",
            "payload": payload,
        });
        let resp = self.post_fetch(body).await?;
        // bridge 返回 `{"text": "..."}` 或裸文本；都兜底为字符串。
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Bridge(format!("read chat text: {e}")))?;
        parse_chat_text(&text)
    }
}

/// 从 bridge chat 响应提取纯文本：优先 `{"text": "..."}`，否则整段字符串。
fn parse_chat_text(raw: &str) -> Result<String, ProviderError> {
    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(t) = v.get("text").and_then(Value::as_str) {
            return Ok(t.to_string());
        }
        if let Some(ch) = v.get("choices").and_then(Value::as_array) {
            if let Some(first) = ch.first() {
                let msg = first
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_str);
                if let Some(t) = msg {
                    return Ok(t.to_string());
                }
            }
        }
    }
    // 裸文本。
    Ok(raw.to_string())
}

/// 解码 `data:image/<type>;base64,<payload>`。非 data URI 返回 None。
fn decode_data_uri(value: &str) -> Option<Vec<u8>> {
    let rest = value.strip_prefix("data:")?;
    let encoded = rest.split_once(";base64,")?.1;
    use base64::Engine;
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
}

impl MockBridgeClient {
    pub fn new() -> Self {
        Self {
            images: std::collections::HashMap::new(),
            chat_text: String::new(),
            last_chat_payload: tokio::sync::Mutex::new(None),
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
    async fn fetch_bytes(&self, image_url: &str) -> Result<Vec<u8>, ProviderError> {
        if let Some(b) = decode_data_uri(image_url) {
            return Ok(b);
        }
        self.images
            .get(image_url)
            .cloned()
            .ok_or_else(|| ProviderError::Bridge(format!("mock no bytes for {image_url}")))
    }

    async fn fetch_chat(&self, payload: &Value) -> Result<String, ProviderError> {
        *self.last_chat_payload.lock().await = Some(payload.clone());
        Ok(self.chat_text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_decodes_to_bytes() {
        // `AA==` = [0x00]; 用一个很小的合法 PNG base64。
        let uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let bytes = decode_data_uri(uri).expect("decode");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn non_data_uri_is_none() {
        assert!(decode_data_uri("https://x.com/a.png").is_none());
    }

    #[test]
    fn parse_chat_text_json_and_plain() {
        assert_eq!(parse_chat_text(r#"{"text":"你好"}"#).unwrap(), "你好");
        assert_eq!(parse_chat_text("plain text").unwrap(), "plain text");
        // choices[0].message.content
        let msg = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        assert_eq!(parse_chat_text(msg).unwrap(), "hi");
    }

    #[tokio::test]
    async fn mock_fetch_chat_records_payload() {
        let mut m = MockBridgeClient::new();
        m.chat_text = "ok".to_string();
        let p = serde_json::json!({"model": "grok-chat-fast"});
        let out = m.fetch_chat(&p).await.unwrap();
        assert_eq!(out, "ok");
        let got = m.last_chat_payload.lock().await;
        assert_eq!(got.as_ref().unwrap()["model"], "grok-chat-fast");
    }
}
