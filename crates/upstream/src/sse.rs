use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use regex::Regex;
use serde_json::Value;

/// Parsed SSE payload event (after `conversation.py` state machine).
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub conversation_id: String,
    pub parent_message_id: String,
    pub text: String,
    pub delta: String,
    pub file_ids: Vec<String>,
    pub sediment_ids: Vec<String>,
    pub blocked: bool,
    pub tool_invoked: Option<bool>,
    pub turn_use_case: String,
    pub raw: Option<Value>,
    pub done: bool,
}

/// Phase-1 text SSE ready: first `conversation_id` or assistant delta.
#[derive(Debug, Clone)]
pub struct TextSseReady {
    pub conversation_id: String,
    pub saw_delta: bool,
    pub event_count: usize,
}

/// Image SSE ready: first `file_id` or `sediment_id` in image tool context.
#[derive(Debug, Clone)]
pub struct ImageSseReady {
    pub conversation_id: String,
    pub file_ids: Vec<String>,
    pub sediment_ids: Vec<String>,
    pub event_count: usize,
}

#[derive(Debug, Default, Clone)]
pub struct ConversationState {
    pub text: String,
    pub raw_text: String,
    pub conversation_id: String,
    pub last_message_id: String,
    pub file_ids: Vec<String>,
    pub sediment_ids: Vec<String>,
    pub blocked: bool,
    pub tool_invoked: Option<bool>,
    pub turn_use_case: String,
}

static FILE_SERVICE_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
static REAL_IMAGE_FILE_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
static SEDIMENT_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
static CONVERSATION_ID_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

