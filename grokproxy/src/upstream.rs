//! Upstream client for the free Build promo path (`cli-chat-proxy`).
//!
//! Two rules encoded here, both learned the hard way:
//!   * never pin an exact model id — upstream renames it without notice;
//!   * a rotated refresh token must reach the store, or the account dies.

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::model::Health;

pub const DEFAULT_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";
pub const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const CLIENT_VERSION: &str = "0.2.93";
/// Only used when `/models` says nothing usable.
pub const FALLBACK_MODEL: &str = "grok-4.6";

/// How an upstream failure should affect the account's health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Refresh token rejected — terminal until re-imported.
    Revoked,
    /// Entitlement denial; retrying this account is pointless for a while.
    Forbidden,
    /// Rate limited or out of quota — back off and retry later.
    Cooling(i64),
    /// Transport hiccup; the account itself is probably fine.
    Transient,
}

impl Failure {
    pub fn health(&self) -> Health {
        match self {
            Failure::Revoked => Health::NeedsReauth,
            Failure::Forbidden => Health::Forbidden,
            Failure::Cooling(_) | Failure::Transient => Health::Cooling,
        }
    }

    pub fn cooling_secs(&self) -> i64 {
        match self {
            Failure::Cooling(secs) => *secs,
            Failure::Transient => 30,
            _ => 0,
        }
    }
}

/// Map an upstream status + body onto a health decision.
///
/// `402`/`429` are transient-by-quota, `403` is an entitlement denial, and a
/// missing status (status 0) means the request never got an answer, which says
/// nothing about the account.
pub fn classify(status: u16, body: &str) -> Failure {
    let lower = body.to_ascii_lowercase();
    if lower.contains("invalid_grant") || lower.contains("refresh token has been revoked") {
        return Failure::Revoked;
    }
    match status {
        0 => Failure::Transient,
        401 => Failure::Revoked,
        402 => Failure::Cooling(1_800),
        403 => {
            if lower.contains("spending-limit") || lower.contains("insufficient") {
                Failure::Cooling(3_600)
            } else {
                Failure::Forbidden
            }
        }
        408 | 409 | 425 | 500 | 502 | 503 | 504 => Failure::Transient,
        429 => Failure::Cooling(600),
        _ => Failure::Transient,
    }
}

