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
        .or(state.config.gptimage_admin_token.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "ACCOUNT_OPS_TOKEN 未配置".into())
}

async fn post_json(state: &AppState, path: &str, body: Value) -> Result<Value, String> {
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

pub async fn oauth_start(state: &AppState, email_hint: &str) -> Result<Value, String> {
    post_json(
        state,
        "/v1/oauth/start",
        json!({ "email_hint": email_hint }),
    )
    .await
}

pub async fn oauth_finish(state: &AppState, session_id: &str, callback: &str) -> Result<Value, String> {
    post_json(
        state,
        "/v1/oauth/finish",
        json!({ "session_id": session_id, "callback": callback }),
    )
    .await
}

pub async fn refresh_one(state: &AppState, account: Value) -> Result<Value, String> {
    let data = post_json(state, "/v1/accounts/refresh-one", json!({ "account": account })).await?;
    data.get("account")
        .cloned()
        .ok_or_else(|| "refresh-one 响应缺少 account".into())
}

pub async fn relogin_one(state: &AppState, account: Value) -> Result<Value, String> {
    let data = post_json(state, "/v1/accounts/relogin-one", json!({ "account": account })).await?;
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
    progress
        .init(&progress_id, tokens.len())
        .await;
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
                if state.accounts.merge_remote_items(&[updated.clone()]).await.is_ok() {
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
