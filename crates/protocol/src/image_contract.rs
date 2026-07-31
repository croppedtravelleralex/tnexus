//! Phase B image/edit/estuary contract shapes (aligned with gptimage `chatgpt_web_request.py`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ContractOptions {
    pub timezone: String,
    pub timezone_offset_min: i32,
    pub contextual_seed: String,
    pub contextual_jitter: bool,
    /// Panda default: pure-HTTP auto-tool (`image_spa_tool_path=true`).
    pub spa_tool_path: bool,
    pub parent_message_id: String,
    pub fixed_message_id: Option<String>,
    pub fixed_create_time: Option<f64>,
}

impl Default for ContractOptions {
    fn default() -> Self {
        Self {
            timezone: "Asia/Shanghai".into(),
            timezone_offset_min: -480,
            contextual_seed: String::new(),
            contextual_jitter: true,
            spa_tool_path: true,
            parent_message_id: "client-created-root".into(),
            fixed_message_id: None,
            fixed_create_time: None,
        }
    }
}

impl ContractOptions {
    pub fn fixture() -> Self {
        Self {
            contextual_seed: "fixture-contextual-seed".into(),
            contextual_jitter: false,
            fixed_create_time: Some(1_730_000_000.0),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageEditRequest {
    #[serde(default = "default_image_model")]
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default = "default_n")]
    pub n: u32,
    #[serde(default = "default_size")]
    pub size: String,
}

fn default_image_model() -> String {
    "gpt-image-2".into()
}
fn default_n() -> u32 {
    1
}
fn default_size() -> String {
    "1024x1024".into()
}

/// picture_v2 mention prefix — must match Python `_picture_v2_prompt`.
pub fn picture_v2_prompt(prompt: &str) -> (String, Value) {
    const MENTION: &str = "@Create image";
    let raw = prompt.trim();
    let stripped = raw
        .strip_prefix(MENTION)
        .map(|s| s.trim_start_matches([' ', '\u{00a0}']))
        .unwrap_or(raw);
    let text = if stripped.is_empty() {
        MENTION.to_string()
    } else {
        format!("{MENTION}\u{00a0}{stripped}")
    };
    let offsets = json!([{
        "id": "picture_v2",
        "symbol": "ecosystemMention",
        "startIndex": 0,
        "endIndex": MENTION.len(),
    }]);
    (text, offsets)
}

pub fn build_prepare_contextual_info() -> Value {
    json!({
        "app_name": "chatgpt.com",
        "has_web_push_capabilities": true,
        "web_push_notification_permission": "default",
    })
}

/// Legacy field names for the verified pure-HTTP image tool route.
pub fn build_pure_http_image_contextual_info() -> Value {
    json!({
        "app_name": "chatgpt.com",
        "is_web_push_capable": true,
        "is_web_push_enabled": false,
    })
}

/// Matches Python `random.Random(int(hashlib.sha256(seed).hexdigest()[:16], 16))`.
pub fn build_client_contextual_info(seed: &str, jitter: bool, app_name: &str) -> Value {
    struct JitterRng {
        state: u64,
    }

    impl JitterRng {
        fn new(seed: &str, jitter: bool) -> Self {
            let state = if jitter {
                let material = if seed.is_empty() {
                    new_uuid()
                } else {
                    seed.to_string()
                };
                let digest = Sha256::digest(material.as_bytes());
                let hex = format!("{:x}", digest);
                u64::from_str_radix(&hex[..16], 16).unwrap_or(0)
            } else {
                0
            };
            Self { state }
        }

        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.state
        }

        fn rand_f(&mut self) -> f64 {
            (self.next_u64() as f64) / (u64::MAX as f64)
        }

        fn rand_i(&mut self, lo: i64, hi: i64) -> i64 {
            if hi <= lo {
                return lo;
            }
            lo + (self.rand_f() * ((hi - lo + 1) as f64)) as i64
        }

        fn pick(&mut self, choices: &[i64]) -> i64 {
            if choices.is_empty() {
                return 0;
            }
            let idx = (self.rand_f() * choices.len() as f64) as usize;
            choices[idx.min(choices.len() - 1)]
        }

        fn pick_f(&mut self, choices: &[f64]) -> f64 {
            if choices.is_empty() {
                return 0.0;
            }
            let idx = (self.rand_f() * choices.len() as f64) as usize;
            choices[idx.min(choices.len() - 1)]
        }
    }

