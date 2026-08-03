//! Pull account quota/status from ChatGPT backend-api (best-effort).

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde_json::{json, Map, Value};

const USER_AGENT_STR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36";

pub async fn merge_user_info(http: &reqwest::Client, account: &Map<String, Value>) -> Map<String, Value> {
    let mut acc = account.clone();
    let token = acc
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return acc;
    }
    if let Ok(info) = fetch_me(http, token).await {
        for key in [
            "email",
            "status",
            "quota",
            "type",
            "restore_at",
            "image_quota_unknown",
            "limits_progress",
        ] {
            if let Some(v) = info.get(key) {
                if !v.is_null() {
                    acc.insert(key.into(), v.clone());
                }
            }
        }
        acc.insert("last_quota_refresh_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    }
    acc.insert(
        "source_type".into(),
        json!(acc.get("source_type").and_then(|v| v.as_str()).unwrap_or("tnexus_refresh")),
    );
    acc
}

async fn fetch_me(http: &reqwest::Client, token: &str) -> Result<Value> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).context("auth header")?,
    );
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
    let resp = http
        .get("https://chatgpt.com/backend-api/me")
        .headers(headers)
        .send()
        .await
        .context("GET /backend-api/me")?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow::anyhow!("me HTTP {status}: {text}"));
    }
    let data: Value = serde_json::from_str(&text).unwrap_or(json!({}));
    normalize_me(data)
}

fn normalize_me(data: Value) -> Result<Value> {
    let obj = data.as_object().cloned().unwrap_or_default();
    let email = obj
        .get("email")
        .or_else(|| obj.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut out = Map::new();
    if !email.is_empty() {
        out.insert("email".into(), json!(email));
    }
    if let Some(plan) = obj.get("plan_type").or_else(|| obj.get("subscription_plan")) {
        out.insert("type".into(), plan.clone());
    }
    if let Some(features) = obj.get("features").and_then(|v| v.as_array()) {
        for f in features {
            if f.get("slug").and_then(|s| s.as_str()) == Some("image_gen") {
                if let Some(q) = f.get("remaining") {
                    out.insert("quota".into(), q.clone());
                }
            }
        }
    }
    if out.get("quota").is_none() {
        out.insert("quota".into(), json!(0));
    }
    out.insert("status".into(), json!("正常"));
    Ok(Value::Object(out))
}
