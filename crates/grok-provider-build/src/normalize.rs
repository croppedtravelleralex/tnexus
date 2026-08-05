//! Responses 请求规整（对齐 Go `cli/normalize.go`，G5-P1 存储路径）。
//!
//! 语义：
//! - 强制 `model` = 调用方指定的上游模型（Go `normalizeResponsesRequest`）
//! - `response_format` → `text.format` 映射（json_object / json_schema 展开）
//! - `prompt_cache_key`：显式提供时不覆盖（Go `ensurePromptCacheKey`）
//!
//! G5-P1 不做 tools 兼容映射（Go `normalizeResponsesTools`）。

use serde_json::{Map, Value};

use crate::error::ProviderError;

/// 规整 Responses 请求体（对齐 Go `normalizeResponsesRequest`）。
///
/// 返回规整后的 JSON；解析失败返回 [`ProviderError::InvalidRequest`]。
pub fn normalize_responses_request(body: &Value, model: &str) -> Result<Value, ProviderError> {
    let Some(payload) = body.as_object() else {
        return Err(ProviderError::InvalidRequest("Responses 请求体必须是 JSON 对象".into()));
    };
    let mut payload = payload.clone();
    payload.insert("model".into(), Value::String(model.to_string()));

    if let Some(response_format) = payload.remove("response_format") {
        // text 字段存在且非 null → 解析为对象；否则新建。
        let mut text: Map<String, Value> = match payload.get("text") {
            Some(Value::Object(o)) => o.clone(),
            _ => Map::new(),
        };
        if is_empty_json(text.get("format")) {
            let formatted = normalize_response_format(response_format)?;
            text.insert("format".into(), formatted);
        }
        payload.insert("text".into(), Value::Object(text));
    }

    Ok(Value::Object(payload))
}

/// 规整 response_format（对齐 Go `normalizeResponseFormat`）。
///
/// `json_schema` 且内嵌 `json_schema` 非空时，把内嵌字段展开到顶层并保留
/// `type=json_schema`（上游不支持嵌套 `json_schema` 键）。
pub fn normalize_response_format(raw: Value) -> Result<Value, ProviderError> {
    let Some(mut format) = raw.as_object().cloned() else {
        return Ok(raw);
    };
    let format_type = format
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if format_type != "json_schema" || is_empty_json(format.get("json_schema")) {
        return Ok(Value::Object(format));
    }
    let schema = format
        .remove("json_schema")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut result = Map::new();
    result.insert("type".into(), Value::String("json_schema".into()));
    for (key, value) in schema {
        result.insert(key, value);
    }
    Ok(Value::Object(result))
}

/// 把粘滞键写入请求体；调用方已显式提供 `prompt_cache_key` 时不覆盖
/// （对齐 Go `ensurePromptCacheKey`）。
pub fn ensure_prompt_cache_key(body: &Value, key: &str) -> Result<Value, ProviderError> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(body.clone());
    }
    let Some(payload) = body.as_object() else {
        return Err(ProviderError::InvalidRequest(
            "解析 Responses 请求以写入 prompt_cache_key".into(),
        ));
    };
    let mut payload = payload.clone();
    let existing = payload.get("prompt_cache_key");
    if existing.is_none() || is_empty_json_value(existing.unwrap()) {
        payload.insert("prompt_cache_key".into(), Value::String(key.to_string()));
    }
    Ok(Value::Object(payload))
}

fn is_empty_json_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// 空 JSON：缺失 / null / 空串（Go `isEmptyJSON`）。
fn is_empty_json(raw: Option<&Value>) -> bool {
    match raw {
        None => true,
        Some(v) => is_empty_json_value(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_model_and_maps_response_format() {
        // Go TestNormalizeResponsesRequest
        let body = json!({
            "model": "public-model",
            "input": [{"type": "reasoning", "id": "old", "encrypted_content": "cipher"}, {"role": "user", "content": "hello"}],
            "prompt_cache_key": "official-key",
            "response_format": {"type": "json_object"},
        });
        let normalized = normalize_responses_request(&body, "grok-4.5").unwrap();
        assert_eq!(normalized["model"], "grok-4.5");
        assert_eq!(normalized["prompt_cache_key"], "official-key");
        let input = normalized["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["encrypted_content"], "cipher");
        assert!(normalized["text"]["format"].is_object(), "response_format mapped to text.format");
        assert!(normalized.get("response_format").is_none(), "response_format removed");
    }

    #[test]
    fn preserves_explicit_prompt_cache_key() {
        // Go TestNormalizeResponsesRequestPreservesExplicitPromptCacheKey
        let body = json!({"model": "public", "input": "hello", "prompt_cache_key": "official-key"});
        let normalized = normalize_responses_request(&body, "grok-4.5").unwrap();
        assert_eq!(normalized["prompt_cache_key"], "official-key");
    }

    #[test]
    fn does_not_invent_prompt_cache_key() {
        // Go TestNormalizeResponsesRequestDoesNotInventPromptCacheKey
        let body = json!({"model": "public", "input": "hello"});
        let normalized = normalize_responses_request(&body, "grok-4.5").unwrap();
        assert!(normalized.get("prompt_cache_key").is_none());
    }

    #[test]
    fn injects_derived_key_without_overriding_explicit() {
        // Go TestEnsurePromptCacheKeyInjectsDerivedKeyWithoutOverridingExplicit
        let body = json!({"model": "grok-4.5", "input": "hello"});
        let injected = ensure_prompt_cache_key(&body, "derived-key").unwrap();
        assert_eq!(injected["prompt_cache_key"], "derived-key");

        let body = json!({"model": "grok-4.5", "input": "hello", "prompt_cache_key": "official-key"});
        let preserved = ensure_prompt_cache_key(&body, "derived-key").unwrap();
        assert_eq!(preserved["prompt_cache_key"], "official-key");
    }

    #[test]
    fn flattens_json_schema() {
        // Go TestNormalizeResponsesRequestFlattensJSONSchema
        let body = json!({
            "model": "public",
            "input": "hello",
            "response_format": {"type": "json_schema", "json_schema": {"name": "answer", "strict": true, "schema": {"type": "object"}}},
        });
        let normalized = normalize_responses_request(&body, "grok-4.5").unwrap();
        let format = &normalized["text"]["format"];
        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["name"], "answer");
        assert!(format.get("json_schema").is_none(), "nested json_schema flattened");
        assert_eq!(format["strict"], true);
    }
}
