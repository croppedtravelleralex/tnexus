//! Web 养号：周期性对本机 `/v1/chat/completions` 发轻量对话（对齐 GPT nurture 语义）。

use std::time::Duration;

use serde_json::json;

pub fn default_nurture_prompt() -> String {
    std::env::var("GROK_NURTURE_PROMPT")
        .unwrap_or_else(|_| "Say hello in one short sentence.".into())
}

pub fn nurture_enabled() -> bool {
    std::env::var("GROK_NURTURE_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn nurture_interval() -> Duration {
    let secs = std::env::var("GROK_NURTURE_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(480);
    Duration::from_secs(secs.max(60))
}

/// 单轮养号：POST OpenAI 兼容 chat（走号池调度）。
pub async fn run_once(
    client: &reqwest::Client,
    base_url: &str,
    auth_key: Option<&str>,
) -> anyhow::Result<()> {
    let prompt = default_nurture_prompt();
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
    let mut req = client.post(&url).json(&json!({
        "model": "grok-chat-fast",
        "stream": false,
        "messages": [{"role": "user", "content": prompt}],
    }));
    if let Some(key) = auth_key.filter(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key.trim());
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "nurture HTTP {status}: {}",
            body.chars().take(400).collect::<String>()
        );
    }
    tracing::info!(bytes = body.len(), "grok_web_nurture_ok");
    Ok(())
}
