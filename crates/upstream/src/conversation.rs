use serde_json::{json, Map, Value};
use uuid::Uuid;

pub const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

/// Map OpenAI image model name to upstream slug (`openai_backend_api.py::_image_model_slug`).
pub fn image_model_slug(model: &str) -> &'static str {
    match model.trim() {
        "" | "gpt-image-2" => "auto",
        "gpt-4o-image" => "gpt-4o-image",
        m if m.contains("codex") => "codex-gpt-image-2",
        _ => "auto",
    }
}

pub fn timezone_offset_min(tz_name: &str) -> i32 {
    match tz_name {
        "Asia/Shanghai" | "Asia/Chongqing" => -480,
        "Asia/Tokyo" => -540,
        "America/New_York" => 300,
        "Europe/London" => 0,
        _ => -480,
    }
}

fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

fn build_prepare_contextual_info() -> Value {
    json!({
        "app_name": "chatgpt.com",
        "has_web_push_capabilities": true,
        "web_push_notification_permission": "default",
    })
}

fn build_pure_http_image_contextual_info() -> Value {
    json!({
        "app_name": "chatgpt.com",
        "is_web_push_capable": true,
        "is_web_push_enabled": false,
    })
}

fn picture_v2_prompt(prompt: &str) -> (String, Vec<Value>) {
    let mention = "@Create image";
    let mut raw = prompt.trim().to_string();
    if raw.starts_with(mention) {
        raw = raw[mention.len()..]
            .trim_start_matches([' ', '\u{00a0}'])
            .to_string();
    }
    let text = if raw.is_empty() {
        mention.to_string()
    } else {
        format!("{mention}\u{00a0}{raw}")
    };
    let offsets = vec![json!({
        "id": "picture_v2",
        "symbol": "ecosystemMention",
        "startIndex": 0,
        "endIndex": mention.len(),
    })];
    (text, offsets)
}

/// SPA `/f/conversation/prepare` body (`build_image_prepare_body`, spa_tool_path=true).
pub fn build_image_prepare_body(
    prompt: &str,
    model_slug: &str,
    timezone: &str,
    spa_tool_path: bool,
) -> Value {
    let tz = if timezone.is_empty() {
        DEFAULT_TIMEZONE
    } else {
        timezone
    };
    let partial_text = if spa_tool_path {
        prompt
    } else {
        "Create image"
    };
    let hints: Vec<&str> = if spa_tool_path {
        vec![]
    } else {
        vec!["picture_v2"]
    };
    json!({
        "action": "next",
        "parent_message_id": "client-created-root",
        "model": model_slug,
        "client_prepare_state": if spa_tool_path { "none" } else { "sent" },
        "client_prepare_dispatch": if spa_tool_path { "debounced" } else { "immediate" },
        "client_prepare_source": if spa_tool_path { "composer_editor_state" } else { "context_change" },
        "timezone_offset_min": timezone_offset_min(tz),
        "timezone": tz,
        "conversation_mode": {"kind": "primary_assistant"},
        "system_hints": hints,
        "partial_query": {
            "id": new_uuid(),
            "author": {"role": "user"},
            "content": {"content_type": "text", "parts": [partial_text]},
        },
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "client_contextual_info": if spa_tool_path {
            build_pure_http_image_contextual_info()
        } else {
            build_prepare_contextual_info()
        },
    })
}

#[derive(Debug, Clone, Default)]
pub struct ImageReference {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub mime_type: String,
    pub file_name: String,
}

