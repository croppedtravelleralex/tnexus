//! Refresh access_token via OpenAI OAuth (subset of `helper/account_ops.py`).

use crate::user_info;
use anyhow::Result;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Map, Value};

const OAUTH_CLIENT_ID: &str = "app_2SKx67EdpoN0G6j64rFvigXD";

pub async fn refresh_access_token(
    http: &reqwest::Client,
    account: &Map<String, Value>,
    force: bool,
) -> Map<String, Value> {
    let mut acc = account.clone();
    let refresh_token = acc
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if refresh_token.is_empty() {
        return acc;
    }

    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"),
    );

    let proxy = acc
        .get("proxy")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let builder = http
        .post("https://auth.openai.com/oauth/token")
        .headers(headers)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", OAUTH_CLIENT_ID),
        ]);
    if !proxy.is_empty() {
        // reqwest doesn't support per-request proxy easily without Client builder — skip for MVP
        let _ = proxy;
    }

    let response = builder.send().await;
    match response {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let data: Value = serde_json::from_str(&text).unwrap_or(json!({}));
            if status.is_success() {
                if let Some(token) = data.get("access_token").and_then(|v| v.as_str()) {
                    if !token.trim().is_empty() {
                        acc.insert("access_token".into(), json!(token.trim()));
                    }
                }
                if let Some(rt) = data.get("refresh_token").and_then(|v| v.as_str()) {
                    if !rt.trim().is_empty() {
                        acc.insert("refresh_token".into(), json!(rt.trim()));
                    }
                }
                if let Some(id) = data.get("id_token").and_then(|v| v.as_str()) {
                    if !id.trim().is_empty() {
                        acc.insert("id_token".into(), json!(id.trim()));
                    }
                }
                acc.insert(
                    "last_token_refresh_at".into(),
                    json!(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                );
                acc.insert("last_token_refresh_error".into(), Value::Null);
            } else if force {
                let err = data
                    .get("error_description")
                    .or_else(|| data.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(text.as_str());
                acc.insert(
                    "last_token_refresh_error".into(),
                    json!(err.chars().take(300).collect::<String>()),
                );
            }
        }
        Err(e) if force => {
            acc.insert("last_token_refresh_error".into(), json!(e.to_string()));
        }
        Err(_) => {}
    }
    acc
}

pub async fn refresh_account(
    http: &reqwest::Client,
    account: &Map<String, Value>,
) -> Result<Value> {
    let acc = refresh_access_token(http, account, false).await;
    let token = acc
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if token.is_empty() {
        return Err(anyhow::anyhow!("access_token is required"));
    }
    let merged = user_info::merge_user_info(http, &acc).await;
    Ok(Value::Object(merged))
}
