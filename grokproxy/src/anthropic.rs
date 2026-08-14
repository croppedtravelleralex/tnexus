//! Anthropic Messages API translation.
//!
//! NewAPI's Anthropic channel type talks `/v1/messages`, which differs from the
//! OpenAI shape in ways that matter:
//!
//! * the system prompt is a top-level field, not a message with `role: system`
//! * content is a list of typed blocks, not a bare string
//! * `max_tokens` is required
//! * the reply carries `stop_reason` rather than `finish_reason`, and usage is
//!   `input_tokens`/`output_tokens` rather than `prompt_`/`completion_tokens`
//!
//! The upstream only speaks the OpenAI shape, so this module converts in both
//! directions rather than adding a second upstream client.

use serde_json::{json, Map, Value};

/// Anthropic requires `max_tokens`; the upstream does not, so a request that
/// somehow arrives without one still needs a defensible ceiling.
const DEFAULT_MAX_TOKENS: i64 = 4096;

/// Anthropic request → OpenAI chat request.
pub fn request_to_openai(anthropic: &Value) -> Value {
    let mut messages: Vec<Value> = Vec::new();

    // A system prompt arrives beside the messages, not inside them.
    if let Some(system) = anthropic.get("system") {
        let text = flatten_content(system);
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    for message in anthropic
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = message.get("content");

        match content {
            Some(Value::Array(blocks)) if role == "assistant" => {
                let text = blocks
                    .iter()
                    .filter_map(|block| match block {
                        Value::Object(map)
                            if map.get("type").and_then(Value::as_str) == Some("text") =>
                        {
                            map.get("text").and_then(Value::as_str)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let tool_calls: Vec<Value> = blocks
                    .iter()
                    .filter_map(|block| {
                        let map = block.as_object()?;
                        if map.get("type").and_then(Value::as_str) != Some("tool_use") {
                            return None;
                        }
                        let id = map.get("id").and_then(Value::as_str)?;
                        let name = map.get("name").and_then(Value::as_str)?;
                        let input = map.get("input").cloned().unwrap_or(json!({}));
                        Some(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                            },
                        }))
                    })
                    .collect();
                let mut assistant = json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    assistant
                        .as_object_mut()
                        .expect("assistant object")
                        .insert("tool_calls".into(), Value::Array(tool_calls));
                }
                messages.push(assistant);
            }
            Some(Value::Array(blocks)) if role == "user" => {
                let mut text_parts: Vec<&str> = Vec::new();
                for block in blocks {
                    let Some(map) = block.as_object() else {
                        continue;
                    };
                    match map.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = map.get("text").and_then(Value::as_str) {
                                text_parts.push(text);
                            }
                        }
                        Some("tool_result") => {
                            if !text_parts.is_empty() {
                                messages.push(json!({
                                    "role": "user",
                                    "content": text_parts.join("\n"),
                                }));
                                text_parts.clear();
                            }
                            let tool_use_id = map
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let result =
                                map.get("content").map(flatten_content).unwrap_or_default();
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": result,
                            }));
                        }
                        _ => {}
                    }
                }
                if !text_parts.is_empty() {
                    messages.push(json!({
                        "role": "user",
                        "content": text_parts.join("\n"),
                    }));
                }
            }
            _ => {
                let text = content.map(flatten_content).unwrap_or_default();
                messages.push(json!({"role": role, "content": text}));
            }
        }
    }

    let mut out = Map::new();
    out.insert(
        "model".into(),
        anthropic.get("model").cloned().unwrap_or(Value::Null),
    );
    out.insert("messages".into(), Value::Array(messages));
    out.insert(
        "max_tokens".into(),
        anthropic
            .get("max_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_MAX_TOKENS)
            .into(),
    );
    // Only forward sampling knobs that were actually set, so the upstream keeps
    // its own defaults for the rest.
    for key in ["temperature", "top_p", "stop_sequences", "stream"] {
        if let Some(value) = anthropic.get(key) {
            let key = if key == "stop_sequences" { "stop" } else { key };
            out.insert(key.into(), value.clone());
        }
    }
    if let Some(tools) = anthropic.get("tools") {
        out.insert("tools".into(), tools.clone());
    }
    Value::Object(out)
}

