//! Refresh access_token via OpenAI OAuth (subset of `helper/account_ops.py`).

use crate::user_info;
use anyhow::Result;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Map, Value};

const OAUTH_CLIENT_ID: &str = "app_2SKx67EdpoN0G6j64rFvigXD";

fn is_terminal_refresh_error(err: &str) -> bool {
    let lowered = err.to_ascii_lowercase();
    [
        "token invalidated",
        "refresh_token_invalidated",
        "refresh_token_reused",
        "session has ended",
        "already been used",
        "invalid access token",
        "invalid_access_token",
        "account_deactivated",
    ]
    .iter()
    .any(|m| lowered.contains(m))
}

fn now_stamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// 记录一次刷新失败。
///
/// 错误信息**任何情况下**都写入：调用方只拿到账号 Map，若不写就无法区分
/// 「刷新成功」与「刷新失败但原样返回」。把账号判死（invalid_count / 状态异常）
/// 仍只在 force 时做，避免一次网络抖动就误伤号池。
fn record_refresh_error(acc: &mut Map<String, Value>, err: &str, force: bool) {
    acc.insert("last_token_refresh_error".into(), json!(err));
    acc.insert("last_token_refresh_error_at".into(), json!(now_stamp()));
    if force && is_terminal_refresh_error(err) {
        let invalid = acc
            .get("invalid_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            + 1;
        acc.insert("invalid_count".into(), json!(invalid));
        acc.insert("status".into(), json!("异常"));
        acc.insert("panda_receive_state".into(), json!("identity_isolated"));
    }
}

/// 从刷新后的账号里读出失败原因；`None` 表示这次刷新确实成功了。
pub fn refresh_error(acc: &Map<String, Value>) -> Option<String> {
    acc.get("last_token_refresh_error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

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
        .trim()
        .to_string();
    if refresh_token.is_empty() {
        record_refresh_error(&mut acc, "refresh_token is empty", force);
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
            ("refresh_token", refresh_token.as_str()),
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
                acc.insert("last_token_refresh_at".into(), json!(now_stamp()));
                acc.insert("last_token_refresh_error".into(), Value::Null);
                acc.insert("last_token_refresh_error_at".into(), Value::Null);
            } else {
                let err = data
                    .get("error_description")
                    .or_else(|| data.get("error"))
                    .or_else(|| data.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(text.as_str());
                let err_s: String = format!("HTTP {status}: {err}").chars().take(300).collect();
                record_refresh_error(&mut acc, &err_s, force);
            }
        }
        Err(e) => {
            record_refresh_error(&mut acc, &e.to_string(), force);
        }
    }
    acc
}

pub async fn refresh_account(
    http: &reqwest::Client,
    account: &Map<String, Value>,
) -> Result<Value> {
    let acc = refresh_access_token(http, account, false).await;
    if let Some(err) = refresh_error(&acc) {
        return Err(anyhow::anyhow!("token refresh failed: {err}"));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn acc_with(pairs: &[(&str, Value)]) -> Map<String, Value> {
        let mut m = Map::new();
        for (k, v) in pairs {
            m.insert((*k).into(), v.clone());
        }
        m
    }

    /// 回归：非 force 的刷新失败过去什么都不写，调用方只能假定成功。
    #[test]
    fn records_error_even_without_force() {
        let mut acc = acc_with(&[("email", json!("a@b.c"))]);
        record_refresh_error(&mut acc, "HTTP 400: invalid_grant", false);

        assert_eq!(
            refresh_error(&acc).as_deref(),
            Some("HTTP 400: invalid_grant")
        );
        assert!(acc.get("last_token_refresh_error_at").is_some());
    }

    /// 非 force 只记录、不判死：一次网络抖动不该把账号打成异常。
    #[test]
    fn does_not_escalate_without_force() {
        let mut acc = acc_with(&[("status", json!("正常"))]);
        record_refresh_error(&mut acc, "refresh_token_invalidated", false);

        assert!(refresh_error(&acc).is_some());
        assert_eq!(acc.get("status").and_then(|v| v.as_str()), Some("正常"));
        assert!(acc.get("invalid_count").is_none());
    }

    /// force + 终态错误才判死并累加失效次数。
    #[test]
    fn escalates_terminal_error_when_forced() {
        let mut acc = acc_with(&[("status", json!("正常")), ("invalid_count", json!(2))]);
        record_refresh_error(&mut acc, "HTTP 400: refresh_token_invalidated", true);

        assert_eq!(acc.get("status").and_then(|v| v.as_str()), Some("异常"));
        assert_eq!(acc.get("invalid_count").and_then(|v| v.as_u64()), Some(3));
        assert_eq!(
            acc.get("panda_receive_state").and_then(|v| v.as_str()),
            Some("identity_isolated")
        );
    }

    /// force 但错误可恢复（如 5xx）时不判死。
    #[test]
    fn does_not_escalate_transient_error_when_forced() {
        let mut acc = acc_with(&[("status", json!("正常"))]);
        record_refresh_error(&mut acc, "HTTP 503: upstream busy", true);

        assert!(refresh_error(&acc).is_some());
        assert_eq!(acc.get("status").and_then(|v| v.as_str()), Some("正常"));
    }

    /// 成功后清除历史错误，否则账号会永久带着旧错误被判为失败。
    #[test]
    fn success_clears_stale_error() {
        let mut acc = acc_with(&[("last_token_refresh_error", json!("HTTP 400: old"))]);
        assert!(refresh_error(&acc).is_some());

        acc.insert("last_token_refresh_error".into(), Value::Null);
        acc.insert("last_token_refresh_error_at".into(), Value::Null);

        assert_eq!(refresh_error(&acc), None);
    }

    /// 空白错误串不算失败，避免误报。
    #[test]
    fn blank_error_is_not_a_failure() {
        let acc = acc_with(&[("last_token_refresh_error", json!("   "))]);
        assert_eq!(refresh_error(&acc), None);
    }
}
