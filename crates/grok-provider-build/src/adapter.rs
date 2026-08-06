//! Grok Build 适配器（对齐 Go `cli/adapter.go` 的 ForwardResponse 最小闭环）。
//!
//! G5-P1 覆盖：
//! - `forward`：通用方法 + 上游请求头注入（Authorization / x-grok-* 系列）
//! - `forward_stored`：`POST /responses` 存储往返（规整 → 发送 → 解析文本）
//!
//! 客户端身份（agent_id 32 hex / session_id UUID）按账号缓存（Go `clientIdentity`）。

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use rand::Rng;
use reqwest::{Client, Method};
use serde_json::{json, Value};

use crate::error::ProviderError;
use crate::normalize::{ensure_prompt_cache_key, normalize_responses_request};
use crate::response::StoredResponse;

/// 适配器配置（对齐 Go `cli.Config`）。
#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub client_version: String,
    pub client_identifier: String,
    pub token_auth: String,
    pub user_agent: String,
    /// 请求总超时（连接 + 响应头 + 体），`GROK2API_UPSTREAM_TIMEOUT_MS` 可覆盖，缺省 60s。
    pub timeout: std::time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: crate::default_base_url(),
            client_version: "0.2.99".into(),
            client_identifier: "grok-shell".into(),
            token_auth: "xai-grok-cli".into(),
            user_agent: "grok-shell/0.2.99 (linux; x86_64)".into(),
            timeout: crate::default_timeout(),
        }
    }
}

/// 上游请求超时（env `GROK2API_UPSTREAM_TIMEOUT_MS` 覆盖，缺省 60_000ms）。
pub fn default_timeout() -> std::time::Duration {
    let ms = std::env::var("GROK2API_UPSTREAM_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(60_000);
    std::time::Duration::from_millis(ms)
}

/// 转发请求（对齐 Go `provider.ResponseResourceRequest` 的 Build 子集）。
#[derive(Debug, Clone)]
pub struct ForwardRequest {
    pub method: Method,
    /// 上游路径，如 `/responses`；可带 query。
    pub path: String,
    pub model: String,
    pub access_token: String,
    pub user_id: Option<String>,
    pub prompt_cache_key: String,
    pub body: Option<Value>,
    /// 是否流式（G5-P1 恒 false）。
    pub streaming: bool,
}

/// 转发响应（状态码 + 原始文本，非流式）。
#[derive(Debug, Clone)]
pub struct ForwardResponse {
    pub status: u16,
    pub body: String,
}

impl ForwardResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, Clone)]
struct ClientIdentity {
    agent_id: String,
    session_id: String,
}

/// Grok Build 适配器。
pub struct BuildAdapter {
    client: Client,
    cfg: RwLock<Config>,
    identities: Mutex<HashMap<i64, ClientIdentity>>,
}

impl BuildAdapter {
    pub fn new(cfg: Config) -> Self {
        let client = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .expect("reqwest client");
        Self::with_client(client, cfg)
    }

    /// 注入 HTTP 客户端（测试指向本地 mock server）。
    pub fn with_client(client: Client, cfg: Config) -> Self {
        Self {
            client,
            cfg: RwLock::new(cfg),
            identities: Mutex::new(HashMap::new()),
        }
    }

    pub fn update_config(&self, cfg: Config) {
        *self.cfg.write().unwrap() = cfg;
    }

    /// Stored response 往返：构造 `store:false, stream:false` 请求 → 规整 → 发送 → 解析。
    pub async fn forward_stored(
        &self,
        model: &str,
        input: Value,
        max_output_tokens: i64,
        access_token: &str,
        prompt_cache_key: &str,
    ) -> Result<StoredResponse, ProviderError> {
        let request = ForwardRequest {
            method: Method::POST,
            path: "/responses".into(),
            model: model.to_string(),
            access_token: access_token.to_string(),
            user_id: None,
            prompt_cache_key: prompt_cache_key.to_string(),
            body: Some(json!({
                "model": model,
                "input": input,
                "max_output_tokens": max_output_tokens,
                "store": false,
                "stream": false,
            })),
            streaming: false,
        };
        let response = self.forward(&request).await?;
        if !response.is_success() {
            return Err(ProviderError::Upstream(format!(
                "上游 Responses 返回 {}: {}",
                response.status,
                truncate(&response.body, 512)
            )));
        }
        let value: Value = serde_json::from_str(&response.body)
            .map_err(|e| ProviderError::Upstream(format!("解析上游响应: {e}")))?;
        StoredResponse::from_json(&value)
    }

