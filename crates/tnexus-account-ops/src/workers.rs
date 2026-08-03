//! Background workers: nurture queue, outlook recovery, quota-window prime.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tracing::{info, warn};

use crate::nurture;
use crate::ops::OpsServices;
use crate::refresh;
use crate::user_info;

const NURTURE_POLL_SECS: u64 = 8;
const OUTLOOK_POLL_SECS: u64 = 12;
const QUOTA_PRIME_POLL_SECS: u64 = 15;

pub fn spawn_all(ops: Arc<OpsServices>, http: reqwest::Client) {
    let ops_n = ops.clone();
    let http_n = http.clone();
    tokio::spawn(async move {
        nurture_loop(ops_n, http_n).await;
    });

    let ops_o = ops.clone();
    let http_o = http.clone();
    tokio::spawn(async move {
        outlook_loop(ops_o, http_o).await;
    });

    let ops_q = ops.clone();
    let http_q = http.clone();
    tokio::spawn(async move {
        quota_prime_loop(ops_q, http_q).await;
    });
}

async fn nurture_loop(ops: Arc<OpsServices>, http: reqwest::Client) {
    loop {
        tokio::time::sleep(Duration::from_secs(NURTURE_POLL_SECS)).await;
        if !ops.nurture_running() {
            continue;
        }
        let job = ops.pop_nurture_job();
        if job.is_none() {
            continue;
        }
        let job = job.unwrap();
        let result = nurture::run_text_nurture(&http, &job.access_token, &job.prompt).await;
        match result {
            Ok(v) => {
                ops.record_nurture_success();
                let bytes = v.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                info!(email = %job.email, bytes, "nurture ok");
            }
            Err(e) => {
                ops.record_nurture_error(e.to_string());
                warn!(email = %job.email, error = %e, "nurture failed");
            }
        }
    }
}

async fn outlook_loop(ops: Arc<OpsServices>, http: reqwest::Client) {
    loop {
        tokio::time::sleep(Duration::from_secs(OUTLOOK_POLL_SECS)).await;
        let settings = ops.outlook_settings_snapshot();
        let enabled = settings
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !enabled {
            continue;
        }
        let pending = ops.outlook_pending_ids();
        for id in pending {
            let (token, account) = match ops.outlook_job(&id) {
                Some(pair) => pair,
                None => continue,
            };
            if token.is_empty() {
                ops.outlook_mark_failed(&id, "missing access_token");
                continue;
            }
            ops.outlook_mark_running(&id);
            let mut acc = account;
            if acc.is_empty() {
                acc.insert("access_token".into(), json!(token));
            }
            let refreshed = refresh::refresh_access_token(&http, &acc, true).await;
            let merged = user_info::merge_user_info(&http, &refreshed).await;
            let ok = merged
                .get("access_token")
                .and_then(|v| v.as_str())
                .map(|t| !t.trim().is_empty())
                .unwrap_or(false);
            if ok {
                ops.outlook_mark_done(&id, merged);
            } else {
                let err = merged
                    .get("last_token_refresh_error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("refresh failed");
                ops.outlook_mark_failed(&id, err);
            }
        }
    }
}

async fn quota_prime_loop(ops: Arc<OpsServices>, http: reqwest::Client) {
    loop {
        tokio::time::sleep(Duration::from_secs(QUOTA_PRIME_POLL_SECS)).await;
        let token = ops.pop_quota_prime_job();
        if token.is_none() {
            continue;
        }
        let job = token.unwrap();
        let gateway = std::env::var("GATEWAY_BASE")
            .or_else(|_| std::env::var("GPTIMAGE_BASE"))
            .unwrap_or_else(|_| "http://127.0.0.1:8014".into());
        let auth = std::env::var("GATEWAY_AUTH_KEY")
            .or_else(|_| std::env::var("UPSTREAM_API_KEY"))
            .unwrap_or_default();
        let prompt = std::env::var("QUOTA_PRIME_PROMPT")
            .unwrap_or_else(|_| "a tiny red dot on white background".into());

        let url = format!("{}/v1/images/generations", gateway.trim_end_matches('/'));
        let mut req = http
            .post(&url)
            .header("Authorization", format!("Bearer {}", auth.trim()))
            .json(&json!({
                "model": "gpt-image-2",
                "prompt": prompt,
                "n": 1,
                "size": "1024x1024",
                "response_format": "b64_json",
            }));
        if !job.email.is_empty() {
            req = req.header("X-Preferred-Account-Email", job.email.trim());
        }
        let resp = req.send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                ops.quota_prime_done_one(true, None);
                info!("quota prime ok");
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                ops.quota_prime_done_one(false, Some(text.chars().take(200).collect()));
                warn!(status = %status, "quota prime failed");
            }
            Err(e) => {
                ops.quota_prime_done_one(false, Some(e.to_string()));
                warn!(error = %e, "quota prime request failed");
            }
        }
    }
}