fn file_service_re() -> &'static Regex {
    FILE_SERVICE_ID_RE.get_or_init(|| Regex::new(r"file-service://([A-Za-z0-9_-]+)").unwrap())
}
fn real_image_file_re() -> &'static Regex {
    REAL_IMAGE_FILE_ID_RE.get_or_init(|| Regex::new(r"\bfile_00000000[a-f0-9]{24}\b").unwrap())
}
fn sediment_re() -> &'static Regex {
    SEDIMENT_ID_RE.get_or_init(|| Regex::new(r"sediment://([A-Za-z0-9_-]+)").unwrap())
}
fn conversation_id_re() -> &'static Regex {
    CONVERSATION_ID_RE.get_or_init(|| Regex::new(r#""conversation_id"\s*:\s*"([^"]+)""#).unwrap())
}

fn add_unique(values: &mut Vec<String>, candidates: &[String]) {
    for candidate in candidates {
        if !candidate.is_empty() && !values.contains(candidate) {
            values.push(candidate.clone());
        }
    }
}

pub fn extract_conversation_ids(payload: &str) -> (String, Vec<String>, Vec<String>) {
    let conversation_id = conversation_id_re()
        .captures(payload)
        .map(|c| c[1].to_string())
        .unwrap_or_default();
    let mut file_ids = Vec::new();
    add_unique(
        &mut file_ids,
        &file_service_re()
            .captures_iter(payload)
            .map(|c| c[1].to_string())
            .collect::<Vec<_>>(),
    );
    add_unique(
        &mut file_ids,
        &real_image_file_re()
            .find_iter(payload)
            .map(|m| m.as_str().to_string())
            .collect::<Vec<_>>(),
    );
    let sediment_ids = sediment_re()
        .captures_iter(payload)
        .map(|c| c[1].to_string())
        .collect();
    (conversation_id, file_ids, sediment_ids)
}

fn is_image_tool_event(event: &Value) -> bool {
    let message = event
        .get("message")
        .or_else(|| event.get("v").and_then(|v| v.get("message")));
    let Some(message) = message else { return false };
    let author = message.get("author").and_then(|a| a.get("role"));
    if author != Some(&Value::String("tool".into())) {
        return false;
    }
    if message
        .get("metadata")
        .and_then(|m| m.get("async_task_type"))
        .and_then(|v| v.as_str())
        == Some("image_gen")
    {
        return true;
    }
    if message
        .get("content")
        .and_then(|c| c.get("content_type"))
        .and_then(|v| v.as_str())
        != Some("multimodal_text")
    {
        return false;
    }
    message
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .map(|parts| {
            parts.iter().any(|part| {
                part.get("content_type")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "image_asset_pointer")
                    .unwrap_or(false)
                    || part
                        .get("asset_pointer")
                        .and_then(|v| v.as_str())
                        .map(|s| s.starts_with("file-service://") || s.starts_with("sediment://"))
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn is_user_message_event(event: &Value) -> bool {
    let message = event
        .get("message")
        .or_else(|| event.get("v").and_then(|v| v.get("message")));
    message
        .and_then(|m| m.get("author"))
        .and_then(|a| a.get("role"))
        .and_then(|r| r.as_str())
        .map(|r| r.eq_ignore_ascii_case("user"))
        .unwrap_or(false)
}

pub fn update_conversation_state(
    state: &mut ConversationState,
    payload: &str,
    event: Option<&Value>,
) {
    let (conversation_id, file_ids, sediment_ids) = extract_conversation_ids(payload);
    if !conversation_id.is_empty() && state.conversation_id.is_empty() {
        state.conversation_id = conversation_id;
    }
    let is_patch_event =
        event.is_some_and(|e| e.get("o").and_then(|v| v.as_str()) == Some("patch"));
    let is_user_msg = event.is_some_and(is_user_message_event);
    let image_context = event.is_some_and(is_image_tool_event)
        || (state.tool_invoked == Some(true) && !is_user_msg)
        || (is_patch_event
            && !is_user_msg
            && (payload.contains("asset_pointer") || payload.contains("file-service://")));
    if image_context {
        add_unique(&mut state.file_ids, &file_ids);
        add_unique(&mut state.sediment_ids, &sediment_ids);
    }
    let Some(event) = event else { return };
    if let Some(cid) = event.get("conversation_id").and_then(|v| v.as_str()) {
        if !cid.is_empty() {
            state.conversation_id = cid.to_string();
        }
    }
    if let Some(v) = event
        .get("v")
        .and_then(|v| v.get("conversation_id"))
        .and_then(|v| v.as_str())
    {
        if !v.is_empty() {
            state.conversation_id = v.to_string();
        }
    }
    let message = event
        .get("message")
        .or_else(|| event.get("v").and_then(|v| v.get("message")));
    if let Some(message) = message {
        let role = message
            .get("author")
            .and_then(|a| a.get("role"))
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let msg_id = message.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !msg_id.is_empty() && matches!(role, "assistant" | "tool") {
            state.last_message_id = msg_id.to_string();
        }
    }
    if event.get("type").and_then(|v| v.as_str()) == Some("moderation")
        && event
            .get("moderation_response")
            .and_then(|m| m.get("blocked"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
    {
        state.blocked = true;
    }
    if event.get("type").and_then(|v| v.as_str()) == Some("server_ste_metadata") {
        if let Some(metadata) = event.get("metadata") {
            if let Some(invoked) = metadata.get("tool_invoked").and_then(|v| v.as_bool()) {
                state.tool_invoked = Some(invoked);
            }
            if let Some(use_case) = metadata.get("turn_use_case").and_then(|v| v.as_str()) {
                state.turn_use_case = use_case.to_string();
            }
        }
    }
}

fn assistant_message_text(message: &Value) -> String {
    let parts = message
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());
    if let Some(parts) = parts {
        let text: String = parts.iter().filter_map(|p| p.as_str()).collect();
        if !text.is_empty() {
            return text;
        }
    }
    message
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

fn apply_patch_op(operation: &Value, current_text: &str) -> String {
    let op = operation.get("o").and_then(|v| v.as_str()).unwrap_or("");
    let value = operation.get("v").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "append" => format!("{current_text}{value}"),
        "replace" => value.to_string(),
        _ => current_text.to_string(),
    }
}

fn apply_text_patch(event: &Value, current_text: &str) -> String {
    if event.get("p").and_then(|v| v.as_str()) == Some("/message/content/parts/0") {
        return apply_patch_op(event, current_text);
    }
    if let Some(Value::String(chunk)) = event.get("v") {
        if !current_text.is_empty() && event.get("p").is_none() && event.get("o").is_none() {
            return format!("{current_text}{chunk}");
        }
    }
    if event.get("o").and_then(|v| v.as_str()) == Some("patch") {
        if let Some(Value::Array(ops)) = event.get("v") {
            let mut text = current_text.to_string();
            for item in ops {
                text = apply_text_patch(item, &text);
            }
            return text;
        }
    }
    if let Some(Value::Array(ops)) = event.get("v") {
        let mut text = current_text.to_string();
        for item in ops {
            text = apply_text_patch(item, &text);
        }
        return text;
    }
    current_text.to_string()
}

fn assistant_raw_text(event: &Value, current_text: &str) -> String {
    for candidate in [event, event.get("v").unwrap_or(&Value::Null)] {
        if !candidate.is_object() {
            continue;
        }
        if let Some(message) = candidate.get("message") {
            let role = message
                .get("author")
                .and_then(|a| a.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if role == "assistant" {
                let text = assistant_message_text(message);
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    apply_text_patch(event, current_text)
}

#[derive(Debug)]
pub struct SseParser {
    state: ConversationState,
    event_count: usize,
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            state: ConversationState::default(),
            event_count: 0,
        }
    }

    pub fn state(&self) -> &ConversationState {
        &self.state
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    /// Parse one `data:` payload line body (without `data:` prefix).
    pub fn feed_line(&mut self, payload: &str) -> Option<SseEvent> {
        let payload = payload.trim();
        if payload.is_empty() {
            return None;
        }
        self.event_count += 1;
        if payload == "[DONE]" {
            return Some(SseEvent {
                event_type: "conversation.done".into(),
                conversation_id: self.state.conversation_id.clone(),
                parent_message_id: self.state.last_message_id.clone(),
                text: self.state.text.clone(),
                delta: String::new(),
                file_ids: self.state.file_ids.clone(),
                sediment_ids: self.state.sediment_ids.clone(),
                blocked: self.state.blocked,
                tool_invoked: self.state.tool_invoked,
                turn_use_case: self.state.turn_use_case.clone(),
                raw: None,
                done: true,
            });
        }

        match serde_json::from_str::<Value>(payload) {
            Ok(event) => {
                update_conversation_state(&mut self.state, payload, Some(&event));
                let next_raw = assistant_raw_text(&event, &self.state.raw_text);
                let next_text = next_raw.clone();
                self.state.raw_text = next_raw;
                if next_text != self.state.text {
                    let delta = if next_text.starts_with(&self.state.text) {
                        next_text[self.state.text.len()..].to_string()
                    } else {
                        next_text.clone()
                    };
                    self.state.text = next_text;
                    return Some(SseEvent {
                        event_type: "conversation.delta".into(),
                        conversation_id: self.state.conversation_id.clone(),
                        parent_message_id: self.state.last_message_id.clone(),
                        text: self.state.text.clone(),
                        delta,
                        file_ids: self.state.file_ids.clone(),
                        sediment_ids: self.state.sediment_ids.clone(),
                        blocked: self.state.blocked,
                        tool_invoked: self.state.tool_invoked,
                        turn_use_case: self.state.turn_use_case.clone(),
                        raw: Some(event),
                        done: false,
                    });
                }
                Some(SseEvent {
                    event_type: "conversation.event".into(),
                    conversation_id: self.state.conversation_id.clone(),
                    parent_message_id: self.state.last_message_id.clone(),
                    text: self.state.text.clone(),
                    delta: String::new(),
                    file_ids: self.state.file_ids.clone(),
                    sediment_ids: self.state.sediment_ids.clone(),
                    blocked: self.state.blocked,
                    tool_invoked: self.state.tool_invoked,
                    turn_use_case: self.state.turn_use_case.clone(),
                    raw: Some(event),
                    done: false,
                })
            }
            Err(_) => {
                update_conversation_state(&mut self.state, payload, None);
                Some(SseEvent {
                    event_type: "conversation.raw".into(),
                    conversation_id: self.state.conversation_id.clone(),
                    parent_message_id: self.state.last_message_id.clone(),
                    text: self.state.text.clone(),
                    delta: String::new(),
                    file_ids: self.state.file_ids.clone(),
                    sediment_ids: self.state.sediment_ids.clone(),
                    blocked: self.state.blocked,
                    tool_invoked: self.state.tool_invoked,
                    turn_use_case: self.state.turn_use_case.clone(),
                    raw: None,
                    done: false,
                })
            }
        }
    }

    pub fn image_ready(&self) -> Option<ImageSseReady> {
        if self.state.file_ids.is_empty() && self.state.sediment_ids.is_empty() {
            return None;
        }
        Some(ImageSseReady {
            conversation_id: self.state.conversation_id.clone(),
            file_ids: self.state.file_ids.clone(),
            sediment_ids: self.state.sediment_ids.clone(),
            event_count: self.event_count,
        })
    }

    pub fn text_ready(&self) -> Option<TextSseReady> {
        let saw_delta = !self.state.text.is_empty();
        if !self.state.conversation_id.is_empty() || saw_delta {
            Some(TextSseReady {
                conversation_id: self.state.conversation_id.clone(),
                saw_delta,
                event_count: self.event_count,
            })
        } else {
            None
        }
    }
}

/// Split raw SSE bytes into `data:` payload strings (`utils/helper.py::iter_sse_payloads`).
pub fn split_sse_data_lines(chunk: &[u8], pending: &mut Vec<u8>) -> Vec<String> {
    pending.extend_from_slice(chunk);
    let mut out = Vec::new();
    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
        let line = pending.drain(..=pos).collect::<Vec<_>>();
        let line = String::from_utf8_lossy(&line)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if let Some(payload) = line.strip_prefix("data:") {
            let payload = payload.trim();
            if !payload.is_empty() {
                out.push(payload.to_string());
            }
        }
    }
    out
}

/// SSE consumption target (`upstream-probe::consume_sse_until_ready`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseConsumeMode {
    Text,
    Image,
}

/// Result of consuming an SSE stream until the target predicate is satisfied.
#[derive(Debug)]
pub struct ConsumedSse {
    pub parser: SseParser,
}

/// Consume an upstream SSE response until `text_ready` or `image_ready` (`upstream-probe`).
pub async fn consume_sse_until(
    resp: wreq::Response,
    mode: SseConsumeMode,
    timeout: Duration,
) -> Result<ConsumedSse> {
    let mut parser = SseParser::new();
    let mut pending = Vec::new();
    let started = Instant::now();
    let deadline = started + timeout;
    let mut stream = resp.into_data_stream();

    while Instant::now() < deadline {
        let next =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), stream.next()).await;
        match next {
            Ok(Some(Ok(chunk))) => {
                for payload in split_sse_data_lines(&chunk, &mut pending) {
                    if let Some(event) = parser.feed_line(&payload) {
                        match mode {
                            SseConsumeMode::Text => {
                                if parser.text_ready().is_some() {
                                    return Ok(ConsumedSse { parser });
                                }
                            }
                            SseConsumeMode::Image => {
                                if parser.image_ready().is_some() {
                                    return Ok(ConsumedSse { parser });
                                }
                            }
                        }
                        if event.done {
                            break;
                        }
                    }
                }
            }
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            Err(_) => break,
        }
    }

    match mode {
        SseConsumeMode::Text => {
            if parser.text_ready().is_some() {
                return Ok(ConsumedSse { parser });
            }
            bail!("sse ended before text ready predicate");
        }
        SseConsumeMode::Image => {
            if parser.image_ready().is_some() {
                return Ok(ConsumedSse { parser });
            }
            bail!("sse ended before image file_id predicate");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_conversation_id_from_payload() {
        let payload = r#"{"conversation_id":"abc-123","type":"message"}"#;
        let (cid, _, _) = extract_conversation_ids(payload);
        assert_eq!(cid, "abc-123");
    }

    #[test]
    fn parser_marks_text_ready_on_conversation_id() {
        let mut parser = SseParser::new();
        parser.feed_line(r#"{"conversation_id":"cid-1","type":"message"}"#);
        let ready = parser.text_ready().expect("ready");
        assert_eq!(ready.conversation_id, "cid-1");
    }
}
