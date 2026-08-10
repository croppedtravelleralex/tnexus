//! chat payload 构造 + 模型别名表（docs/39 主文档 §4.2，docs/39d §4.1/4.2）。
//!
//! OCR 别名 `grok-vision-ocr`（TNexus 新增，非 Go 现名，39 主文档 §4.2）：
//!   内部映射 `grok-chat-fast`；上游 payload 强制 `enableImageGeneration=false`
//!   + `enableImageStreaming=false`，保留 `fileAttachments`。
//!
//! 非 OCR 普通 chat：`enableImage = !TextOnly`（39c §4.2 `buildWebChatPayload`）；
//! G1 无 TextOnly 扩写 → 有图则 `enableImageGeneration=true`。

use serde_json::json;
use serde_json::Value;

use crate::attachments::FileAttachment;

/// OCR 对外模型别名（39 主文档 §4.2/§10）。
// 模型路由 / OCR 常量契约在 grok-domain::provider（端口层），此处 re-export 兼容旧路径。
pub use grok_domain::{public_models, ALIAS_OCR, DEFAULT_OCR_SYSTEM_PROMPT, UPSTREAM_OCR_MODEL};

/// grok.com 前端 chat payload（对齐 `scripts/grok_pure_http_client.py::chat_payload`）。
pub fn build_frontend_chat_payload(
    message: &str,
    file_attachments: &[Value],
    enable_image_generation: bool,
) -> Value {
    json!({
        "collectionIds": [],
        "disabledConnectorIds": [],
        "deviceEnvInfo": {
            "darkModeEnabled": false,
            "devicePixelRatio": 2,
            "screenHeight": 1328,
            "screenWidth": 2056,
            "viewportHeight": 1083,
            "viewportWidth": 2056,
        },
        "disableMemory": true,
        "disableSearch": false,
        "disableSelfHarmShortCircuit": false,
        "disableTextFollowUps": false,
        "enableImageGeneration": enable_image_generation,
        "enableImageStreaming": enable_image_generation,
        "enableSideBySide": true,
        "fileAttachments": file_attachments,
        "forceConcise": false,
        "forceSideBySide": false,
        "imageAttachments": [],
        "imageGenerationCount": if enable_image_generation { 2 } else { 0 },
        "isAsyncChat": false,
        "message": message,
        "modeId": "fast",
        "responseMetadata": {},
        "returnImageBytes": false,
        "returnRawGrokInXaiRequest": false,
        "sendFinalMetadata": true,
        "temporary": true,
    })
}

/// 构造 grok Web chat 上游 payload（对应 Go `buildWebChatPayload`）。
///
/// `system_prompt` 仅 OCR 时使用（§4.2 可配置默认）；非 OCR 沿用 normalized.prompt。
pub fn build_web_chat_payload(
    prompt_text: &str,
    attachments: &[FileAttachment],
    ocr: bool,
    system_prompt: &str,
) -> Value {
    let file_attachments: Vec<Value> = attachments
        .iter()
        .map(|a| {
            json!({
                "source_url": a.source_url,
                "file_name": a.file_name,
                "mime_type": a.mime_type,
                "data_base64": a.data_b64,
            })
        })
        .collect();

    // enableImage = !TextOnly：OCR 显式 false，普通带图请求 true。
    let enable_image_generation = if ocr { false } else { !attachments.is_empty() };

    let message = if ocr && !system_prompt.trim().is_empty() {
        format!("{system_prompt}\n\n{prompt_text}")
    } else {
        prompt_text.to_string()
    };

    build_frontend_chat_payload(&message, &file_attachments, enable_image_generation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_attachment() -> FileAttachment {
        FileAttachment {
            source_url: "https://x.com/a.png".to_string(),
            file_name: "attachment_1.png".to_string(),
            mime_type: "image/png".to_string(),
            data_b64: "aGVsbG8=".to_string(),
        }
    }

    #[test]
    fn ocr_payload_golden_model_and_flags() {
        let att = vec![sample_attachment()];
        let p = build_web_chat_payload("描述图片", &att, true, DEFAULT_OCR_SYSTEM_PROMPT);
        assert_eq!(p["modeId"], "fast");
        assert_eq!(p["enableImageGeneration"], false);
        assert_eq!(p["enableImageStreaming"], false);
        assert_eq!(p["fileAttachments"].as_array().unwrap().len(), 1);
        let msg = p["message"].as_str().unwrap();
        assert!(msg.contains(DEFAULT_OCR_SYSTEM_PROMPT));
        assert!(msg.contains("描述图片"));
    }

    #[test]
    fn non_ocr_with_images_enables_generation() {
        let att = vec![sample_attachment()];
        let p = build_web_chat_payload("hello", &att, false, "");
        assert_eq!(p["modeId"], "fast");
        assert_eq!(p["enableImageGeneration"], true);
        assert_eq!(p["fileAttachments"].as_array().unwrap().len(), 1);
        assert_eq!(p["message"], "hello");
    }

    #[test]
    fn non_ocr_text_only_disables_generation() {
        let p = build_web_chat_payload("just text", &[], false, "");
        assert_eq!(p["enableImageGeneration"], false, "无图 ⇒ 不触发生图");
        assert!(p["fileAttachments"].as_array().unwrap().is_empty());
    }

    #[test]
    fn ocr_without_attachments_still_no_generation() {
        let p = build_web_chat_payload("描述", &[], true, "");
        assert_eq!(p["modeId"], "fast");
        assert_eq!(p["enableImageGeneration"], false);
    }

    #[test]
    fn public_models_contains_ocr_alias() {
        let models = public_models();
        assert!(models.contains(&(ALIAS_OCR, UPSTREAM_OCR_MODEL)));
    }
}