    let mut rng = JitterRng::new(seed, jitter);
    let pixel_ratio: Value = if jitter {
        json!(rng.pick_f(&[1.0, 1.25, 1.5, 2.0]))
    } else {
        json!(2)
    };
    json!({
        "is_dark_mode": if jitter { rng.rand_f() < 0.35 } else { false },
        "time_since_loaded": if jitter { rng.rand_i(45, 980) } else { 120 },
        "page_height": if jitter { rng.pick(&[900, 1000, 1072, 1100, 1200]) } else { 900 },
        "page_width": if jitter { rng.pick(&[1280, 1400, 1440, 1724, 1920]) } else { 1400 },
        "pixel_ratio": pixel_ratio,
        "screen_height": if jitter { rng.pick(&[1080, 1200, 1440]) } else { 1440 },
        "screen_width": if jitter { rng.pick(&[1920, 2560, 1512]) } else { 2560 },
        "app_name": app_name,
        "has_web_push_capabilities": true,
        "web_push_notification_permission": "default",
    })
}

pub fn build_image_prepare_body(prompt: &str, model_slug: &str) -> Value {
    build_image_prepare_body_opts(prompt, model_slug, &ContractOptions::default())
}

pub fn build_image_prepare_body_opts(
    prompt: &str,
    model_slug: &str,
    opts: &ContractOptions,
) -> Value {
    let spa = opts.spa_tool_path;
    let (prepare_state, dispatch, source, hints, partial_part, contextual) = if spa {
        (
            "none",
            "debounced",
            "composer_editor_state",
            json!([]),
            prompt,
            build_pure_http_image_contextual_info(),
        )
    } else {
        (
            "sent",
            "immediate",
            "context_change",
            json!(["picture_v2"]),
            "Create image",
            build_prepare_contextual_info(),
        )
    };
    json!({
        "action": "next",
        "parent_message_id": opts.parent_message_id,
        "model": model_slug,
        "client_prepare_state": prepare_state,
        "client_prepare_dispatch": dispatch,
        "client_prepare_source": source,
        "timezone_offset_min": opts.timezone_offset_min,
        "timezone": opts.timezone,
        "conversation_mode": { "kind": "primary_assistant" },
        "system_hints": hints,
        "partial_query": {
            "id": opts.fixed_message_id.clone().unwrap_or_else(new_uuid),
            "author": { "role": "user" },
            "content": { "content_type": "text", "parts": [partial_part] },
        },
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "client_contextual_info": contextual,
    })
}

pub fn build_image_start_body(prompt: &str, model_slug: &str) -> Value {
    build_image_start_body_opts(prompt, model_slug, &ContractOptions::default())
}

pub fn build_image_start_body_opts(
    prompt: &str,
    model_slug: &str,
    opts: &ContractOptions,
) -> Value {
    build_image_start_body_with_refs_opts(prompt, model_slug, &[], opts)
}

pub fn build_image_start_body_with_refs(
    prompt: &str,
    model_slug: &str,
    refs: &[ImageRef],
) -> Value {
    build_image_start_body_with_refs_opts(prompt, model_slug, refs, &ContractOptions::default())
}

