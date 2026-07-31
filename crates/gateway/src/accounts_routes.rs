//! Admin account pool routes — gptimage-compatible subset for the web UI.

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use helper_client::PinAccount;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    200
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(default = "default_days")]
    pub days: usize,
}

fn default_days() -> usize {
    14
}

#[derive(Debug, Deserialize)]
pub struct SchedulingBulkBody {
    pub emails: Vec<String>,
    pub enabled: bool,
}

fn pin_to_account(pin: &PinAccount) -> Value {
    let status = if pin.access_token.is_empty() {
        "异常"
    } else {
        "正常"
    };
    json!({
        "access_token": pin.access_token,
        "email": pin.email,
        "type": "openai",
        "status": status,
        "quota": 0,
        "image_quota_unknown": true,
        "image_schedulable": !pin.access_token.is_empty(),
        "proxy": pin.proxy,
        "success": 0,
        "fail": 0,
        "created_at": null,
    })
}

fn compute_stats(items: &[Value]) -> Value {
    let total = items.len();
    let mut normal = 0usize;
    let mut abnormal = 0usize;
    let mut schedulable = 0usize;
    for item in items {
        let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "正常" {
            normal += 1;
        } else {
            abnormal += 1;
        }
        if item.get("image_schedulable").and_then(|v| v.as_bool()).unwrap_or(false) {
            schedulable += 1;
        }
    }
    json!({
        "total": total,
        "active": normal,
        "limited": 0,
        "abnormal": abnormal,
        "disabled": 0,
        "total_quota": 0,
        "schedulable": schedulable,
        "scheduling_enabled": schedulable,
        "image_schedulable": schedulable,
        "available_image_quota": 0,
        "verified_quota_count": 0,
        "stale_quota_count": total,
    })
}

pub async fn list_accounts(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> Json<Value> {
    let guard = st.accounts.lock().await;
    let all_items: Vec<Value> = guard.values().map(pin_to_account).collect();
    let stats = compute_stats(&all_items);
    let total = all_items.len();
    let page = if q.limit == 0 {
        vec![]
    } else {
        all_items.into_iter().skip(q.offset).take(q.limit).collect()
    };
    drop(guard);
    Json(json!({
        "items": page,
        "total": total,
        "offset": q.offset,
        "limit": q.limit,
        "stats": stats,
    }))
}

pub async fn activity_daily(Query(q): Query<ActivityQuery>) -> Json<Value> {
    let days = q.days.clamp(1, 90);
    let today = chrono::Utc::now().date_naive();
    let mut items = Vec::with_capacity(days);
    for i in (0..days).rev() {
        let date = today - chrono::Days::new(i as u64);
        items.push(json!({
            "date": date.format("%Y-%m-%d").to_string(),
            "registered": 0,
            "uploaded": 0,
            "received": 0,
            "deleted": 0,
            "images": 0,
            "images_api": 0,
            "images_chat": 0,
            "dialogues": 0,
            "dialogues_real": 0,
            "dialogues_nurture": 0,
        }));
    }
    Json(json!({
        "days": days,
        "sync_label": "本地",
        "items": items,
    }))
}

pub async fn scheduling_bulk(
    State(st): State<Arc<AppState>>,
    Json(body): Json<SchedulingBulkBody>,
) -> Json<Value> {
    let updated = st
        .scheduling_gate
        .set_bulk(&body.emails, body.enabled);
    Json(json!({
        "ok": true,
        "updated": updated,
        "enabled": body.enabled,
        "source": "gateway-local",
    }))
}

pub async fn reload_from_storage(State(st): State<Arc<AppState>>) -> Json<Value> {
    let count = st.accounts.lock().await.len();
    Json(json!({
        "ok": true,
        "total": count,
    }))
}
