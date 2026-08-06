//! Quota-window prime via gateway upstream image (fallback when account-ops unavailable).

use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct QuotaPrimeJob {
    inner: Arc<RwLock<Option<Value>>>,
}

impl QuotaPrimeJob {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn status(&self) -> Value {
        self.inner.read().await.clone().unwrap_or_else(|| {
            json!({
                "running": false,
                "state": "idle",
                "queue": [],
                "source": "tnexus-local",
            })
        })
    }

    pub async fn enqueue_tokens(
        &self,
        state: Arc<AppState>,
        tokens: Vec<String>,
        accounts: Vec<Value>,
    ) -> Result<Value, String> {
        if tokens.is_empty() {
            return Err("access_tokens is required".into());
        }
        {
            let guard = self.inner.read().await;
            if guard
                .as_ref()
                .and_then(|v| v.get("running"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return Err("quota prime job already running".into());
            }
        }
        let job = json!({
            "running": true,
            "state": "running",
            "total": tokens.len(),
            "processed": 0,
            "succeeded": 0,
            "failed": 0,
            "source": "tnexus-gateway-fallback",
        });
        *self.inner.write().await = Some(job.clone());

        let count = tokens.len();
        let store = self.clone();
        tokio::spawn(async move {
            run_prime_batch(state, store, tokens, accounts).await;
        });
        Ok(json!({ "queued": count, "source": "tnexus-gateway-fallback" }))
    }
}

async fn run_prime_batch(
    state: Arc<AppState>,
    store: QuotaPrimeJob,
    tokens: Vec<String>,
    accounts: Vec<Value>,
) {
    let gateway = state.config.gateway_base.trim_end_matches('/');
    let prompt = std::env::var("QUOTA_PRIME_PROMPT")
        .unwrap_or_else(|_| "a tiny red dot on white background".into());
    let mut processed = 0usize;
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    let total = tokens.len();
    for token in tokens {
        let email = accounts
            .iter()
            .find(|row| {
                row.get("access_token")
                    .and_then(|v| v.as_str())
                    .map(|t| t == token)
                    .unwrap_or(false)
            })
            .and_then(|row| row.get("email").and_then(|v| v.as_str()))
            .map(str::to_string)
            .unwrap_or_default();

        let mut patch = json!({ "quota_window_prime_state": "running" });
        let _ = state.accounts.update_by_token(&token, &patch).await;

        let body = json!({
            "model": "gpt-image-2",
            "prompt": prompt,
            "n": 1,
            "size": "1024x1024",
            "response_format": "b64_json",
        });
        let mut req = state
            .http
            .post(format!("{gateway}/v1/images/generations"))
            .json(&body);
        if let Some(key) = state.config.gateway_internal_token.as_deref() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        if !email.is_empty() {
            req = req.header("X-Preferred-Account-Email", email);
        }

        let result = req.send().await;
        processed += 1;
        match result {
            Ok(resp) if resp.status().is_success() => {
                succeeded += 1;
                patch = json!({
                    "quota_window_prime_state": "done",
                    "quota_window_primed_at": chrono::Utc::now().to_rfc3339(),
                    "quota_window_prime_last_error": null,
                });
            }
            Ok(resp) => {
                failed += 1;
                let text = resp.text().await.unwrap_or_default();
                patch = json!({
                    "quota_window_prime_state": "failed",
                    "quota_window_prime_last_error": text.chars().take(300).collect::<String>(),
                });
            }
            Err(err) => {
                failed += 1;
                patch = json!({
                    "quota_window_prime_state": "failed",
                    "quota_window_prime_last_error": err.to_string(),
                });
            }
        }
        let _ = state.accounts.update_by_token(&token, &patch).await;
        let status = json!({
            "running": processed < total,
            "state": if processed < total { "running" } else { "completed" },
            "total": total,
            "processed": processed,
            "succeeded": succeeded,
            "failed": failed,
            "source": "tnexus-gateway-fallback",
        });
        *store.inner.write().await = Some(status);
        tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
    }
    let mut final_status = store.inner.write().await;
    if let Some(row) = final_status.as_mut() {
        row["running"] = json!(false);
        row["state"] = json!("completed");
    }
}
