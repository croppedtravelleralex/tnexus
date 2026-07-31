use crate::gptimage_proxy::{admin_token, proxy_get, proxy_post};
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

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/summary", get(ops_summary))
        .route("/nurture/status", get(nurture_status))
        .route("/nurture/enable", post(nurture_enable))
        .route("/nurture/enqueue", post(nurture_enqueue))
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
    })))
}

async fn nurture_status(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&state).is_some() {
        let data = proxy_get(&state, "/api/ops/nurture/status", "").await?;
        return Ok(Json(data));
    }
    Ok(Json(json!({
        "enabled": false,
        "running": false,
        "queue": { "depth": 0, "oldest_age_sec": 0 },
        "completed_in_day": 0,
        "max_per_account_per_day": 0,
        "last_error": null,
        "source": "tnexus-local",
    })))
}

async fn nurture_enable(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<NurtureEnableBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&state).is_some() {
        let data = proxy_post(
            &state,
            "/api/ops/nurture/enable",
            json!({ "enabled": body.enabled }),
        )
        .await?;
        return Ok(Json(data));
    }
    Ok(Json(json!({ "enabled": body.enabled, "source": "tnexus-local" })))
}

async fn nurture_enqueue(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<NurtureEnqueueBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&state).is_some() {
        let data = proxy_post(
            &state,
            "/api/ops/nurture/enqueue",
            json!({
                "prompt": body.prompt,
                "source": if body.source.is_empty() { "tnexus_ui" } else { body.source.as_str() },
                "access_tokens": body.access_tokens,
            }),
        )
        .await?;
        return Ok(Json(data));
    }
    Err((
        StatusCode::SERVICE_UNAVAILABLE,
        "养号队列需要配置 GPTIMAGE_ADMIN_TOKEN 并连接 gptimage".into(),
    ))
}

async fn image_pipeline_snapshot(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&state).is_some() {
        if let Ok(data) = proxy_get(&state, "/api/ops/image-pipeline/snapshot", "").await {
            return Ok(Json(data));
        }
    }
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
        let ps = timings.get("ps_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let sse = timings
            .get("sse_stream_ms")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let dl = timings.get("download_ms").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if ps + sse + dl <= 0.0 {
            continue;
        }
        ps_ms += ps;
        sse_ms += sse;
        download_ms += dl;
        count += 1;
    }
    let avg = |sum: f64| if count > 0 { sum / count as f64 } else { 0.0 };

    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM jobs WHERE status NOT IN ('done', 'failed')",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    json!({
        "source": "tnexus-local",
        "in_flight": running,
        "ps_queue_depth": running,
        "ss_queue_depth": running,
        "ps": { "limit": 4, "active": running.min(4), "queued": (running - running.min(4)).max(0) },
        "ss": { "limit": 8, "active": running.min(8), "queued": (running - running.min(8)).max(0) },
        "slot_topology": {
            "ps_capacity": 4,
            "ss_capacity": 8,
            "ps_inflight": running.min(4),
            "ss_inflight": running.min(8),
            "ps_queued": (running - running.min(4)).max(0),
            "ss_queued": (running - running.min(8)).max(0),
            "pipeline_in_flight": running,
        },
        "phase_avg_ms": {
            "ps_ms": avg(ps_ms),
            "sse_stream_ms": avg(sse_ms),
            "download_ms": avg(download_ms),
            "samples": count,
        },
        "segments": [],
    })
}

async fn risk_metrics(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    if admin_token(&state).is_some() {
        if let Ok(data) = proxy_get(&state, "/api/ops/risk-audit/status", "").await {
            return Ok(Json(data));
        }
    }
    let failed_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM jobs WHERE status = 'failed' AND updated_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let done_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM jobs WHERE status = 'done' AND updated_at > NOW() - INTERVAL '24 hours'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "source": "tnexus-local",
        "jobs_failed_24h": failed_24h,
        "jobs_done_24h": done_24h,
        "failure_rate_24h": if done_24h + failed_24h > 0 {
            failed_24h as f64 / (done_24h + failed_24h) as f64
        } else {
            0.0
        },
    })))
}