/// OpenAI chat response → Anthropic message response.
pub fn response_to_anthropic(openai: &Value, model: &str) -> Value {
    let choice = openai
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first());
    let message = choice.and_then(|choice| choice.get("message"));
    let text = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let finish = choice
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .unwrap_or("stop");

    let mut content_blocks: Vec<Value> = Vec::new();
    if !text.is_empty() {
        content_blocks.push(json!({"type": "text", "text": text}));
    }
    if let Some(tool_calls) = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array)
    {
        for tool_call in tool_calls {
            let id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = tool_call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = tool_call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input = serde_json::from_str(args).unwrap_or(json!({}));
            content_blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }));
        }
    }
    if content_blocks.is_empty() {
        content_blocks.push(json!({"type": "text", "text": ""}));
    }

    let usage = openai.get("usage");
    let read = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };

    json!({
        "id": openai.get("id").and_then(Value::as_str).unwrap_or("msg_grokproxy"),
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content_blocks,
        "stop_reason": stop_reason(finish),
        "stop_sequence": Value::Null,
        "usage": {
            "input_tokens": read("prompt_tokens"),
            "output_tokens": read("completion_tokens"),
        },
    })
}

/// Anthropic SSE from a complete message, so NewAPI's stream test does not
/// have to speak a different protocol than the non-stream path.
pub fn response_to_sse(message: &Value) -> String {
    let blocks = message
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut start = message.clone();
    if let Some(object) = start.as_object_mut() {
        object.insert("content".into(), json!([]));
        object.insert("stop_reason".into(), Value::Null);
        if let Some(usage) = object.get_mut("usage").and_then(Value::as_object_mut) {
            usage.insert("output_tokens".into(), json!(0));
        }
    }
    let output_tokens = message
        .pointer("/usage/output_tokens")
        .cloned()
        .unwrap_or(json!(0));
    let stop = message
        .get("stop_reason")
        .cloned()
        .unwrap_or(json!("end_turn"));

    let mut events = vec![format!(
        "event: message_start\ndata: {}\n\n",
        json!({"type": "message_start", "message": start})
    )];

    for (index, block) in blocks.iter().enumerate() {
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("text");
        match block_type {
            "tool_use" => {
                events.push(format!(
                    "event: content_block_start\ndata: {}\n\n",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {
                            "type": "tool_use",
                            "id": block.get("id").cloned().unwrap_or(Value::Null),
                            "name": block.get("name").cloned().unwrap_or(Value::Null),
                            "input": {},
                        },
                    })
                ));
                let input = block.get("input").cloned().unwrap_or(json!({}));
                events.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                        },
                    })
                ));
            }
            _ => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                events.push(format!(
                    "event: content_block_start\ndata: {}\n\n",
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": {"type": "text", "text": ""},
                    })
                ));
                events.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    json!({
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {"type": "text_delta", "text": text},
                    })
                ));
            }
        }
        events.push(format!(
            "event: content_block_stop\ndata: {}\n\n",
            json!({"type": "content_block_stop", "index": index})
        ));
    }

    events.push(format!(
        "event: message_delta\ndata: {}\n\n",
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": Value::Null}, "usage": {"output_tokens": output_tokens}})
    ));
    events.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".into());
    events.concat()
}

/// Anthropic's vocabulary for why generation ended.
fn stop_reason(finish_reason: &str) -> &'static str {
    match finish_reason {
        "length" => "max_tokens",
        "stop_sequence" => "stop_sequence",
        "tool_calls" | "function_call" => "tool_use",
        _ => "end_turn",
    }
}

