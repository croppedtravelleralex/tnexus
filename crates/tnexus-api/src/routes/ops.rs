use crate::account_ops;
use crate::middleware::AdminUser;
use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct NurtureEnqueueBody {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    access_tokens: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NurtureEnableBody {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct NurtureProcessOneBody {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Deserialize)]
struct IpNurtureBindingBody {
    binding_key: String,
    #[serde(default)]
    preset_id: String,
    #[serde(default)]
    custom_matrix: Option<Vec<Vec<f64>>>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/summary", get(ops_summary))
        .route("/nurture/status", get(nurture_status))
        .route("/nurture/enable", post(nurture_enable))
        .route("/nurture/enqueue", post(nurture_enqueue))
        .route("/nurture/process-one", post(nurture_process_one))
        .route("/ip-nurture/presets", get(ip_nurture_presets))
        .route("/ip-nurture/bindings", get(ip_nurture_bindings).post(ip_nurture_save_binding))
        .route("/image-pipeline/snapshot", get(image_pipeline_snapshot))
        .route("/risk/metrics", get(risk_metrics))
}

async fn ops_summary(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    let jobs_total: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM jobs")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let jobs_running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM jobs WHERE status NOT IN ('done', 'failed')",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let jobs_done: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM jobs WHERE status = 'done'")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let jobs_failed: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM jobs WHERE status = 'failed'")
            .fetch_one(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let results_total: i64 = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM job_results")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let accounts_total = state.accounts.list(0, usize::MAX).await;
    let accounts_total = accounts_total
        .get("total")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Ok(Json(json!({
        "jobs_total": jobs_total,
        "jobs_running": jobs_running,
        "jobs_done": jobs_done,
        "jobs_failed": jobs_failed,
        "results_total": results_total,
        "accounts_total": accounts_total,
        "source": "tnexus-local",
        "account_ops": account_ops::ops_available(&state),
    })))
}

async fn nurture_status(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if account_ops::ops_available(&state) {
        if let Ok(data) = account_ops::nurture_status(&state).await {
            return Ok(Json(data));
        }
    }
    Ok(Json(json!({
        "enabled": false,
        "running": false,
        "queue": { "depth": 0, "oldest_age_sec": 0 },
        "completed_in_day": 0,
        "max_per_account_per_day": 0,
        "last_error": null,
        "source": "tnexus-local",
        "message": "养号服务未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）",
    })))
}

async fn nurture_enable(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<NurtureEnableBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if account_ops::ops_available(&state) {
        let data = account_ops::nurture_enable(&state, body.enabled)
            .await
            .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e))?;
        return Ok(Json(data));
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "养号服务未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
    ))
}

async fn nurture_enqueue(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<NurtureEnqueueBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !account_ops::ops_available(&state) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "养号服务未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
        ));
    }
    let mut accounts = Vec::new();
    for token in &body.access_tokens {
        if let Some(row) = state.accounts.export_account_for_token(token).await {
            accounts.push(row);
        }
    }
    let data = account_ops::nurture_enqueue(
        &state,
        json!({
            "prompt": body.prompt,
            "source": if body.source.is_empty() { "tnexus_ui" } else { body.source.as_str() },
            "access_tokens": body.access_tokens,
            "accounts": accounts,
        }),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(data))
}

async fn nurture_process_one(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<NurtureProcessOneBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if !account_ops::ops_available(&state) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "定向对话未配置（需 ACCOUNT_OPS_TOKEN + GPTIMAGE_ROOT）".into(),
        ));
    }
    let mut payload = json!({
        "prompt": body.prompt,
        "access_token": body.access_token,
        "email": body.email,
        "source": if body.source.is_empty() { "tnexus_accounts_ui" } else { body.source.as_str() },
    });
    if !body.access_token.is_empty() {
        if let Some(account) = state.accounts.export_account_for_token(&body.access_token).await {
            payload["account"] = account;
        }
    }
    let data = account_ops::nurture_process_one(&state, payload)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e))?;
    if let Some(account) = data.get("account").cloned() {
        let _ = state.accounts.merge_remote_items(&[account]).await;
    } else if let Some(updated) = data.get("updated_account").cloned() {
        let _ = state.accounts.merge_remote_items(&[updated]).await;
    }
    Ok(Json(data))
}

async fn ip_nurture_presets(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    Ok(Json(state.nurture_store.presets()))
}

async fn ip_nurture_bindings(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    Ok(Json(state.nurture_store.bindings().await))
}

async fn ip_nurture_save_binding(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<IpNurtureBindingBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let matrix = body.custom_matrix.map(|rows| json!(rows));
    let data = state
        .nurture_store
        .save_binding(&body.binding_key, &body.preset_id, matrix)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(data))
}

async fn image_pipeline_snapshot(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    Ok(Json(local_pipeline_snapshot(&state).await))
}

async fn local_pipeline_snapshot(state: &AppState) -> Value {
    let rows = sqlx::query(
        r#"SELECT phase_timings_ms, status, updated_at, created_at
           FROM jobs
           WHERE phase_timings_ms IS NOT NULL
             AND phase_timings_ms::text <> '{}'
           ORDER BY updated_at DESC
           LIMIT 200"#,
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let mut ps_ms = 0.0;
    let mut sse_ms = 0.0;
    let mut download_ms = 0.0;
    let mut count = 0usize;
    for row in &rows {
        let timings: serde_json::Value = row.get("phase_timings_ms");
        if let Some(v) = timings.get("ps_ms").and_then(|v| v.as_f64()) {
            ps_ms += v;
        }
        if let Some(v) = timings.get("sse_ms").and_then(|v| v.as_f64()) {
            sse_ms += v;
        }
        if let Some(v) = timings.get("download_ms").and_then(|v| v.as_f64()) {
            download_ms += v;
        }
        count += 1;
    }
    let denom = count.max(1) as f64;
    json!({
        "source": "tnexus-local",
        "sample_count": count,
        "avg_phase_ms": {
            "ps": ps_ms / denom,
            "sse": sse_ms / denom,
            "download": download_ms / denom,
        },
    })
}

async fn risk_metrics(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    let breakdown = state.accounts.schedulable_breakdown().await;
    Ok(Json(json!({
        "source": "tnexus-local",
        "schedulable_breakdown": breakdown,
    })))
}
