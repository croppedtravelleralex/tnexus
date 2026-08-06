use crate::config::JOB_EVENTS_PREFIX;
use crate::config::JOB_QUEUE_KEY;
use crate::jobs::{
    create_job, delete_jobs, get_job_detail, get_job_status, list_job_summaries, list_jobs,
};
use crate::middleware::AuthUser;
use crate::models::{
    parse_director_models, parse_mode, parse_provider, parse_workflow, JobDetail, JobListItem,
    JobRecord,
};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use futures::{stream::Stream, StreamExt};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tnexus_domain::factors::FactorPoint;
use tnexus_domain::gen_config::GenConfig;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateJobBody {
    pub mode: String,
    pub workflow_path: String,
    pub ps_enabled: bool,
    pub provider: String,
    #[serde(default)]
    pub director_models: Vec<String>,
    pub gen_config: GenConfig,
    pub director_factors: FactorPoint,
    pub ps_factors: FactorPoint,
    pub input_prompt: String,
    #[serde(default)]
    pub conversation_id: Option<Uuid>,
    #[serde(default)]
    pub actor_image_counts: Value,
}

#[derive(Serialize)]
struct CreateJobResponse {
    job_id: Uuid,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/",
            post(create_job_handler)
                .get(list_jobs_handler)
                .delete(delete_jobs_handler),
        )
        .route("/summaries", get(list_summaries_handler))
        .route("/{id}", get(get_job_handler))
        .route("/{id}/status", get(get_job_status_handler))
        .route("/{id}/events", get(job_events_handler))
}

async fn create_job_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<CreateJobBody>,
) -> Result<Json<CreateJobResponse>, (StatusCode, String)> {
    if body.input_prompt.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "prompt required".into()));
    }
    let _ = parse_mode(&body.mode).ok_or((StatusCode::BAD_REQUEST, "invalid mode".into()))?;
    let _ = parse_workflow(&body.workflow_path)
        .ok_or((StatusCode::BAD_REQUEST, "invalid workflow_path".into()))?;
    let _ = parse_provider(&body.provider)
        .ok_or((StatusCode::BAD_REQUEST, "invalid provider".into()))?;

    let models_input = if body.director_models.is_empty() {
        vec!["gpt".into()]
    } else {
        body.director_models.clone()
    };
    if !parse_director_models(&models_input) {
        return Err((StatusCode::BAD_REQUEST, "invalid director_models".into()));
    }

    let director_models = if body.mode == "casting" {
        models_input
    } else {
        vec![models_input
            .first()
            .cloned()
            .unwrap_or_else(|| "gpt".into())]
    };

    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;

    let ps_enabled = if body.workflow_path == "keyword_ps" {
        true
    } else {
        body.ps_enabled
    };

    let job = create_job(
        &state,
        user_id,
        &body.mode,
        &body.workflow_path,
        ps_enabled,
        &body.provider,
        director_models,
        body.gen_config,
        body.director_factors,
        body.ps_factors,
        body.input_prompt.trim(),
        body.conversation_id,
        if body.actor_image_counts.is_null() {
            serde_json::json!({})
        } else {
            body.actor_image_counts
        },
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut redis = state.redis.clone();
    redis
        .rpush::<_, _, ()>(JOB_QUEUE_KEY, job.id.to_string())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(CreateJobResponse { job_id: job.id }))
}

async fn list_jobs_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Vec<JobRecord>>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let jobs = list_jobs(&state, user_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(jobs))
}

async fn list_summaries_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Vec<JobListItem>>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let items = list_job_summaries(&state, user_id, 100)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items))
}

#[derive(Deserialize)]
struct DeleteJobsBody {
    ids: Vec<Uuid>,
}

async fn delete_jobs_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<DeleteJobsBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let deleted = delete_jobs(&state, user_id, &body.ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true, "deleted": deleted })))
}

async fn get_job_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<JobDetail>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let detail = get_job_detail(&state, id, user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    Ok(Json(detail))
}

#[derive(Serialize)]
struct JobStatusResponse {
    status: String,
    error_message: Option<String>,
    progress: u8,
}

async fn get_job_status_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatusResponse>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let row = get_job_status(&state, id, user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    let (status, error_message) = row;
    Ok(Json(JobStatusResponse {
        progress: progress_for_status(&status),
        status,
        error_message,
    }))
}

async fn job_events_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let job = crate::jobs::get_job(&state, id, Some(user_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;

    let channel = format!("{JOB_EVENTS_PREFIX}{id}");
    let redis_client = state.redis_client.clone();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        if let Ok(mut pubsub) = redis_client.get_async_pubsub().await {
            if pubsub.subscribe(&channel).await.is_ok() {
                let mut msg_stream = pubsub.on_message();
                while let Some(msg) = msg_stream.next().await {
                    if let Ok(payload) = msg.get_payload::<String>() {
                        if event_tx.send(payload).is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let initial = serde_json::json!({
        "job_id": id,
        "stage": job.status,
        "progress": progress_for_status(&job.status),
    });

    let stream = async_stream::stream! {
        yield Ok(Event::default().data(initial.to_string()));
        if job.status == "done" || job.status == "failed" {
            return;
        }
        loop {
            tokio::select! {
                payload = event_rx.recv() => {
                    match payload {
                        Some(payload) => {
                            yield Ok(Event::default().data(payload.clone()));
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) {
                                let stage = v.get("stage").and_then(|s| s.as_str()).unwrap_or("");
                                if stage == "done" || stage == "failed" {
                                    break;
                                }
                            }
                        }
                        None => break,
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(900)) => {
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn progress_for_status(status: &str) -> u8 {
    match status {
        "queued" => 5,
        "directing" => 25,
        "generating" => 55,
        "uploading" => 85,
        "done" => 100,
        "failed" => 0,
        _ => 0,
    }
}