/// Newest plain `grok-<major>.<minor>` advertised by `/models`.
///
/// Suffixed variants are special-purpose aliases, not the promo chat model.
pub fn pick_chat_model(ids: &[String]) -> Option<String> {
    let mut best: Option<((u32, u32), String)> = None;
    for id in ids {
        let Some(version) = model_version_key(id) else {
            continue;
        };
        if best.as_ref().map(|(v, _)| version > *v).unwrap_or(true) {
            best = Some((version, id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

/// Semantic version for `grok-M.m` ids; other shapes sort last.
pub fn model_version_key(id: &str) -> Option<(u32, u32)> {
    let rest = id.strip_prefix("grok-")?;
    let (major, minor) = rest.split_once('.')?;
    let (Ok(major), Ok(minor)) = (major.parse::<u32>(), minor.parse::<u32>()) else {
        return None;
    };
    Some((major, minor))
}

/// Quota the upstream reports on every chat response.
///
/// There is no quota *endpoint* — /usage, /credits, /quota and /rest/rate-limits
/// all 404 — but the chat response carries `x-ratelimit-*` headers, which is
/// the only place remaining quota is observable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimit {
    pub limit_tokens: i64,
    pub remaining_tokens: i64,
    pub limit_requests: i64,
    pub remaining_requests: i64,
}

impl RateLimit {
    pub fn from_headers(headers: &reqwest::header::HeaderMap) -> Self {
        let read = |name: &str| -> i64 {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .and_then(|text| text.trim().parse::<i64>().ok())
                .unwrap_or(-1)
        };
        RateLimit {
            limit_tokens: read("x-ratelimit-limit-tokens"),
            remaining_tokens: read("x-ratelimit-remaining-tokens"),
            limit_requests: read("x-ratelimit-limit-requests"),
            remaining_requests: read("x-ratelimit-remaining-requests"),
        }
    }

    /// -1 means the header was absent, so "no data" is distinguishable from
    /// "genuinely zero left".
    pub fn observed(&self) -> bool {
        self.limit_tokens >= 0 || self.limit_requests >= 0
    }
}

/// A chat/responses call plus whatever quota the upstream disclosed.
#[derive(Debug)]
pub struct ChatOutcome {
    pub body: serde_json::Value,
    pub rate_limit: RateLimit,
}

#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

#[derive(Debug)]
pub struct UpstreamError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upstream {} {}", self.status, truncate(&self.body, 200))
    }
}

impl std::error::Error for UpstreamError {}

impl UpstreamError {
    pub fn failure(&self) -> Failure {
        classify(self.status, &self.body)
    }
}

pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

/// A token endpoint that has not answered in this long is not going to.
///
/// Chat needs a generous timeout, but reusing it for refresh makes every dead
/// account cost the full chat budget before the scheduler can move on — a
/// mostly-stale pool then fails requests purely on timeouts.
const REFRESH_TIMEOUT_SECS: u64 = 15;

/// Swap the host:port of a proxy URL while keeping scheme and credentials.
pub fn rewrite_proxy_host(proxy: &str, relay: &str) -> String {
    let (scheme, rest) = match proxy.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("http", proxy),
    };
    match rest.rsplit_once('@') {
        Some((credentials, _host)) => format!("{scheme}://{credentials}@{relay}"),
        None => format!("{scheme}://{relay}"),
    }
}

#[derive(Clone)]
pub struct Upstream {
    base_url: String,
    timeout: Duration,
    refresh_timeout: Duration,
    default_proxy: String,
    sticky_relay: String,
}

impl Upstream {
    pub fn new(base_url: impl Into<String>, timeout_secs: u64) -> Self {
        let timeout = Duration::from_secs(timeout_secs.max(5));
        Upstream {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            refresh_timeout: timeout.min(Duration::from_secs(REFRESH_TIMEOUT_SECS)),
            timeout,
            default_proxy: String::new(),
            sticky_relay: String::new(),
        }
    }

    #[cfg(test)]
    pub fn refresh_timeout(&self) -> Duration {
        self.refresh_timeout
    }

    /// Egress for accounts that carry no sticky `proxy_url` of their own.
    pub fn with_default_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.default_proxy = proxy.into().trim().to_string();
        self
    }

    /// Where the sticky relay actually listens, as `host:port`.
    ///
    /// Imported credentials carry a `proxy_url` whose host was whatever the
    /// minting deployment used (e.g. an old container's bridge gateway). The
    /// part worth keeping is the `user:pass`, which selects the sticky egress
    /// slot; the address is deployment-specific and must be overridable.
    pub fn with_sticky_relay(mut self, relay: impl Into<String>) -> Self {
        self.sticky_relay = relay.into().trim().to_string();
        self
    }

    /// Per-account sticky egress wins; the configured default is the fallback.
    pub fn effective_proxy(&self, account_proxy: &str) -> String {
        let trimmed = account_proxy.trim();
        if trimmed.is_empty() {
            return self.default_proxy.clone();
        }
        if self.sticky_relay.is_empty() {
            return trimmed.to_string();
        }
        rewrite_proxy_host(trimmed, &self.sticky_relay)
    }

    /// One client per call: each account may carry a different sticky egress,
    /// and reqwest bakes the proxy into the client.
    fn client(&self, proxy_url: &str) -> Result<reqwest::Client> {
        self.client_with_timeout(proxy_url, self.timeout)
    }

    fn client_with_timeout(&self, proxy_url: &str, timeout: Duration) -> Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(format!("grok-cli/{CLIENT_VERSION}"));
        let proxy = self.effective_proxy(proxy_url);
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(&proxy)?);
        }
        Ok(builder.build()?)
    }

    fn cli_headers(&self, extra: &serde_json::Value, access_token: &str) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        for (key, value) in [
            ("X-XAI-Token-Auth", "xai-grok-cli"),
            ("x-grok-client-version", CLIENT_VERSION),
            ("x-grok-client-identifier", "grok-shell"),
            ("x-grok-client-surface", "tui"),
            ("x-grok-client-name", "grok-shell"),
        ] {
            headers.insert(
                HeaderName::from_static_str_checked(key),
                HeaderValue::from_static(value),
            );
        }
        let (agent_id, session_id) = stable_client_identity(access_token);
        let req_id = random_hex(8);
        let conv_id = random_hex(16);
        let trace_id = random_hex(16);
        let span_id = random_hex(8);
        for (key, value) in [
            ("x-grok-agent-id", agent_id),
            ("x-grok-session-id", session_id.clone()),
            ("x-grok-conv-id", conv_id.clone()),
            ("x-grok-req-id", req_id.clone()),
            ("x-grok-conversation-id", conv_id),
            ("x-grok-session-id-legacy", session_id),
            ("x-grok-request-id", req_id),
            (
                "traceparent",
                format!("00-{trace_id}-{span_id}-01"),
            ),
            ("tracestate", String::new()),
        ] {
            if let Ok(parsed) = HeaderValue::from_str(&value) {
                headers.insert(HeaderName::from_static_str_checked(key), parsed);
            }
        }
        if let Some(map) = extra.as_object() {
            for (key, value) in map {
                let (Ok(name), Some(text)) = (HeaderName::try_from(key.as_str()), value.as_str())
                else {
                    continue;
                };
                if let Ok(parsed) = HeaderValue::from_str(text) {
                    headers.insert(name, parsed);
                }
            }
        }
        headers
    }

    pub async fn refresh_token(&self, refresh_token: &str, proxy_url: &str) -> Result<TokenPair> {
        let client = self.client_with_timeout(proxy_url, self.refresh_timeout)?;
        let response = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
            ])
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;

        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        let parsed: TokenResponse = serde_json::from_str(&text).map_err(|err| UpstreamError {
            status,
            body: format!("bad token json: {err}"),
        })?;
        let expires_at = crate::jwt::access_token_expiry(&parsed.access_token)
            .unwrap_or_else(|| crate::now() + parsed.expires_in.unwrap_or(21_600));
        Ok(TokenPair {
            access_token: parsed.access_token,
            // Empty means "upstream kept the old one" — the store treats it as no-op.
            refresh_token: parsed.refresh_token.unwrap_or_default(),
            expires_at,
        })
    }

    pub async fn list_models(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
    ) -> Result<Vec<String>> {
        let client = self.client(proxy_url)?;
        let response = client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(access_token)
            .headers(self.cli_headers(extra_headers, access_token))
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        let parsed: ModelsResponse =
            serde_json::from_str(&text).unwrap_or(ModelsResponse { data: Vec::new() });
        Ok(parsed
            .data
            .into_iter()
            .map(|entry| entry.id)
            .filter(|id| !id.is_empty())
            .collect())
    }

    /// Forward an OpenAI-shaped chat request, keeping the quota headers.
    pub async fn chat_completions(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> Result<ChatOutcome> {
        self.post_json(
            "chat/completions",
            access_token,
            proxy_url,
            extra_headers,
            payload,
        )
        .await
    }

    pub async fn responses(
        &self,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> Result<ChatOutcome> {
        self.post_json("responses", access_token, proxy_url, extra_headers, payload)
            .await
    }

    async fn post_json(
        &self,
        path: &str,
        access_token: &str,
        proxy_url: &str,
        extra_headers: &serde_json::Value,
        payload: &serde_json::Value,
    ) -> Result<ChatOutcome> {
        let client = self.client(proxy_url)?;
        let response = client
            .post(format!("{}/{}", self.base_url, path))
            .bearer_auth(access_token)
            .headers(self.cli_headers(extra_headers, access_token))
            .json(payload)
            .send()
            .await
            .map_err(|err| UpstreamError {
                status: 0,
                body: err.to_string(),
            })?;
        let status = response.status().as_u16();
        // Read quota before consuming the body; the headers are the only place
        // remaining tokens/requests are reported.
        let rate_limit = RateLimit::from_headers(response.headers());
        let text = response.text().await.unwrap_or_default();
        if status != 200 {
            return Err(UpstreamError { status, body: text }.into());
        }
        Ok(ChatOutcome {
            body: parse_chat_body(&text)?,
            rate_limit,
        })
    }
}

/// The Build upstream returns JSON for `stream: false` and SSE for
/// `stream: true`. Callers of this crate always ask for JSON, but if a
/// `stream: true` still leaks through we assemble the chunks rather than
/// failing with "expected value at line 1 column 1" on the `data:` prefix.
pub fn parse_chat_body(text: &str) -> Result<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("bad upstream json: empty body"));
    }
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if looks_like_sse(trimmed) {
        return assemble_sse(trimmed);
    }
    Err(anyhow!(
        "bad upstream json: not an object (prefix={:?})",
        truncate(trimmed, 80)
    ))
}

