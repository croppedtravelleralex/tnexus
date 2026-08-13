//! Native TNexus account ops client (helper account_ops_face on :9011).

use crate::state::AppState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct ProgressStore {
    inner: Arc<RwLock<HashMap<String, Value>>>,
}

impl ProgressStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn init(&self, progress_id: &str, total: usize) {
        let mut guard = self.inner.write().await;
        guard.insert(
            progress_id.to_string(),
            json!({
                "total": total,
                "processed": 0,
                "done": false,
                "error": null,
                "status_counts": { "正常": 0, "限流": 0, "异常": 0, "禁用": 0 },
                "total_quota": 0,
            }),
        );
    }

    pub async fn bump(&self, progress_id: &str, status: &str, quota: i64) {
        let mut guard = self.inner.write().await;
        let Some(row) = guard.get_mut(progress_id) else {
            return;
        };
        row["processed"] = json!(row["processed"].as_u64().unwrap_or(0) + 1);
        if let Some(counts) = row.get_mut("status_counts").and_then(|v| v.as_object_mut()) {
            let key = status.to_string();
            let cur = counts.get(&key).and_then(|v| v.as_u64()).unwrap_or(0);
            counts.insert(key, json!(cur + 1));
        }
        row["total_quota"] = json!(row["total_quota"].as_i64().unwrap_or(0) + quota);
    }

    pub async fn finish(&self, progress_id: &str, result: Value, error: Option<&str>) {
        let mut guard = self.inner.write().await;
        let Some(row) = guard.get_mut(progress_id) else {
            return;
        };
        row["done"] = json!(true);
        row["result"] = result;
        if let Some(err) = error {
            row["error"] = json!(err);
        }
    }

    pub async fn get(&self, progress_id: &str) -> Option<Value> {
        let guard = self.inner.read().await;
        guard.get(progress_id).cloned()
    }
}

fn ops_token(state: &AppState) -> Result<String, String> {
    state
        .config
        .account_ops_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "ACCOUNT_OPS_TOKEN 未配置".into())
}

