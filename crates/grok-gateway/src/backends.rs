//! G5-P3 真实协议后端：把归一化后的对话输入接到 grok-provider-build / grok-provider-console。
//!
//! - [`BuildResponsesBackend`]：`/v1/responses` → Build `POST /responses` stored response
//!   单次往返（`store:false, stream:false`），取回 `output_text` 文本。
//! - [`ConsoleMessagesBackend`]：`/v1/messages` → Console `POST /v1/chat/completions`
//!   SSE 流式往返，拼接 `choices[0].delta.content` 分片。
//!
//! 上游地址可注入（`Config{base_url}`）或走各自 `default_base_url()`（env / 常量），
//! 便于测试用 TcpListener mock server 指到本地。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use grok_conversation::NormalizedChatInput;
use grok_provider_build::{BuildAdapter, Config as BuildConfig, ProviderError};
use grok_provider_console::{ChatDelta, Config as ConsoleConfig, ConsoleAdapter};

use crate::error::GatewayError;
use crate::handlers::ProtocolBackend;

/// Build stored response 最大输出 token（Go 探测用 16；对话默认 1024）。
const MAX_OUTPUT_TOKENS: i64 = 1024;

/// `/v1/responses` 真实后端（Build provider，G5-P1 stored response 往返）。
pub struct BuildResponsesBackend {
    adapter: BuildAdapter,
    access_token: String,
}

impl BuildResponsesBackend {
    /// `base_url` 为空时走 `grok_provider_build::default_base_url()`（env / 常量）。
    pub fn new(base_url: Option<String>, access_token: String) -> Self {
        let cfg = BuildConfig {
            base_url: base_url.unwrap_or_else(grok_provider_build::default_base_url),
            ..Default::default()
        };
        Self {
            adapter: BuildAdapter::new(cfg),
            access_token,
        }
    }

    /// 归一化输入 → OpenAI Responses `input` 数组（input_text / input_image）。
    fn to_input(normalized: &NormalizedChatInput) -> Value {
        let mut content = vec![json!({"type": "input_text", "text": normalized.prompt})];
        for image in &normalized.images {
            content.push(json!({"type": "input_image", "image_url": image}));
        }
        json!([{
            "type": "message",
            "role": "user",
            "content": content,
        }])
    }
}

#[async_trait]
impl ProtocolBackend for BuildResponsesBackend {
    async fn complete(
        &self,
        model: &str,
        normalized: &NormalizedChatInput,
    ) -> Result<String, GatewayError> {
        let input = Self::to_input(normalized);
        let stored = self
            .adapter
            .forward_stored(
                model,
                input,
                MAX_OUTPUT_TOKENS,
                &self.access_token,
                "", // G5-P3 无 prompt cache key；有需求时经请求头透传
            )
            .await
            .map_err(sanitize_upstream_error)?;
        Ok(stored.text())
    }
}

/// `/v1/messages` 真实后端（Console provider，G5-A2/A3 SSE 流式）。
pub struct ConsoleMessagesBackend {
    adapter: ConsoleAdapter,
    access_token: String,
}

impl ConsoleMessagesBackend {
    /// `base_url` 为空时走 `grok_provider_console::default_base_url()`（env / 常量）。
    pub fn new(base_url: Option<String>, access_token: String) -> Self {
        let cfg = ConsoleConfig {
            base_url: base_url.unwrap_or_else(grok_provider_console::default_base_url),
            ..Default::default()
        };
        Self {
            adapter: ConsoleAdapter::new(cfg),
            access_token,
        }
    }

    /// 归一化输入 → OpenAI chat messages 数组（text / image_url 混合 content）。
    fn to_messages(normalized: &NormalizedChatInput) -> Value {
        let mut content = vec![json!({"type": "text", "text": normalized.prompt})];
        for image in &normalized.images {
            content.push(json!({"type": "image_url", "image_url": {"url": image}}));
        }
        json!([{ "role": "user", "content": content }])
    }
}

#[async_trait]
impl ProtocolBackend for ConsoleMessagesBackend {
    async fn complete(
        &self,
        model: &str,
        normalized: &NormalizedChatInput,
    ) -> Result<String, GatewayError> {
        let messages = Self::to_messages(normalized);
        let deltas = self
            .adapter
            .forward_chat(model, &messages, &self.access_token)
            .await
            .map_err(sanitize_console_error)?;
        Ok(join_deltas(&deltas))
    }
}

/// 拼接分片 content（不含 role / finish_reason）。
fn join_deltas(deltas: &[ChatDelta]) -> String {
    deltas
        .iter()
        .filter_map(|d| d.content.as_deref())
        .collect::<String>()
}

