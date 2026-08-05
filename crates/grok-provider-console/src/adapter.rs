//! Console 适配器（G5-A2/A3：chat completions 流式往返）。
//!
//! `POST {base}/v1/chat/completions`（stream=true）→ 增量读上游 SSE →
//! 规整为 OpenAI 兼容分片序列（`Vec<ChatDelta>`）。
//!
//! 请求头对齐 Go `console/adapter.go::applyHeaders`：`Authorization: Bearer anonymous`、
//! `Cookie: sso=<token>; sso-rw=<token>`（SSO 令牌直接注入）、`x-cluster`、
//! Origin/Referer、`Accept: text/event-stream`。
//!
//! 边界（本任务不做）：/v1/responses 协议、egress 租约、凭据导入/加密（令牌明文）、
//! 模型目录/别名（G5-P2 后续）。

use std::sync::RwLock;
use std::time::Duration;

use reqwest::{Client, StatusCode};
use serde_json::Value;

use crate::error::{ProviderError, UpstreamError};
use crate::normalize::build_chat_request;
use crate::sse::{ChatDelta, SseParser, parse_chat_delta};

/// 适配器配置（对齐 Go `console.Config`）。
#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub user_agent: String,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: crate::default_base_url(),
            user_agent: "grok-console/0.1".into(),
            timeout: Duration::from_secs(300),
        }
    }
}

/// Console 适配器。
pub struct ConsoleAdapter {
    client: Client,
    cfg: RwLock<Config>,
}

impl ConsoleAdapter {
    pub fn new(cfg: Config) -> Self {
        Self::with_client(Self::build_client(&cfg), cfg)
    }

    /// 注入 HTTP 客户端（测试指向本地 mock server）。
    pub fn with_client(client: Client, cfg: Config) -> Self {
        Self {
            client,
            cfg: RwLock::new(cfg),
        }
    }

    fn build_client(cfg: &Config) -> Client {
        Client::builder().timeout(cfg.timeout).build().expect("reqwest client")
    }

    pub fn update_config(&self, cfg: Config) {
        *self.cfg.write().unwrap() = cfg;
    }

    /// 流式 chat 往返：构造请求 → 发送 → 增量解析 SSE → 归一化分片序列。
    ///
    /// 错误分类：请求参数非法 → `InvalidRequest`；非 2xx → `Upstream`
    /// （消息含解析出的 error.type 与 message）；连接失败 → `Http`；超时 → `Timeout`。
    pub async fn forward_chat(
        &self,
        model: &str,
        messages: &Value,
        access_token: &str,
    ) -> Result<Vec<ChatDelta>, ProviderError> {
        let cfg = self.cfg.read().unwrap().clone();
        let url = format!(
            "{}/v1/chat/completions",
            cfg.base_url.trim_end_matches('/')
        );
        let body = build_chat_request(model, messages, true)?;

        let response = self
            .client
            .post(&url)
            .header("Accept", "text/event-stream")
            .header("Authorization", "Bearer anonymous")
            .header("Cookie", format!("sso={access_token}; sso-rw={access_token}"))
            .header("Origin", "https://console.x.ai")
            .header("Referer", "https://console.x.ai/")
            .header("x-cluster", "https://us-east-1.api.x.ai")
            .header("User-Agent", cfg.user_agent)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| classify_reqwest_error(e, cfg.timeout))?;

        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .await
                .map_err(|e| ProviderError::Upstream(format!("读取错误响应: {e}")))?;
            let upstream = UpstreamError::parse(status.as_u16(), &text);
            return Err(ProviderError::Upstream(format!(
                "上游 {status} {}: {}",
                upstream.error_type, upstream.message
            )));
        }

        // 2xx：增量读流
        let mut parser = SseParser::new();
        let mut deltas = Vec::new();
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| classify_reqwest_error(e, cfg.timeout))?;
            for event in parser.feed(&bytes) {
                collect_delta(&mut deltas, &event.data)?;
            }
        }
        for event in parser.finish() {
            collect_delta(&mut deltas, &event.data)?;
        }
        Ok(deltas)
    }
}

fn collect_delta(deltas: &mut Vec<ChatDelta>, data: &str) -> Result<(), ProviderError> {
    if let Some(delta) = parse_chat_delta(data)? {
        deltas.push(delta);
    }
    Ok(())
}

fn classify_reqwest_error(error: reqwest::Error, timeout: Duration) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout(timeout)
    } else if error.is_connect() {
        ProviderError::Http(format!("连接上游失败: {error}"))
    } else {
        ProviderError::Http(format!("请求上游失败: {error}"))
    }
}

/// 供测试/调用方快速判读错误状态码。
pub fn is_rate_limit(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
}
