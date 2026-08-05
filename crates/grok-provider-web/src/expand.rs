//! Prompt 扩写（image.go expandPrompt，docs/39d §4.2，docs/39 主文档 §2.2）。
//!
//! Go：`expandPrompt` → `openChatWithScope(TextOnly:true, ScopeWebExpand)`。
//! TextOnly:true 设置 `enableImageGeneration=false`（扩写只是文本生成，不应生图）。
//! ScopeWebExpand 为「仅并发闸门」，节点回退 grok_web（不入库）。
//!
//! G2 实现：通过 [`BridgeClient::fetch_chat`] 发一个标记为 `expand` 的文本对话，
//! 返回上游文本即为扩写后的 prompt。未接真实 bridge 时用 mock。

use std::sync::Arc;

use grok_domain::egress::Scope;
use serde_json::{json, Value};

use crate::bridge::BridgeClient;
use crate::error::ProviderError;

/// 扩写 system 指令（对齐 Go 扩写语义：只做润色/扩写，产出可直接生图的 prompt）。
pub const EXPAND_SYSTEM_PROMPT: &str =
    "You are a prompt enhancement assistant. Rewrite the user prompt into a rich, vivid, \
     detailed English image-generation prompt. Output ONLY the enhanced prompt, no preface.";

/// 调用 bridge 对 prompt 做扩写，返回扩展后的文本 prompt。
///
/// - 传入 `scope_web_expand`（Go ScopeWebExpand）供后续 egress 分类；G2 仅记录该语义，
///   实际走 `fetch_chat` 的 expand action。bridge 为 mock/真实统一。
/// - 任何桥接/上游失败返回 `ProviderError`，由调用方决定是否回退原 prompt。
pub async fn expand_prompt(
    bridge: &dyn BridgeClient,
    prompt: &Value,
    _scope_web_expand: Scope,
) -> Result<String, ProviderError> {
    let messages = vec![
        json!({"role": "system", "content": EXPAND_SYSTEM_PROMPT}),
        json!({"role": "user", "content": prompt.clone()}),
    ];
    let payload = json!({
        "action": "expand",
        "text_only": true,
        "messages": messages,
    });
    let expanded = bridge.fetch_chat(&payload).await?;
    let trimmed = expanded.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::Upstream(
            "empty prompt expansion".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}