fn looks_like_sse(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("data:") || line.starts_with("event:")
    })
}

fn assemble_sse(text: &str) -> Result<serde_json::Value> {
    let mut content = String::new();
    let mut tool_calls: Vec<MergedToolCall> = Vec::new();
    let mut id = None;
    let mut model = None;
    let mut finish = String::from("stop");
    let mut usage = None;
    let mut saw_chunk = false;

    for line in text.lines() {
        let Some(data) = line.trim_start().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let chunk: serde_json::Value = serde_json::from_str(data).map_err(|err| {
            anyhow!(
                "bad upstream json: sse chunk: {err} (prefix={:?})",
                truncate(data, 80)
            )
        })?;
        if let Some(error) = chunk.get("error") {
            return Err(anyhow!("upstream stream error: {error}"));
        }
        // A full chat.completion object stuffed into one SSE event.
        if chunk.pointer("/choices/0/message").is_some() {
            return Ok(chunk);
        }
        saw_chunk = true;
        if id.is_none() {
            id = chunk
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
        if model.is_none() {
            model = chunk
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
        }
        if let Some(reported) = chunk.get("usage") {
            usage = Some(reported.clone());
        }
        if let Some(piece) = chunk
            .pointer("/choices/0/delta/content")
            .and_then(serde_json::Value::as_str)
        {
            content.push_str(piece);
        }
        if let Some(delta) = chunk.pointer("/choices/0/delta/tool_calls") {
            merge_tool_call_delta(&mut tool_calls, delta);
        }
        if let Some(reason) = chunk
            .pointer("/choices/0/finish_reason")
            .and_then(serde_json::Value::as_str)
        {
            if !reason.is_empty() {
                finish = reason.to_string();
            }
        }
    }

    if !saw_chunk {
        return Err(anyhow!(
            "bad upstream json: empty sse (prefix={:?})",
            truncate(text, 80)
        ));
    }

    let mut message = serde_json::json!({"role": "assistant", "content": content});
    let built_tool_calls = build_tool_calls_json(&tool_calls);
    if !built_tool_calls.is_empty() {
        message["tool_calls"] = serde_json::Value::Array(built_tool_calls);
    }

    Ok(serde_json::json!({
        "id": id.unwrap_or_else(|| "chatcmpl-grokproxy".into()),
        "object": "chat.completion",
        "model": model.unwrap_or_else(|| FALLBACK_MODEL.to_string()),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish,
        }],
        "usage": usage.unwrap_or_else(|| serde_json::json!({
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
        })),
    }))
}