/// `/f/conversation` image start body (`build_image_start_body`).
pub fn build_image_start_body(
    prompt: &str,
    model_slug: &str,
    timezone: &str,
    references: &[ImageReference],
    spa_tool_path: bool,
) -> Value {
    let tz = if timezone.is_empty() {
        DEFAULT_TIMEZONE
    } else {
        timezone
    };
    let hints: Vec<&str> = if spa_tool_path {
        vec![]
    } else {
        vec!["picture_v2"]
    };
    let (prompt_part, custom_symbol_offsets) = if spa_tool_path {
        (prompt.to_string(), Vec::<Value>::new())
    } else {
        picture_v2_prompt(prompt)
    };

    let mut parts: Vec<Value> = references
        .iter()
        .map(|item| {
            json!({
                "content_type": "image_asset_pointer",
                "asset_pointer": format!("file-service://{}", item.file_id),
                "width": item.width,
                "height": item.height,
                "size_bytes": item.file_size,
            })
        })
        .collect();
    parts.push(json!(prompt_part));

    let content = if references.is_empty() {
        json!({"content_type": "text", "parts": [prompt_part]})
    } else {
        json!({"content_type": "multimodal_text", "parts": parts})
    };

    let mut user_message = Map::new();
    user_message.insert("id".into(), json!(new_uuid()));
    user_message.insert("author".into(), json!({"role": "user"}));
    user_message.insert("content".into(), content);

    if !spa_tool_path {
        let metadata = json!({
            "system_hints": hints,
            "serialization_metadata": {"custom_symbol_offsets": custom_symbol_offsets},
        });
        user_message.insert(
            "create_time".into(),
            json!(chrono::Utc::now().timestamp_millis() as f64 / 1000.0),
        );
        user_message.insert("metadata".into(), metadata);
    }

    json!({
        "action": "next",
        "messages": [Value::Object(user_message)],
        "parent_message_id": "client-created-root",
        "model": model_slug,
        "client_prepare_state": "none",
        "timezone_offset_min": timezone_offset_min(tz),
        "timezone": tz,
        "conversation_mode": {"kind": "primary_assistant"},
        "enable_message_followups": true,
        "system_hints": hints,
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "client_contextual_info": if spa_tool_path {
            build_pure_http_image_contextual_info()
        } else {
            json!({
                "is_dark_mode": false,
                "time_since_loaded": 120,
                "page_height": 900,
                "page_width": 1400,
                "pixel_ratio": 2.0,
                "screen_height": 1440,
                "screen_width": 2560,
                "app_name": "chatgpt.com",
                "has_web_push_capabilities": true,
                "web_push_notification_permission": "default",
            })
        },
        "paragen_cot_summary_display_override": "allow",
        "force_parallel_switch": "auto",
    })
}

/// Minimal text conversation body (`chatgpt_web_request.py::build_chat_body`).
pub fn build_text_chat_body(prompt: &str, model: &str, timezone: &str) -> Value {
    let tz = if timezone.is_empty() {
        DEFAULT_TIMEZONE
    } else {
        timezone
    };
    let msg_id = new_uuid();
    json!({
        "action": "next",
        "messages": [{
            "id": msg_id,
            "author": {"role": "user"},
            "content": {"content_type": "text", "parts": [prompt]},
        }],
        "model": model,
        "parent_message_id": "client-created-root",
        "conversation_mode": {"kind": "primary_assistant"},
        "client_prepare_state": "none",
        "enable_message_followups": true,
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "system_hints": [],
        "timezone": tz,
        "timezone_offset_min": timezone_offset_min(tz),
        "paragen_cot_summary_display_override": "allow",
        "force_parallel_switch": "auto",
        "history_and_training_disabled": true,
        "client_contextual_info": {
            "is_dark_mode": false,
            "time_since_loaded": 120,
            "page_height": 900,
            "page_width": 1400,
            "pixel_ratio": 2.0,
            "screen_height": 1440,
            "screen_width": 2560,
            "app_name": "chatgpt.com",
            "has_web_push_capabilities": true,
            "web_push_notification_permission": "default",
        }
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn chat_body_has_parent_root() {
        let body = build_text_chat_body("hi", "auto", DEFAULT_TIMEZONE);
        assert_eq!(body["parent_message_id"], "client-created-root");
    }

    #[test]
    fn spa_image_prepare_matches_fixture_shape() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/protocol/image_prepare_body.json");
        let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let body =
            build_image_prepare_body("sunset over ocean", "gpt-image-2", DEFAULT_TIMEZONE, true);
        assert_eq!(body["action"], fixture["action"]);
        assert_eq!(body["system_hints"], fixture["system_hints"]);
        assert_eq!(
            body["client_contextual_info"],
            fixture["client_contextual_info"]
        );
        assert_eq!(
            body["partial_query"]["content"]["parts"][0],
            "sunset over ocean"
        );
    }

    #[test]
    fn spa_image_start_matches_fixture_shape() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/protocol/image_start_body.json");
        let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let body = build_image_start_body(
            "a red cube on white background",
            "gpt-image-2",
            DEFAULT_TIMEZONE,
            &[],
            true,
        );
        assert_eq!(body["system_hints"], fixture["system_hints"]);
        assert_eq!(
            body["client_contextual_info"],
            fixture["client_contextual_info"]
        );
        assert_eq!(
            body["messages"][0]["content"]["parts"][0],
            "a red cube on white background"
        );
    }

    #[test]
    fn image_model_slug_maps_gpt_image_2_to_auto() {
        assert_eq!(image_model_slug("gpt-image-2"), "auto");
    }
}