/// 发起请求并原样交回 (是否 2xx, 响应体文本)。
/// 失败时也保留响应体，供 refresh-one 取回带错误标记的账号。
async fn post_raw(state: &AppState, path: &str, body: Value) -> Result<(bool, String), String> {
    let base = state.config.account_ops_base.trim_end_matches('/');
    let token = ops_token(state)?;
    let resp = state
        .http
        .post(format!("{base}{path}"))
        .header("X-Account-Ops-Token", token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("account_ops 请求失败: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("account_ops 响应读取失败: {e}"))?;
    Ok((status.is_success(), text))
}

fn ops_error_message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.get("detail")
                .and_then(|d| d.get("error"))
                .or_else(|| v.get("error"))
                .and_then(|e| e.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.to_string())
}

async fn post_json(state: &AppState, path: &str, body: Value) -> Result<Value, String> {
    let (ok, text) = post_raw(state, path, body).await?;
    if !ok {
        return Err(ops_error_message(&text));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("account_ops JSON 解析失败: {e}"))
}

async fn get_json(state: &AppState, path: &str) -> Result<Value, String> {
    let base = state.config.account_ops_base.trim_end_matches('/');
    let token = ops_token(state)?;
    let resp = state
        .http
        .get(format!("{base}{path}"))
        .header("X-Account-Ops-Token", token)
        .send()
        .await
        .map_err(|e| format!("account_ops 请求失败: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("account_ops 响应读取失败: {e}"))?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.get("detail")
                    .and_then(|d| d.get("error"))
                    .or_else(|| v.get("error"))
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .unwrap_or(text);
        return Err(message);
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("account_ops JSON 解析失败: {e}"))
}

pub fn ops_available(state: &AppState) -> bool {
    state.config.account_ops_token.is_some()
}

pub async fn nurture_status(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/nurture/status").await
}

pub async fn nurture_enable(state: &AppState, enabled: bool) -> Result<Value, String> {
    post_json(state, "/v1/nurture/enable", json!({ "enabled": enabled })).await
}

pub async fn nurture_enqueue(state: &AppState, body: Value) -> Result<Value, String> {
    post_json(state, "/v1/nurture/enqueue", body).await
}

pub async fn nurture_process_one(state: &AppState, body: Value) -> Result<Value, String> {
    post_json(state, "/v1/nurture/process-one", body).await
}

pub async fn outlook_auto_recovery_status(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/outlook/auto-recovery/status").await
}

pub async fn outlook_auto_recovery_settings(
    state: &AppState,
    body: Value,
) -> Result<Value, String> {
    post_json(state, "/v1/outlook/auto-recovery/settings", body).await
}

pub async fn outlook_recover_one(state: &AppState, body: Value) -> Result<Value, String> {
    post_json(state, "/v1/outlook/recover-one", body).await
}

pub async fn outlook_recover_progress(
    state: &AppState,
    progress_id: &str,
) -> Result<Value, String> {
    get_json(
        state,
        &format!("/v1/outlook/recover/progress/{progress_id}"),
    )
    .await
}

pub async fn quota_prime_enqueue(state: &AppState, body: Value) -> Result<Value, String> {
    post_json(state, "/v1/quota-window/prime", body).await
}

pub async fn quota_prime_status(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/quota-window/prime/status").await
}

pub async fn proxy_runtime_get(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/proxy/runtime").await
}

pub async fn proxy_runtime_save(state: &AppState, body: Value) -> Result<Value, String> {
    post_json(state, "/v1/proxy/runtime", body).await
}

pub async fn proxy_test(state: &AppState, url: &str) -> Result<Value, String> {
    post_json(state, "/v1/proxy/test", json!({ "url": url })).await
}

pub async fn webshare_cf_scan_status(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/webshare-cf-scan/status").await
}

pub async fn webshare_cf_scan_inventory(state: &AppState) -> Result<Value, String> {
    get_json(state, "/v1/webshare-cf-scan/inventory").await
}

pub async fn webshare_cf_scan_run_once(state: &AppState) -> Result<Value, String> {
    post_json(state, "/v1/webshare-cf-scan/run-once", json!({})).await
}

pub async fn oauth_start(state: &AppState, email_hint: &str) -> Result<Value, String> {
    post_json(
        state,
        "/v1/oauth/start",
        json!({ "email_hint": email_hint }),
    )
    .await
}

pub async fn oauth_finish(
    state: &AppState,
    session_id: &str,
    callback: &str,
) -> Result<Value, String> {
    post_json(
        state,
        "/v1/oauth/finish",
        json!({ "session_id": session_id, "callback": callback }),
    )
    .await
}

pub async fn refresh_one(state: &AppState, account: Value) -> Result<Value, String> {
    let (ok, text) = post_raw(
        state,
        "/v1/accounts/refresh-one",
        json!({ "account": account }),
    )
    .await?;
    let data: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !ok {
        // account-ops 失败时会回带打了 last_token_refresh_error 的账号。
        // 写回库，让「哪些号刷不动」变成可查状态，而不是只在批量报告里一闪而过。
        if let Some(failed) = data.get("account").cloned() {
            if let Err(e) = state.accounts.merge_remote_items(&[failed]).await {
                tracing::warn!(error = %e, "refresh-one 失败标记写回失败");
            }
        }
        return Err(ops_error_message(&text));
    }
    data.get("account")
        .cloned()
        .ok_or_else(|| "refresh-one 响应缺少 account".into())
}

pub async fn relogin_one(state: &AppState, account: Value) -> Result<Value, String> {
    let data = post_json(
        state,
        "/v1/accounts/relogin-one",
        json!({ "account": account }),
    )
    .await?;
    data.get("account")
        .cloned()
        .ok_or_else(|| "relogin-one 响应缺少 account".into())
}

pub async fn spawn_refresh(
    state: Arc<AppState>,
    tokens: Vec<String>,
    progress: ProgressStore,
    progress_id: String,
) {
    progress.init(&progress_id, tokens.len()).await;
    let mut refreshed = 0usize;
    let mut errors: Vec<Value> = Vec::new();
    let mut updated_items: Vec<Value> = Vec::new();

    for token in tokens {
        let account = state.accounts.export_account_for_token(&token).await;
        let Some(account) = account else {
            errors.push(json!({ "token": token.chars().take(8).collect::<String>(), "error": "account not found" }));
            progress.bump(&progress_id, "异常", 0).await;
            continue;
        };
        match refresh_one(&state, account).await {
            Ok(updated) => {
                let status = updated
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("正常")
                    .to_string();
                let quota = updated.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
                if let Err(e) = state.accounts.merge_remote_items(&[updated.clone()]).await {
                    errors.push(json!({ "error": e.to_string() }));
                } else {
                    refreshed += 1;
                    updated_items.push(updated);
                }
                progress.bump(&progress_id, &status, quota).await;
            }
            Err(err) => {
                errors.push(json!({ "error": err }));
                progress.bump(&progress_id, "异常", 0).await;
            }
        }
    }

    let list = state.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let result = json!({
        "refreshed": refreshed,
        "errors": errors,
        "relogined": 0,
        "items": updated_items,
        "stats": stats,
    });
    progress.finish(&progress_id, result, None).await;
}

pub async fn spawn_relogin(
    state: Arc<AppState>,
    tokens: Vec<String>,
    progress: ProgressStore,
    progress_id: String,
) {
    progress.init(&progress_id, tokens.len()).await;
    let mut relogined = 0usize;
    let mut errors: Vec<Value> = Vec::new();
    let mut updated_items: Vec<Value> = Vec::new();

    for token in tokens {
        let account = state.accounts.export_account_for_token(&token).await;
        let Some(account) = account else {
            errors.push(json!({ "error": "account not found" }));
            progress.bump(&progress_id, "异常", 0).await;
            continue;
        };
        match relogin_one(&state, account).await {
            Ok(updated) => {
                let status = updated
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("正常")
                    .to_string();
                let quota = updated.get("quota").and_then(|v| v.as_i64()).unwrap_or(0);
                if state
                    .accounts
                    .merge_remote_items(&[updated.clone()])
                    .await
                    .is_ok()
                {
                    relogined += 1;
                    updated_items.push(updated);
                }
                progress.bump(&progress_id, &status, quota).await;
            }
            Err(err) => {
                errors.push(json!({ "error": err }));
                progress.bump(&progress_id, "异常", 0).await;
            }
        }
    }

    let list = state.accounts.list(0, usize::MAX).await;
    let stats = list.get("stats").cloned().unwrap_or(json!({}));
    let result = json!({
        "relogined": relogined,
        "errors": errors,
        "items": updated_items,
        "stats": stats,
    });
    progress.finish(&progress_id, result, None).await;
}

pub fn new_progress_id() -> String {
    Uuid::new_v4().to_string()
}