/// Whether the caller asked for SSE. The upstream is always called with
/// `stream: false`; this flag only changes how we dress the reply.
pub fn wants_stream(payload: &serde_json::Value) -> bool {
    payload
        .get("stream")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// The Build chat proxy answers `stream: true` with SSE (or an empty 200).
/// We never ask it for a stream; if the client wanted one, we synthesize SSE
/// from the complete JSON after the fact.
pub fn disable_streaming(payload: &mut serde_json::Value) {
    if let Some(object) = payload.as_object_mut() {
        object.insert("stream".into(), serde_json::Value::Bool(false));
        object.remove("stream_options");
    }
}

/// One-shot OpenAI SSE from a complete chat.completion body, so a streaming
/// client (NewAPI's model test, playground) still sees `data:` events.
///
/// Strict clients (pi, OpenAI SDK) expect the first chunk to carry only
/// `delta.role`, then separate chunks for `delta.content` / `delta.tool_calls`.
pub fn completion_to_sse(body: &serde_json::Value) -> String {
    let id = body
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("chatcmpl-grokproxy");
    let model = body
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(FALLBACK_MODEL);
    let message = body.pointer("/choices/0/message");
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|c| match c {
            serde_json::Value::String(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("");
    let tool_calls = message.and_then(|m| m.get("tool_calls")).cloned();
    let finish = body
        .pointer("/choices/0/finish_reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("stop");
    let created = body.get("created").cloned().unwrap_or(serde_json::json!(0));
    let usage = body.get("usage").cloned();

    let mut events: Vec<String> = Vec::new();
    let role_chunk = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant"},
            "finish_reason": serde_json::Value::Null,
        }],
    });
    events.push(format!("data: {role_chunk}\n\n"));

    if !content.is_empty() {
        let content_chunk = serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {"content": content},
                "finish_reason": serde_json::Value::Null,
            }],
        });
        events.push(format!("data: {content_chunk}\n\n"));
    }

    if let Some(tool_calls) = tool_calls {
        append_tool_call_sse_chunks(
            &mut events,
            id,
            model,
            &created,
            tool_calls,
        );
    }

    let mut last = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": finish,
        }],
    });
    if let Some(usage) = usage {
        last.as_object_mut()
            .expect("object just constructed")
            .insert("usage".into(), usage);
    }
    events.push(format!("data: {last}\n\n"));
    events.push("data: [DONE]\n\n".into());
    events.concat()
}