/// Collapse Anthropic's content blocks into the plain string the upstream wants.
///
/// Content is either a bare string or a list of typed blocks. Non-text blocks
/// (images, tool results) have no OpenAI-string equivalent and are dropped
/// rather than serialised into the prompt as JSON noise.
fn flatten_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Value::String(text) => Some(text.as_str()),
                Value::Object(map) => match map.get("type").and_then(Value::as_str) {
                    Some("text") | None => map.get("text").and_then(Value::as_str),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Anthropic-shaped error body. Its clients key off `type`, so an OpenAI error
/// envelope here would read as a malformed response rather than a failure.
pub fn error_body(message: &str) -> Value {
    json!({
        "type": "error",
        "error": {"type": "api_error", "message": message},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_prompt_becomes_a_system_message() {
        let request = json!({
            "model": "grok-4.6",
            "system": "You are terse.",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "hi"}],
        });
        let openai = request_to_openai(&request);
        assert_eq!(
            openai["messages"],
            json!([
                {"role": "system", "content": "You are terse."},
                {"role": "user", "content": "hi"},
            ])
        );
        assert_eq!(openai["max_tokens"], 100);
    }

    #[test]
    fn content_blocks_collapse_to_text() {
        let request = json!({
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"},
            ]}],
        });
        let openai = request_to_openai(&request);
        assert_eq!(openai["messages"][0]["content"], "first\nsecond");
    }

    #[test]
    fn a_system_prompt_given_as_blocks_is_also_handled() {
        // Anthropic allows the same block form for `system`.
        let request = json!({
            "system": [{"type": "text", "text": "Be brief."}],
            "messages": [{"role": "user", "content": "hi"}],
        });
        let openai = request_to_openai(&request);
        assert_eq!(openai["messages"][0]["content"], "Be brief.");
    }

    #[test]
    fn non_text_blocks_are_dropped_not_serialised() {
        // An image block rendered as JSON would reach the model as prompt noise.
        let request = json!({
            "messages": [{"role": "user", "content": [
                {"type": "image", "source": {"data": "AAAA"}},
                {"type": "text", "text": "describe"},
            ]}],
        });
        let openai = request_to_openai(&request);
        assert_eq!(openai["messages"][0]["content"], "describe");
    }

    #[test]
    fn a_missing_max_tokens_still_yields_a_bounded_request() {
        let openai = request_to_openai(&json!({"messages": []}));
        assert_eq!(openai["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn only_the_sampling_knobs_that_were_set_are_forwarded() {
        let openai = request_to_openai(&json!({"messages": [], "temperature": 0.2}));
        assert_eq!(openai["temperature"], 0.2);
        assert!(openai.get("top_p").is_none(), "unset knobs stay unset");
    }

    #[test]
    fn stop_sequences_are_renamed_to_the_openai_key() {
        let openai = request_to_openai(&json!({"messages": [], "stop_sequences": ["END"]}));
        assert_eq!(openai["stop"], json!(["END"]));
    }

    #[test]
    fn the_reply_is_translated_including_usage_and_stop_reason() {
        let upstream = json!({
            "id": "chatcmpl-1",
            "choices": [{"message": {"role": "assistant", "content": "hello"},
                         "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15},
        });
        let out = response_to_anthropic(&upstream, "grok-4.6");
        assert_eq!(out["type"], "message");
        assert_eq!(out["role"], "assistant");
        assert_eq!(out["content"], json!([{"type": "text", "text": "hello"}]));
        assert_eq!(out["stop_reason"], "end_turn");
        assert_eq!(out["usage"]["input_tokens"], 12);
        assert_eq!(out["usage"]["output_tokens"], 3);
        assert_eq!(out["model"], "grok-4.6");
    }

    #[test]
    fn hitting_the_token_ceiling_is_reported_as_max_tokens() {
        let upstream = json!({"choices": [{"message": {"content": "trunc"},
                                           "finish_reason": "length"}]});
        assert_eq!(
            response_to_anthropic(&upstream, "grok-4.6")["stop_reason"],
            "max_tokens"
        );
    }

    #[test]
    fn an_empty_upstream_reply_still_produces_a_valid_message() {
        // A malformed body must not become a malformed response to the client.
        let out = response_to_anthropic(&json!({}), "grok-4.6");
        assert_eq!(out["content"], json!([{"type": "text", "text": ""}]));
        assert_eq!(out["usage"]["input_tokens"], 0);
        assert_eq!(out["stop_reason"], "end_turn");
    }

    #[test]
    fn errors_use_the_anthropic_envelope() {
        let body = error_body("pool empty");
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["message"], "pool empty");
    }

    #[test]
    fn tool_results_become_openai_tool_messages() {
        let request = json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_1", "content": "42"},
                ],
            }],
        });
        let openai = request_to_openai(&request);
        assert_eq!(openai["messages"][0]["role"], "tool");
        assert_eq!(openai["messages"][0]["tool_call_id"], "call_1");
        assert_eq!(openai["messages"][0]["content"], "42");
    }

    #[test]
    fn assistant_tool_use_is_forwarded() {
        let request = json!({
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "call_1", "name": "bash", "input": {"command": "ls"}},
                ],
            }],
        });
        let openai = request_to_openai(&request);
        assert_eq!(
            openai["messages"][0]["tool_calls"][0]["function"]["name"],
            "bash"
        );
    }

    #[test]
    fn openai_tool_calls_become_anthropic_tool_use() {
        let upstream = json!({
            "choices": [{
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"},
                    }],
                },
                "finish_reason": "tool_calls",
            }],
        });
        let out = response_to_anthropic(&upstream, "grok-4.6");
        assert_eq!(out["stop_reason"], "tool_use");
        assert_eq!(out["content"][0]["type"], "tool_use");
        assert_eq!(out["content"][0]["name"], "bash");
    }

    #[test]
    fn a_stream_reply_carries_the_text_in_a_delta() {
        let message = response_to_anthropic(
            &json!({
                "id": "chatcmpl-1",
                "choices": [{"message": {"content": "hello"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1},
            }),
            "grok-4.6",
        );
        let sse = response_to_sse(&message);
        assert!(sse.contains("event: message_start"));
        assert!(sse.contains("\"text\":\"hello\""));
        assert!(sse.contains("event: message_stop"));
    }
}
