//! OpenAI → grok 内部对话表示的归一化（纯协议逻辑，无 IO）。
//!
//! 端口自 Go `provider/web/chat.go` 的 `normalizeOpenAIInput` /
//! `contentTextAndImages` / `extractImageURL`，仅保留 chat / OCR 主路径
//! （docs/39d §4.2 OCR 函数链，docs/39c §2 G-OCR-*）。
//!
//! 边界：`prepareChatAttachments`、data URI 解码、远端 URL 大小/SSRF 校验
//! 属 provider-web / image-pipeline（有 IO），不在本 crate。

use serde::Deserialize;
use serde_json::Value;

use crate::error::{ConversationError, ConversationResult};
use crate::limits::{MAX_CHAT_IMAGE_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES};

/// OpenAI `POST /v1/chat/completions` 的 message（本 crate 只消费 role + content）。
#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    /// role：system / user / assistant。可省略。
    #[serde(default)]
    pub role: String,
    /// content：可为 JSON 字符串，或多模态 part 数组。
    pub content: Value,
    /// Go `type` 字段（function_call 等）。OCR 主路径不消费，保留字段兼容。
    #[serde(rename = "type", default)]
    pub type_name: String,
}

/// 归一化后的 grok 内部对话输入。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NormalizedChatInput {
    /// 按角色包裹后的纯文本（`[role]\ntext`）。
    pub prompt: String,
    /// 图片清单（image_url 的 URL 或 data URI），保持输入顺序。
    pub images: Vec<String>,
}

impl NormalizedChatInput {
    pub fn is_empty(&self) -> bool {
        self.prompt.trim().is_empty() && self.images.is_empty()
    }
}

/// 单条消息归一化结果（contentTextAndImages）。
struct MessageParts {
    text: String,
    images: Vec<String>,
}

/// 归一化 `messages` 到 grok 内部表示（chat 操作）。
pub fn normalize_chat_input(messages: Vec<ChatMessage>) -> ConversationResult<NormalizedChatInput> {
    if messages.is_empty() {
        return Err(ConversationError::EmptyMessages);
    }
    build_from_messages(messages)
}

/// 将 role+text 拼为 `[role]\ntext` 段并累积图片。
fn build_from_messages(messages: Vec<ChatMessage>) -> ConversationResult<NormalizedChatInput> {
    let mut prompt_parts = Vec::new();
    let mut images = Vec::new();

    for message in messages {
        let parts = content_text_and_images(&message.content)?;
        images.extend(parts.images);
        let role = message.role.trim();
        let text = parts.text.trim();
        if text.is_empty() {
            continue;
        }
        let role_lower = role.to_lowercase();
        prompt_parts.push(format!("[{}]\n{}", role_lower, text));
    }

    let prompt = prompt_parts.join("\n\n");
    validate_images(&images)?;
    Ok(NormalizedChatInput { prompt, images })
}

/// 图片数量与总大小校验（G-OCR-4 / G-OCR-5）。
fn validate_images(images: &[String]) -> ConversationResult<()> {
    if images.len() > MAX_CHAT_IMAGE_ATTACHMENTS {
        return Err(ConversationError::TooManyImages {
            max: MAX_CHAT_IMAGE_ATTACHMENTS,
        });
    }

    let mut total: u64 = 0;
    for image in images {
        if let Some(size) = data_uri_payload_bytes(image) {
            total = total.saturating_add(size);
            if total > MAX_TOTAL_IMAGE_BYTES {
                return Err(ConversationError::ImagesTooLarge {
                    max_bytes: MAX_TOTAL_IMAGE_BYTES,
                });
            }
        }
        // 远端 https URL 不在本层校验大小（pipeline 取图后复验，见 limits.rs）。
    }
    Ok(())
}

/// 计算 data URI 的原始字节数（base64 payload → 近似 ~3/4 长度）。非 data URI 返回 None。
fn data_uri_payload_bytes(value: &str) -> Option<u64> {
    // data URI 形如 `data:image/png;base64,<payload>`，分隔符为 `;base64,`。
    let rest = value.strip_prefix("data:")?;
    let payload = rest.split_once(";base64,")?.1;
    let encoded_len = payload.len() as u64;
    // base64 解码后 ≈ 编码长度 * 3/4（忽略 padding 精确数，仅作总量粗校验）
    Some(encoded_len * 3 / 4)
}

/// 从多模态 `content` 提取文本与图片清单。
///
/// - content 为字符串 → 返回该字符串，无图片。
/// - content 为 part 数组 → 逐个处理 text / input_text / output_text 与
///   image_url / input_image / image；`file_id`、`input_audio` / `file` /
///   `input_file`、未知 type 均返回明确错误（G-OCR-6）。
fn content_text_and_images(content: &Value) -> ConversationResult<MessageParts> {
    // 显式处理 null / 空。
    if content.is_null() {
        return Ok(MessageParts {
            text: String::new(),
            images: Vec::new(),
        });
    }

    // 字符串 content。
    if let Some(text) = content.as_str() {
        return Ok(MessageParts {
            text: text.to_string(),
            images: Vec::new(),
        });
    }

    // 数组 content。
    let parts = content
        .as_array()
        .ok_or(ConversationError::InvalidContent)?;
    let mut text_parts = Vec::new();
    let mut images = Vec::new();

    for part in parts {
        let type_name = part.get("type").and_then(Value::as_str).unwrap_or_default();
        match type_name {
            "text" | "input_text" | "output_text" => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text_parts.push(text.to_string());
                    }
                }
            }
            "image_url" | "input_image" | "image" => {
                let url = extract_image_url(part);
                if !url.is_empty() {
                    images.push(url);
                } else if let Some(file_id) = part.get("file_id").and_then(Value::as_str) {
                    if !file_id.is_empty() {
                        return Err(ConversationError::UnsupportedFileId);
                    }
                    return Err(ConversationError::MissingImageUrl);
                } else {
                    return Err(ConversationError::MissingImageUrl);
                }
            }
            // Go 明确列出的不支持类型：input_audio / file / input_file。
            "input_audio" | "file" | "input_file" => {
                return Err(ConversationError::UnsupportedContentType {
                    type_name: type_name.to_string(),
                });
            }
            other => {
                return Err(ConversationError::UnknownContentType {
                    type_name: other.to_string(),
                });
            }
        }
    }

    Ok(MessageParts {
        text: text_parts.join("\n"),
        images,
    })
}