pub fn build_image_start_body_with_refs_opts(
    prompt: &str,
    model_slug: &str,
    refs: &[ImageRef],
    opts: &ContractOptions,
) -> Value {
    let spa = opts.spa_tool_path;
    let (prompt_part, symbol_offsets) = if spa {
        (prompt.to_string(), json!([]))
    } else {
        picture_v2_prompt(prompt)
    };
    let mut parts: Vec<Value> = refs
        .iter()
        .map(|r| {
            json!({
                "content_type": "image_asset_pointer",
                "asset_pointer": format!("file-service://{}", r.file_id),
                "width": r.width,
                "height": r.height,
                "size_bytes": r.file_size,
            })
        })
        .collect();
    parts.push(json!(prompt_part));

    let content = if refs.is_empty() {
        json!({ "content_type": "text", "parts": [prompt_part] })
    } else {
        json!({ "content_type": "multimodal_text", "parts": parts })
    };

    let top_hints = if spa {
        json!([])
    } else {
        json!(["picture_v2"])
    };
    let contextual = if spa {
        build_pure_http_image_contextual_info()
    } else {
        let seed = if opts.contextual_seed.is_empty() {
            prompt.chars().take(64).collect::<String>()
        } else {
            opts.contextual_seed.clone()
        };
        build_client_contextual_info(&seed, opts.contextual_jitter, "chatgpt.com")
    };

    let mut user_message = json!({
        "id": opts.fixed_message_id.clone().unwrap_or_else(new_uuid),
        "author": { "role": "user" },
        "content": content,
    });
    if !spa {
        let mut metadata = json!({
            "system_hints": ["picture_v2"],
            "serialization_metadata": { "custom_symbol_offsets": symbol_offsets },
        });
        if !refs.is_empty() {
            metadata["attachments"] = json!(refs
                .iter()
                .map(|r| {
                    json!({
                        "id": r.file_id,
                        "mimeType": r.mime_type,
                        "name": r.file_name,
                        "size": r.file_size,
                        "width": r.width,
                        "height": r.height,
                    })
                })
                .collect::<Vec<_>>());
        }
        if let Some(obj) = user_message.as_object_mut() {
            obj.insert(
                "create_time".into(),
                json!(opts.fixed_create_time.unwrap_or_else(current_unix_time)),
            );
            obj.insert("metadata".into(), metadata);
        }
    }

    json!({
        "action": "next",
        "messages": [user_message],
        "parent_message_id": opts.parent_message_id,
        "model": model_slug,
        "client_prepare_state": "none",
        "timezone_offset_min": opts.timezone_offset_min,
        "timezone": opts.timezone,
        "conversation_mode": { "kind": "primary_assistant" },
        "enable_message_followups": true,
        "system_hints": top_hints,
        "supports_buffering": true,
        "supported_encodings": ["v1"],
        "client_contextual_info": contextual,
        "paragen_cot_summary_display_override": "allow",
        "force_parallel_switch": "auto",
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRef {
    pub file_id: String,
    pub mime_type: String,
    pub file_name: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
}

pub fn build_estuary_download_headers(access_token: &str) -> Value {
    json!({
        "Accept": "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
        "Authorization": format!("Bearer {access_token}"),
    })
}

pub fn validate_estuary_headers(headers: &Value) -> Result<(), &'static str> {
    let auth = header_value_ci(headers, "Authorization").unwrap_or_default();
    if !auth.starts_with("Bearer ") {
        return Err("estuary requires Bearer Authorization on API session");
    }
    let token = auth.trim_start_matches("Bearer ").trim();
    if token.is_empty() {
        return Err("estuary requires non-empty Bearer token");
    }
    Ok(())
}

pub fn validate_resource_put_headers(headers: &Value) -> Result<(), &'static str> {
    for key in ["Authorization", "OAI-Device-Id", "OAI-Language"] {
        if header_value_ci(headers, key).is_some() {
            return Err("resource PUT must not include API session headers");
        }
    }
    Ok(())
}

fn header_value_ci(headers: &Value, key: &str) -> Option<String> {
    let obj = headers.as_object()?;
    let want = key.to_ascii_lowercase();
    obj.iter()
        .find(|(k, _)| k.to_ascii_lowercase() == want)
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.to_string())
}

pub fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

fn current_unix_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn path_for(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Deep-compare JSON for fixture tests; `volatile_paths` are skipped (dot paths).
pub fn assert_json_matches_except(built: &Value, golden: &Value, volatile_paths: &[&str]) {
    fn walk(path: &str, built: &Value, golden: &Value, volatile: &[&str]) {
        if volatile.contains(&path) {
            return;
        }
        match (built, golden) {
            (Value::Object(bo), Value::Object(go)) => {
                for (k, gv) in go {
                    let child = path_for(path, k);
                    let bv = bo.get(k).unwrap_or_else(|| panic!("missing key {child}"));
                    walk(&child, bv, gv, volatile);
                }
                for k in bo.keys() {
                    if !go.contains_key(k) {
                        panic!("extra key {}", path_for(path, k));
                    }
                }
            }
            (Value::Array(ba), Value::Array(ga)) => {
                assert_eq!(ba.len(), ga.len(), "array len at {path}");
                for (i, (bv, gv)) in ba.iter().zip(ga.iter()).enumerate() {
                    walk(&format!("{path}[{i}]"), bv, gv, volatile);
                }
            }
            _ => assert_eq!(built, golden, "value mismatch at {path}"),
        }
    }
    walk("", built, golden, volatile_paths);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_are_unique_per_build() {
        let a = build_image_start_body("p", "gpt-image-2");
        let b = build_image_start_body("p", "gpt-image-2");
        assert_ne!(a["messages"][0]["id"], b["messages"][0]["id"]);
    }

    #[test]
    fn picture_v2_prefix_matches_python_shape() {
        let (text, offsets) = picture_v2_prompt("a red cube");
        assert!(text.starts_with("@Create image"));
        assert_eq!(offsets[0]["id"], "picture_v2");
    }

    #[test]
    fn estuary_requires_bearer() {
        let ok = build_estuary_download_headers("REDACTED_TOKEN_VALUE");
        assert!(validate_estuary_headers(&ok).is_ok());
        assert!(validate_estuary_headers(&json!({})).is_err());
    }

    #[test]
    fn resource_put_rejects_bearer_case_insensitive() {
        assert!(validate_resource_put_headers(&json!({"Content-Type":"image/png"})).is_ok());
        assert!(validate_resource_put_headers(&json!({"authorization":"Bearer x"})).is_err());
    }
}