#[derive(Default)]
struct MergedToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn merge_tool_call_delta(calls: &mut Vec<MergedToolCall>, delta: &serde_json::Value) {
    let Some(items) = delta.as_array() else {
        return;
    };
    for item in items {
        let index = item.get("index").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
        while calls.len() <= index {
            calls.push(MergedToolCall::default());
        }
        let slot = &mut calls[index];
        if let Some(id) = item.get("id").and_then(serde_json::Value::as_str) {
            slot.id = Some(id.to_string());
        }
        if let Some(name) = item.pointer("/function/name").and_then(serde_json::Value::as_str) {
            slot.name = Some(name.to_string());
        }
        if let Some(args) = item.pointer("/function/arguments").and_then(serde_json::Value::as_str)
        {
            slot.arguments.push_str(args);
        }
    }
}

fn build_tool_calls_json(calls: &[MergedToolCall]) -> Vec<serde_json::Value> {
    calls
        .iter()
        .enumerate()
        .filter_map(|(index, call)| {
            let name = call.name.as_deref()?;
            Some(serde_json::json!({
                "id": call.id.clone().unwrap_or_else(|| format!("call_grokproxy_{index}")),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": if call.arguments.is_empty() { "{}" } else { &call.arguments },
                },
            }))
        })
        .collect()
}