    /// 通用转发（对齐 Go `ForwardResponse` 的请求侧：头部注入 + 可选规整）。
    pub async fn forward(
        &self,
        request: &ForwardRequest,
    ) -> Result<ForwardResponse, ProviderError> {
        let cfg = self.cfg.read().unwrap().clone();
        let url = format!(
            "{}/{}",
            cfg.base_url.trim_end_matches('/'),
            request.path.trim_start_matches('/')
        );

        // 规整请求体：模型覆盖 + response_format 映射 + prompt_cache_key（有体时才做，
        // 对齐 Go `NormalizeBody` 分支；GET/DELETE 资源方法无体）。
        let mut body = request.body.clone().unwrap_or(Value::Null);
        if body != Value::Null {
            body = normalize_responses_request(&body, &request.model)?;
            if !request.prompt_cache_key.trim().is_empty() {
                body = ensure_prompt_cache_key(&body, &request.prompt_cache_key)?;
            }
        }

        let identity = self.client_identity(0);
        let req_id = random_hex(16);
        let conv_id = if request.prompt_cache_key.trim().is_empty() {
            random_hex(16)
        } else {
            request.prompt_cache_key.trim().to_string()
        };
        let trace_id = random_hex(16);
        let span_id = random_hex(8);

        let mut rb = self
            .client
            .request(request.method.clone(), &url)
            .header("Authorization", format!("Bearer {}", request.access_token))
            .header("X-XAI-Token-Auth", cfg.token_auth)
            .header("x-grok-client-version", cfg.client_version.clone())
            .header("x-grok-client-identifier", cfg.client_identifier.clone())
            .header("x-grok-client-surface", "tui")
            .header("x-grok-client-name", cfg.client_identifier.clone())
            .header("x-grok-agent-id", identity.agent_id.clone())
            .header("x-grok-session-id", identity.session_id.clone())
            .header("x-grok-conv-id", conv_id.clone())
            .header("x-grok-req-id", req_id.clone())
            .header("x-grok-conversation-id", conv_id)
            .header("x-grok-session-id-legacy", identity.session_id)
            .header("x-grok-request-id", req_id)
            .header("traceparent", format!("00-{trace_id}-{span_id}-01"))
            .header("tracestate", "")
            .header("Accept-Encoding", "gzip")
            .header("User-Agent", cfg.user_agent);
        if let Some(user_id) = &request.user_id {
            rb = rb.header("x-userid", user_id);
        }
        if !request.streaming {
            rb = rb.header("Accept", "application/json");
        }
        if body != Value::Null {
            rb = rb.header("Content-Type", "application/json").json(&body);
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderError::Timeout(cfg.timeout)
                } else if e.is_connect() {
                    ProviderError::Http(format!("连接上游失败: {e}"))
                } else {
                    ProviderError::Http(format!("请求上游失败: {e}"))
                }
            })?;
        let status = resp.status().as_u16();
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Upstream(format!("读取响应: {e}")))?;
        Ok(ForwardResponse { status, body: text })
    }

    /// 账号级客户端身份（Go `clientIdentity`）：首见生成并缓存。
    fn client_identity(&self, account_id: i64) -> ClientIdentity {
        let mut map = self.identities.lock().unwrap();
        map.entry(account_id)
            .or_insert_with(|| ClientIdentity {
                agent_id: random_hex(16),
                session_id: random_uuid(),
            })
            .clone()
    }
}

/// 生成 `bytes_length` 字节的 hex（小写，2 倍长度；Go `randomHex`）。
pub fn random_hex(bytes_length: usize) -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..bytes_length).map(|_| rng.gen()).collect();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 生成 v4 UUID 字符串（Go `randomUUID`）。
pub fn random_uuid() -> String {
    let mut rng = rand::thread_rng();
    let mut value = [0u8; 16];
    rng.fill(&mut value);
    value[6] = (value[6] & 0x0f) | 0x40;
    value[8] = (value[8] & 0x3f) | 0x80;
    let hex: String = value.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}…", &text[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_hex_and_uuid_shapes() {
        assert_eq!(random_hex(16).len(), 32);
        assert_eq!(random_uuid().len(), 36);
        let uuid = random_uuid();
        assert_eq!(uuid.chars().nth(14).unwrap(), '4', "v4 version nibble");
        assert_ne!(random_uuid(), uuid);
    }
}