/// 上游错误脱敏：只保留分类 + 有限消息，**不向客户端透出 base_url / 内网拓扑**。
///
/// `ProviderError::Http` 内部含 URL（如 "请求 https://…: connect …"），
/// 一律折叠为固定文案；`Upstream` 仅透出状态码与解析出的 message（截断 512）。
fn sanitize_upstream_error(e: ProviderError) -> GatewayError {
    match e {
        ProviderError::InvalidRequest(msg) => GatewayError::InvalidRequest(msg),
        ProviderError::Upstream(msg) => GatewayError::Upstream(sanitize_message(&msg)),
        ProviderError::Timeout(d) => {
            GatewayError::Upstream(format!("上游请求超时（{}ms）", d.as_millis()))
        }
        ProviderError::Http(_) => GatewayError::Upstream("上游请求失败".to_string()),
    }
}

/// 同上，适用于 console provider 的错误（各 crate 的 `ProviderError` 为独立类型）。
fn sanitize_console_error(e: grok_provider_console::ProviderError) -> GatewayError {
    use grok_provider_console::ProviderError as ConsoleError;
    match e {
        ConsoleError::InvalidRequest(msg) => GatewayError::InvalidRequest(msg),
        ConsoleError::Upstream(msg) => GatewayError::Upstream(sanitize_message(&msg)),
        ConsoleError::Timeout(d) => {
            GatewayError::Upstream(format!("上游请求超时（{}ms）", d.as_millis()))
        }
        ConsoleError::Http(_) => GatewayError::Upstream("上游请求失败".to_string()),
    }
}

/// 截断并剥离疑似 URL/拓扑痕迹的消息（保留正文片段，避免泄露内网信息）。
fn sanitize_message(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len().min(512));
    for token in msg.split_whitespace() {
        let t = token.trim();
        if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("//") {
            out.push_str("[redacted] ");
            continue;
        }
        out.push_str(t);
        out.push(' ');
        if out.len() >= 512 {
            out.truncate(512);
            break;
        }
    }
    out.trim().to_string()
}

/// 默认真实后端对（Build + Console），base_url 可覆盖（测试指 mock server）。
///
/// **未配置保护**：对应 env token（`GROK2API_BUILD_TOKEN` / `GROK2API_CONSOLE_TOKEN`）
/// 为空时返回 `None`（该端点将 503，绝不携带空 Bearer 外呼真实 grok）。
/// 显式 [`BuildResponsesBackend::new`] / [`ConsoleMessagesBackend::new`] 不受此限
/// （信任调用方注入，如内部网/mock）。
/// (responses, messages) 后端对。
pub type ProtocolBackendPair = (
    Option<Arc<dyn ProtocolBackend>>,
    Option<Arc<dyn ProtocolBackend>>,
);

pub fn default_protocol_backends(
    build_base_url: Option<String>,
    console_base_url: Option<String>,
) -> ProtocolBackendPair {
    let build_token = std::env::var("GROK2API_BUILD_TOKEN").unwrap_or_default();
    let console_token = std::env::var("GROK2API_CONSOLE_TOKEN").unwrap_or_default();
    let responses = if build_token.trim().is_empty() {
        None
    } else {
        Some(
            Arc::new(BuildResponsesBackend::new(build_base_url, build_token))
                as Arc<dyn ProtocolBackend>,
        )
    };
    let messages = if console_token.trim().is_empty() {
        None
    } else {
        Some(
            Arc::new(ConsoleMessagesBackend::new(console_base_url, console_token))
                as Arc<dyn ProtocolBackend>,
        )
    };
    (responses, messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_input_to_build_input_shape() {
        let normalized = NormalizedChatInput {
            prompt: "[user]\nhello".into(),
            images: vec!["data:image/png;base64,AAAA".into()],
        };
        let input = BuildResponsesBackend::to_input(&normalized);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "[user]\nhello");
        assert_eq!(input[0]["content"][1]["type"], "input_image");
        assert_eq!(
            input[0]["content"][1]["image_url"],
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn normalized_input_to_console_messages_shape() {
        let normalized = NormalizedChatInput {
            prompt: "[user]\nhi".into(),
            images: vec!["data:image/png;base64,BBBB".into()],
        };
        let messages = ConsoleMessagesBackend::to_messages(&normalized);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["type"], "text");
        assert_eq!(messages[0]["content"][0]["text"], "[user]\nhi");
        assert_eq!(messages[0]["content"][1]["type"], "image_url");
    }

    #[test]
    fn joins_delta_contents() {
        let deltas = vec![
            ChatDelta {
                index: 0,
                role: Some("assistant".into()),
                content: Some("hel".into()),
                finish_reason: None,
            },
            ChatDelta {
                index: 0,
                role: None,
                content: Some("lo".into()),
                finish_reason: None,
            },
            ChatDelta {
                index: 0,
                role: None,
                content: None,
                finish_reason: Some("stop".into()),
            },
        ];
        assert_eq!(join_deltas(&deltas), "hello");
    }
}