fn append_tool_call_sse_chunks(
    events: &mut Vec<String>,
    id: &str,
    model: &str,
    created: &serde_json::Value,
    tool_calls: serde_json::Value,
) {
    let Some(items) = tool_calls.as_array() else {
        return;
    };
    for (index, call) in items.iter().enumerate() {
        let id_val = call
            .get("id")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(format!("call_grokproxy_{index}")));
        let name = call
            .pointer("/function/name")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let args = call
            .pointer("/function/arguments")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("{}");
        let start = serde_json::json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": index,
                        "id": id_val,
                        "type": "function",
                        "function": {"name": name, "arguments": ""},
                    }],
                },
                "finish_reason": serde_json::Value::Null,
            }],
        });
        events.push(format!("data: {start}\n\n"));
        if !args.is_empty() && args != "{}" {
            let args_chunk = serde_json::json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": index,
                            "function": {"arguments": args},
                        }],
                    },
                    "finish_reason": serde_json::Value::Null,
                }],
            });
            events.push(format!("data: {args_chunk}\n\n"));
        }
    }
}

fn random_hex(bytes_length: usize) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..bytes_length).map(|_| rng.gen()).collect();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn stable_client_identity(access_token: &str) -> (String, String) {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut agent_hasher = DefaultHasher::new();
    access_token.hash(&mut agent_hasher);
    "agent".hash(&mut agent_hasher);
    let agent_id = format!("{:032x}", agent_hasher.finish());

    let mut session_hasher = DefaultHasher::new();
    access_token.hash(&mut session_hasher);
    "session".hash(&mut session_hasher);
    let session_bits = session_hasher.finish();
    let session_id = format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (session_bits >> 32) as u32,
        ((session_bits >> 16) & 0xffff) as u16,
        (session_bits & 0xffff) as u16,
        ((session_bits >> 48) & 0xffff) as u16,
        session_bits & 0xffffffffffff
    );
    (agent_id, session_id)
}

/// `HeaderName::from_static` panics on a bad name; this keeps startup safe.
trait HeaderNameExt {
    fn from_static_str_checked(value: &'static str) -> reqwest::header::HeaderName;
}

impl HeaderNameExt for reqwest::header::HeaderName {
    fn from_static_str_checked(value: &'static str) -> reqwest::header::HeaderName {
        reqwest::header::HeaderName::try_from(value)
            .unwrap_or(reqwest::header::HeaderName::from_static("x-grok-unused"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn newest_model_wins() {
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.5", "grok-4.6"])).as_deref(),
            Some("grok-4.6")
        );
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.6", "grok-4.10"])).as_deref(),
            Some("grok-4.10")
        );
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.9", "grok-5.0"])).as_deref(),
            Some("grok-5.0")
        );
    }

    #[test]
    fn suffixed_aliases_do_not_win() {
        assert_eq!(
            pick_chat_model(&ids(&["grok-4.20-0309-non-reasoning", "grok-4.6"])).as_deref(),
            Some("grok-4.6")
        );
    }

    #[test]
    fn no_usable_model_is_none() {
        assert_eq!(pick_chat_model(&ids(&["gpt-4o", "grok-3-mini"])), None);
        assert_eq!(pick_chat_model(&[]), None);
    }