/// 提取 part 的 image_url：支持字符串或 `{ "url": ... }` 对象。
fn extract_image_url(part: &Value) -> String {
    match part.get("image_url") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => map
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn msg(role: &str, content: Value) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content,
            type_name: String::new(),
        }
    }

    #[test]
    fn separates_text_and_image() {
        let content = json!([
            {"type": "text", "text": "描述这张图"},
            {"type": "image_url", "image_url": {"url": DATA_URI}},
        ]);
        let out = normalize_chat_input(vec![msg("user", content)]).unwrap();
        assert_eq!(out.prompt, "[user]\n描述这张图");
        assert_eq!(out.images, vec![DATA_URI.to_string()]);
    }

    #[test]
    fn responses_input_image() {
        let content = json!([
            {"type": "input_text", "text": "what is this"},
            {"type": "input_image", "image_url": DATA_URI},
        ]);
        let out = normalize_chat_input(vec![msg("user", content)]).unwrap();
        assert_eq!(out.prompt, "[user]\nwhat is this");
        assert_eq!(out.images, vec![DATA_URI.to_string()]);
    }

    #[test]
    fn input_file_rejected() {
        let content = json!([
            {"type": "input_file", "file_url": "https://example.com/a.pdf"},
        ]);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert!(matches!(
            err,
            ConversationError::UnsupportedContentType { .. }
        ));
        assert!(err.to_string().contains("input_file"));
    }

    #[test]
    fn input_image_file_id_rejected() {
        // G-OCR-6
        let content = json!([
            {"type": "input_image", "file_id": "file_123"},
        ]);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert_eq!(err, ConversationError::UnsupportedFileId);
    }

    #[test]
    fn image_missing_url_rejected() {
        let content = json!([{"type": "image_url"}]);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert_eq!(err, ConversationError::MissingImageUrl);
    }

    #[test]
    fn unknown_content_type_rejected() {
        let content = json!([{"type": "mystery", "text": "x"}]);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert!(matches!(err, ConversationError::UnknownContentType { .. }));
    }

    #[test]
    fn nine_images_rejected() {
        // G-OCR-4: 9 → 400（9 > 8）
        let parts: Vec<Value> = (0..9)
            .map(|_| json!({"type": "image_url", "image_url": {"url": DATA_URI}}))
            .collect();
        let content = Value::Array(parts);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert_eq!(
            err,
            ConversationError::TooManyImages {
                max: MAX_CHAT_IMAGE_ATTACHMENTS,
            }
        );
    }

    #[test]
    fn eight_images_accepted() {
        let parts: Vec<Value> = (0..8)
            .map(|_| json!({"type": "image_url", "image_url": {"url": DATA_URI}}))
            .collect();
        let out = normalize_chat_input(vec![msg("user", Value::Array(parts))]).unwrap();
        assert_eq!(out.images.len(), 8);
    }

    #[test]
    fn oversized_images_rejected() {
        // G-OCR-5: 超大 data URI（解码后 >64MiB）→ 400
        // base64 解码 ≈ 编码长度 * 3/4；需编码长度 * 3/4 > 64MiB → 编码长度 > ~89.5MiB
        // 取 90MiB 编码串：90M*3/4 = 67.5MiB > 64MiB
        let pad = "A".repeat(90 * 1024 * 1024);
        let url = format!("data:image/png;base64,{}", pad);
        let content = json!([{"type": "image_url", "image_url": {"url": url}}]);
        let err = normalize_chat_input(vec![msg("user", content)]).unwrap_err();
        assert!(matches!(err, ConversationError::ImagesTooLarge { .. }));
    }

    #[test]
    fn empty_text_content_yields_no_prompt_text() {
        let content = json!([{"type": "text", "text": ""}]);
        let out = normalize_chat_input(vec![msg("user", content)]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn string_content_returns_text_no_images() {
        let out = normalize_chat_input(vec![msg("user", json!("hello"))]).unwrap();
        assert_eq!(out.prompt, "[user]\nhello");
        assert!(out.images.is_empty());
    }

    #[test]
    fn multiple_messages_preserve_order_and_roles() {
        let sys = msg("system", json!("你是一名帮助理解图片的助手"));
        let usr = msg("user", json!("图里有什么"));
        let out = normalize_chat_input(vec![sys, usr]).unwrap();
        assert_eq!(
            out.prompt,
            "[system]\n你是一名帮助理解图片的助手\n\n[user]\n图里有什么"
        );
    }
}
