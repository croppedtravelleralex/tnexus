//! Text nurture — lightweight POST to ChatGPT backend-api (no gateway pin required).

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Value};
use uuid::Uuid;

const USER_AGENT_STR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";

pub fn default_nurture_prompt() -> String {
    std::env::var("NURTURE_DEFAULT_PROMPT")
        .unwrap_or_else(|_| "Say hello in one short sentence.".into())
}

fn text_chat_body(prompt: &str) -> Value {
    let msg_id = Uuid::new_v4().to_string();
    json!({
        "action": "next",
        "messages": [{
            "id": msg_id,
            "author": {"role": "user"},
            "content": {"content_type": "text", "parts": [prompt]},
        }],
        "model": "auto",
        "parent_message_id": "client-created-root",
        "conversation_mode": {"kind": "primary_assistant"},
        "history_and_training_disabled": true,
        "timezone": "Asia/Shanghai",
        "timezone_offset_min": -480,
    })
}

pub async fn run_text_nurture(http: &reqwest::Client, access_token: &str, prompt: &str) -> Result<Value> {
    let token = access_token.trim();
    if token.is_empty() {
        anyhow::bail!("access_token required");
    }
    let prompt = if prompt.trim().is_empty() {
        default_nurture_prompt()
    } else {
        prompt.trim().to_string()
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).context("auth header")?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "oai-language",
        HeaderValue::from_static("en-US"),
    );

    let resp = http
        .post("https://chatgpt.com/backend-api/conversation")
        .headers(headers)
        .json(&text_chat_body(&prompt))
        .send()
        .await
        .context("POST /backend-api/conversation")?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("nurture HTTP {status}: {}", body.chars().take(400).collect::<String>());
    }

    Ok(json!({
        "ok": true,
        "prompt": prompt,
        "bytes": body.len(),
        "at": Utc::now().to_rfc3339(),
        "source": "tnexus-account-ops",
    }))
}