    #[test]
    fn revoked_refresh_is_terminal() {
        let failure = classify(400, r#"{"error":"invalid_grant"}"#);
        assert_eq!(failure, Failure::Revoked);
        assert_eq!(failure.health(), Health::NeedsReauth);
    }

    #[test]
    fn permission_denied_is_not_a_cooldown() {
        let failure = classify(403, r#"{"code":"permission-denied"}"#);
        assert_eq!(failure, Failure::Forbidden);
        assert_eq!(failure.health(), Health::Forbidden);
    }

    #[test]
    fn spending_limit_cools_instead_of_banning() {
        let failure = classify(403, "personal-team-blocked:spending-limit");
        assert!(matches!(failure, Failure::Cooling(_)));
    }

    #[test]
    fn quota_and_rate_limit_cool_down() {
        assert!(matches!(classify(402, "no credit"), Failure::Cooling(_)));
        assert!(matches!(classify(429, "slow down"), Failure::Cooling(_)));
    }

    #[test]
    fn unanswered_request_blames_the_network_not_the_account() {
        let failure = classify(0, "connection reset");
        assert_eq!(failure, Failure::Transient);
        assert_eq!(failure.health(), Health::Cooling);
        assert!(failure.cooling_secs() < 60);
    }

    #[test]
    fn server_errors_are_transient() {
        for status in [500u16, 502, 503, 504] {
            assert_eq!(classify(status, "boom"), Failure::Transient);
        }
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("中文很长的内容", 2), "中文…");
    }

    #[test]
    fn account_sticky_proxy_beats_the_default() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5).with_default_proxy("http://default:1");
        assert_eq!(
            upstream.effective_proxy("http://sticky:2"),
            "http://sticky:2"
        );
    }

    #[test]
    fn default_proxy_covers_accounts_without_one() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5).with_default_proxy("http://default:1");
        assert_eq!(upstream.effective_proxy(""), "http://default:1");
        assert_eq!(upstream.effective_proxy("   "), "http://default:1");
    }

    #[test]
    fn no_proxy_configured_means_direct() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5);
        assert_eq!(upstream.effective_proxy(""), "");
    }

    #[test]
    fn refresh_gives_up_long_before_the_chat_budget() {
        // A stale pool must not spend the full chat timeout per dead account.
        let upstream = Upstream::new(DEFAULT_BASE_URL, 120);
        assert_eq!(
            upstream.refresh_timeout(),
            Duration::from_secs(REFRESH_TIMEOUT_SECS)
        );
    }

    #[test]
    fn a_short_chat_timeout_also_shortens_refresh() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 8);
        assert_eq!(upstream.refresh_timeout(), Duration::from_secs(8));
    }

    fn headers_from(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (key, value) in pairs {
            map.insert(
                reqwest::header::HeaderName::try_from(*key).unwrap(),
                reqwest::header::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn quota_is_read_from_the_real_header_names() {
        // Observed on a live chat response; this is the only place the free
        // Build tier discloses remaining quota.
        let quota = RateLimit::from_headers(&headers_from(&[
            ("x-ratelimit-limit-tokens", "1000000"),
            ("x-ratelimit-remaining-tokens", "994300"),
            ("x-ratelimit-limit-requests", "21"),
            ("x-ratelimit-remaining-requests", "18"),
        ]));
        assert_eq!(quota.limit_tokens, 1_000_000);
        assert_eq!(quota.remaining_tokens, 994_300);
        assert_eq!(quota.remaining_requests, 18);
        assert!(quota.observed());
    }

    #[test]
    fn absent_headers_mean_unknown_not_zero() {
        let quota = RateLimit::from_headers(&headers_from(&[]));
        assert!(!quota.observed());
        // -1 keeps "never observed" distinct from "nothing left".
        assert_eq!(quota.remaining_tokens, -1);
    }

    #[test]
    fn garbage_header_values_do_not_panic() {
        let quota = RateLimit::from_headers(&headers_from(&[(
            "x-ratelimit-remaining-tokens",
            "not-a-number",
        )]));
        assert_eq!(quota.remaining_tokens, -1);
    }

    #[test]
    fn sticky_relay_rewrite_keeps_the_credentials() {
        // The user:pass selects the sticky egress slot; only the address moves.
        assert_eq!(
            rewrite_proxy_host("http://mail-bob:sticky@172.20.0.1:18100", "127.0.0.1:18100"),
            "http://mail-bob:sticky@127.0.0.1:18100"
        );
    }

    #[test]
    fn rewrite_handles_missing_scheme_and_credentials() {
        assert_eq!(
            rewrite_proxy_host("172.20.0.1:18100", "127.0.0.1:18100"),
            "http://127.0.0.1:18100"
        );
        assert_eq!(
            rewrite_proxy_host("http://172.20.0.1:18100", "127.0.0.1:18100"),
            "http://127.0.0.1:18100"
        );
    }

    #[test]
    fn relay_override_applies_only_to_account_proxies() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5)
            .with_default_proxy("http://default:1")
            .with_sticky_relay("127.0.0.1:18100");
        assert_eq!(
            upstream.effective_proxy("http://mail-a:sticky@172.20.0.1:18100"),
            "http://mail-a:sticky@127.0.0.1:18100"
        );
        // A blank account proxy still falls through to the default untouched.
        assert_eq!(upstream.effective_proxy(""), "http://default:1");
    }

    #[test]
    fn without_relay_override_account_proxy_is_used_as_is() {
        let upstream = Upstream::new(DEFAULT_BASE_URL, 5);
        assert_eq!(
            upstream.effective_proxy("http://mail-a:sticky@172.20.0.1:18100"),
            "http://mail-a:sticky@172.20.0.1:18100"
        );
    }

    #[test]
    fn json_chat_bodies_parse_as_objects() {
        let body = parse_chat_body(r#"{"choices":[{"message":{"content":"ping"}}]}"#).unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ping");
    }

    #[test]
    fn sse_chat_bodies_are_assembled_instead_of_rejected() {
        // This is what NewAPI's stream test used to surface as
        // "bad upstream json: expected value at line 1 column 1".
        let sse = concat!(
            "data: {\"id\":\"abc\",\"model\":\"grok-4.6\",\"choices\":[{\"delta\":{\"content\":\"pi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ng\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let body = parse_chat_body(sse).unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ping");
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert_eq!(body["id"], "abc");
    }

    #[test]
    fn empty_bodies_are_a_parse_error_not_a_silent_object() {
        assert!(parse_chat_body("").is_err());
        assert!(parse_chat_body("   \n").is_err());
    }

    #[test]
    fn disable_streaming_strips_the_flag_newapi_sends() {
        let mut payload = serde_json::json!({
            "model": "grok-4.6",
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        assert!(wants_stream(&payload));
        disable_streaming(&mut payload);
        assert_eq!(payload["stream"], false);
        assert!(payload.get("stream_options").is_none());
        assert!(!wants_stream(&payload));
    }

    #[test]
    fn completion_to_sse_is_something_a_stream_client_can_read() {
        let sse = completion_to_sse(&serde_json::json!({
            "id": "x",
            "model": "grok-4.6",
            "created": 1,
            "choices": [{"message": {"content": "hi"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        }));
        assert!(sse.contains("data: "));
        assert!(sse.contains("chat.completion.chunk"));
        assert!(sse.contains("\"role\":\"assistant\""));
        assert!(sse.contains("\"content\":\"hi\""));
        assert!(!sse.contains("\"role\":\"assistant\",\"content\""));
        assert!(sse.contains("data: [DONE]"));
    }

    #[test]
    fn completion_to_sse_emits_tool_calls_separately() {
        let sse = completion_to_sse(&serde_json::json!({
            "id": "x",
            "model": "grok-4.6",
            "created": 1,
            "choices": [{
                "message": {
                    "content": "",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "bash", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
        }));
        assert!(sse.contains("tool_calls"));
        assert!(sse.contains("\"index\":0"));
        assert!(sse.contains("\"finish_reason\":\"tool_calls\""));
    }

    #[test]
    fn model_version_key_orders_semantically() {
        assert!(model_version_key("grok-4.10").unwrap() > model_version_key("grok-4.6").unwrap());
    }

    #[test]
    fn assemble_sse_merges_tool_call_deltas() {
        let sse = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"x\\\":1}\"}}]}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n\
                   data: [DONE]\n\n";
        let body = assemble_sse(sse).expect("assemble");
        assert_eq!(body["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "bash");
    }
}
