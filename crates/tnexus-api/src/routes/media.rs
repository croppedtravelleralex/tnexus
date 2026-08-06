use crate::jobs::result_to_view;
use crate::middleware::{AdminUser, AuthUser};
use tnexus_auth::Role;
use crate::models::JobResultRecord;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use tnexus_image_archive::get_newapi_user_id;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct LogsQuery {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    start_date: String,
    #[serde(default)]
    end_date: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    outcome: String,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    200
}

#[derive(Debug, Deserialize)]
struct DeleteLogsBody {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ImagesQuery {
    #[serde(default)]
    start_date: String,
    #[serde(default)]
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct SetImageTagsBody {
    path: String,
    tags: Vec<String>,
}

fn parse_tags(value: Option<serde_json::Value>) -> Vec<String> {
    let Some(v) = value else { return vec![] };
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    if let Some(obj) = v.as_object() {
        if let Some(tags) = obj.get("tags").and_then(|t| t.as_array()) {
            return tags
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
    }
    vec![]
}

fn tags_to_json(tags: &[String]) -> serde_json::Value {
    serde_json::Value::Array(tags.iter().map(|t| serde_json::Value::String(t.clone())).collect())
}

#[derive(Debug, Deserialize)]
struct DeleteImagesBody {
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    start_date: String,
    #[serde(default)]
    end_date: String,
    #[serde(default)]
    all_matching: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_logs))
        .route("/delete", post(delete_logs))
}

pub fn image_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_images))
        .route("/thumb/{id}", get(get_image_thumb))
        .route("/original/{id}", get(get_image_original))
        .route("/delete", post(delete_images))
        .route("/tags", get(list_tags).post(set_image_tags))
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.trim().is_empty() {
        return None;
    }
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

fn format_log_time(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

async fn list_logs(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let log_type = q.r#type.trim();
    if log_type.is_empty() || log_type == "call" {
        let start = parse_date(&q.start_date);
        let end = parse_date(&q.end_date);
        let limit = q.limit.clamp(1, 2000);

        let rows = sqlx::query(
            r#"SELECT id, input_prompt, status, error_message, provider, created_at, updated_at, phase_timings_ms
               FROM jobs
               WHERE ($1::date IS NULL OR created_at::date >= $1)
                 AND ($2::date IS NULL OR created_at::date <= $2)
               ORDER BY created_at DESC
               LIMIT $3"#,
        )
        .bind(start)
        .bind(end)
        .bind(limit)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mut items = Vec::new();
        for row in rows {
            let id: Uuid = row.get("id");
            let status: String = row.get("status");
            let created_at: DateTime<Utc> = row.get("created_at");
            let updated_at: DateTime<Utc> = row.get("updated_at");
            let prompt: String = row.get("input_prompt");
            let provider: String = row.get("provider");
            let error_message: Option<String> = row.get("error_message");
            let phase_timings: serde_json::Value = row.get("phase_timings_ms");

            let wall_ms = phase_timings
                .get("wall_clock_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| (updated_at - created_at).num_milliseconds().max(0) as u64);
            let call_status = if status == "done" {
                "success"
            } else if status == "failed" {
                "failed"
            } else {
                "running"
            };

            let summary = if status == "failed" {
                format!("调用失败 · {provider}")
            } else if status == "done" {
                format!("调用完成 · {provider}")
            } else {
                format!("进行中 · {provider}")
            };

            items.push(json!({
                "id": id.to_string(),
                "time": format_log_time(created_at),
                "type": "call",
                "summary": summary,
                "detail": {
                    "status": call_status,
                    "task_id": id.to_string(),
                    "provider": provider,
                    "prompt": prompt,
                    "total_wall_ms": wall_ms,
                    "duration_ms": wall_ms,
                    "error": error_message,
                    "phase_timings_ms": phase_timings,
                }
            }));
        }
        return Ok(Json(json!({ "items": items })));
    }

    Ok(Json(json!({ "items": [] })))
}

async fn delete_logs(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    Json(body): Json<DeleteLogsBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let ids: Vec<Uuid> = body
        .ids
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();
    if ids.is_empty() {
        return Ok(Json(json!({ "removed": 0 })));
    }
    let result = sqlx::query("DELETE FROM jobs WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "removed": result.rows_affected() })))
}

async fn list_images(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Query(q): Query<ImagesQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let start = parse_date(&q.start_date);
    let end = parse_date(&q.end_date);
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let is_admin = user.claims.role == Role::Admin;
    let bound_newapi = if is_admin {
        None
    } else {
        get_newapi_user_id(&state.pool, user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let mut items = Vec::new();

    let worker_rows = if is_admin {
        sqlx::query(
            r#"SELECT jr.id, jr.job_id, jr.provider, jr.r2_key_original, jr.r2_key_preview,
                      jr.r2_key_thumb, jr.agent_prompt, jr.revised_prompt, jr.keywords, jr.inline_preview_b64,
                      jr.source_url, jr.width, jr.height, jr.size_bytes, jr.generation_ms, jr.created_at,
                      j.input_prompt, j.updated_at, j.phase_timings_ms, j.status
               FROM job_results jr
               JOIN jobs j ON j.id = jr.job_id
               WHERE ($1::date IS NULL OR jr.created_at::date >= $1)
                 AND ($2::date IS NULL OR jr.created_at::date <= $2)
               ORDER BY jr.created_at DESC
               LIMIT 2000"#,
        )
        .bind(start)
        .bind(end)
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query(
            r#"SELECT jr.id, jr.job_id, jr.provider, jr.r2_key_original, jr.r2_key_preview,
                      jr.r2_key_thumb, jr.agent_prompt, jr.revised_prompt, jr.keywords, jr.inline_preview_b64,
                      jr.source_url, jr.width, jr.height, jr.size_bytes, jr.generation_ms, jr.created_at,
                      j.input_prompt, j.updated_at, j.phase_timings_ms, j.status
               FROM job_results jr
               JOIN jobs j ON j.id = jr.job_id
               WHERE j.user_id = $3
                 AND ($1::date IS NULL OR jr.created_at::date >= $1)
                 AND ($2::date IS NULL OR jr.created_at::date <= $2)
               ORDER BY jr.created_at DESC
               LIMIT 2000"#,
        )
        .bind(start)
        .bind(end)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for row in worker_rows {
        if let Some(item) = worker_row_to_item(&state, &row).await? {
            items.push(item);
        }
    }

    let api_rows = if is_admin {
        sqlx::query(
            r#"SELECT id, prompt, agent_prompt, r2_key_original, r2_key_preview, r2_key_thumb,
                      inline_preview_b64, source_url, width, height, size_bytes, generation_ms,
                      keywords, created_at, backup_status, newapi_user_id, newapi_token_name
               FROM user_image_records
               WHERE source = 'gateway_openapi'
                 AND ($1::date IS NULL OR created_at::date >= $1)
                 AND ($2::date IS NULL OR created_at::date <= $2)
               ORDER BY created_at DESC
               LIMIT 2000"#,
        )
        .bind(start)
        .bind(end)
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query(
            r#"SELECT id, prompt, agent_prompt, r2_key_original, r2_key_preview, r2_key_thumb,
                      inline_preview_b64, source_url, width, height, size_bytes, generation_ms,
                      keywords, created_at, backup_status, newapi_user_id, newapi_token_name
               FROM user_image_records
               WHERE source = 'gateway_openapi'
                 AND (
                   owner_user_id = $3
                   OR ($4::bigint IS NOT NULL AND newapi_user_id = $4)
                 )
                 AND ($1::date IS NULL OR created_at::date >= $1)
                 AND ($2::date IS NULL OR created_at::date <= $2)
               ORDER BY created_at DESC
               LIMIT 2000"#,
        )
        .bind(start)
        .bind(end)
        .bind(user_id)
        .bind(bound_newapi)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for row in api_rows {
        if let Some(item) = api_row_to_item(&state, &row).await? {
            items.push(item);
        }
    }

    items.sort_by(|a, b| {
        let ta = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        let tb = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        tb.cmp(ta)
    });
    items.truncate(2000);

    Ok(Json(json!({ "items": items })))
}

async fn worker_row_to_item(
    state: &AppState,
    row: &sqlx::postgres::PgRow,
) -> Result<Option<Value>, (StatusCode, String)> {
        let record = JobResultRecord {
            id: row.get("id"),
            job_id: row.get("job_id"),
            provider: row.get("provider"),
            r2_key_original: row.get("r2_key_original"),
            r2_key_preview: row.get("r2_key_preview"),
            r2_key_thumb: row.get("r2_key_thumb"),
            agent_prompt: row.get("agent_prompt"),
            revised_prompt: row.get("revised_prompt"),
            keywords: row.get("keywords"),
            inline_preview_b64: row.get("inline_preview_b64"),
            source_url: row.get("source_url"),
            width: row.get("width"),
            height: row.get("height"),
            size_bytes: row.get("size_bytes"),
            generation_ms: row.get("generation_ms"),
            created_at: row.get("created_at"),
        };
        let prompt: String = row.get("input_prompt");
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");
        let phase_timings: serde_json::Value = row.get("phase_timings_ms");
        let wall_ms = phase_timings
            .get("wall_clock_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| (updated_at - created_at).num_milliseconds().max(0) as u64);
        let has_r2_thumb = record
            .r2_key_thumb
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let view = result_to_view(&state, record)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let preview = view.preview_url.clone();
        let thumb = view.thumb_url.clone();
        let download = view.download_url.clone();
        let url = download
            .clone()
            .or(preview.clone())
            .or(thumb.clone())
            .unwrap_or_default();
        let has_inline = view
            .preview_b64
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let thumb_api_url = if has_inline || has_r2_thumb {
            Some(format!("/api/images/thumb/{}", view.id))
        } else {
            None
        };
        if url.is_empty() && thumb_api_url.is_none() {
            return Ok(None);
        }
        let keywords: Option<serde_json::Value> = row.get("keywords");
        let tags = parse_tags(keywords);
        let size_bytes: Option<i64> = row.get("size_bytes");
        Ok(Some(json!({
            "rel": view.id.to_string(),
            "name": format!("{}.png", view.id),
            "date": created_at.format("%Y-%m-%d").to_string(),
            "size": size_bytes.unwrap_or(0),
            "url": url,
            "thumbnail_url": thumb.clone().or(Some(url.clone())),
            "thumb_api_url": thumb_api_url,
            "b64_json": view.b64_json,
            "preview_b64": view.preview_b64,
            "created_at": created_at.to_rfc3339(),
            "duration_ms": wall_ms,
            "prompt": prompt,
            "tags": tags,
            "source": "studio",
        })))
}

async fn api_row_to_item(
    _state: &AppState,
    row: &sqlx::postgres::PgRow,
) -> Result<Option<Value>, (StatusCode, String)> {
    let id: Uuid = row.get("id");
    let prompt: String = row
        .get::<Option<String>, _>("prompt")
        .filter(|s| !s.is_empty())
        .or_else(|| row.get("agent_prompt"))
        .unwrap_or_default();
    let created_at: DateTime<Utc> = row.get("created_at");
    let generation_ms: Option<i64> = row.get("generation_ms");
    let size_bytes: Option<i64> = row.get("size_bytes");
    let keywords: Option<serde_json::Value> = row.get("keywords");
    let backup_status: String = row.get("backup_status");
    let inline_preview_b64: Option<String> = row.get("inline_preview_b64");
    let source_url: Option<String> = row.get("source_url");
    let r2_key_thumb: Option<String> = row.get("r2_key_thumb");

    let has_inline = inline_preview_b64
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_r2_thumb = r2_key_thumb
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let thumb_api_url = if has_inline || has_r2_thumb {
        Some(format!("/api/images/thumb/{}", id))
    } else {
        None
    };
    let url = source_url.clone().unwrap_or_default();
    if url.is_empty() && thumb_api_url.is_none() && !has_inline {
        return Ok(None);
    }
    let tags = parse_tags(keywords);
    Ok(Some(json!({
        "rel": id.to_string(),
        "name": format!("{}.png", id),
        "date": created_at.format("%Y-%m-%d").to_string(),
        "size": size_bytes.unwrap_or(0),
        "url": url,
        "thumbnail_url": if url.is_empty() { None } else { Some(url.clone()) },
        "thumb_api_url": thumb_api_url,
        "preview_b64": inline_preview_b64,
        "created_at": created_at.to_rfc3339(),
        "duration_ms": generation_ms.unwrap_or(0),
        "prompt": prompt,
        "tags": tags,
        "source": "api",
        "backup_status": backup_status,
        "newapi_token_name": row.get::<Option<String>, _>("newapi_token_name"),
    })))
}

async fn delete_images(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<DeleteImagesBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let is_admin = user.claims.role == Role::Admin;

    if body.all_matching {
        let start = parse_date(&body.start_date);
        let end = parse_date(&body.end_date);
        if is_admin {
            let jr = sqlx::query(
                r#"DELETE FROM job_results jr
                   WHERE ($1::date IS NULL OR jr.created_at::date >= $1)
                     AND ($2::date IS NULL OR jr.created_at::date <= $2)"#,
            )
            .bind(start)
            .bind(end)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let ur = sqlx::query(
                r#"DELETE FROM user_image_records
                   WHERE ($1::date IS NULL OR created_at::date >= $1)
                     AND ($2::date IS NULL OR created_at::date <= $2)"#,
            )
            .bind(start)
            .bind(end)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            return Ok(Json(json!({
                "removed": jr.rows_affected() + ur.rows_affected()
            })));
        }
        let bound_newapi = get_newapi_user_id(&state.pool, user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let jr = sqlx::query(
            r#"DELETE FROM job_results jr
               USING jobs j
               WHERE j.id = jr.job_id AND j.user_id = $3
                 AND ($1::date IS NULL OR jr.created_at::date >= $1)
                 AND ($2::date IS NULL OR jr.created_at::date <= $2)"#,
        )
        .bind(start)
        .bind(end)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let ur = sqlx::query(
            r#"DELETE FROM user_image_records
               WHERE (owner_user_id = $3 OR ($4::bigint IS NOT NULL AND newapi_user_id = $4))
                 AND ($1::date IS NULL OR created_at::date >= $1)
                 AND ($2::date IS NULL OR created_at::date <= $2)"#,
        )
        .bind(start)
        .bind(end)
        .bind(user_id)
        .bind(bound_newapi)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(json!({
            "removed": jr.rows_affected() + ur.rows_affected()
        })));
    }

    let ids: Vec<Uuid> = body
        .paths
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();
    if ids.is_empty() {
        return Ok(Json(json!({ "removed": 0 })));
    }

    let mut removed = 0u64;
    if is_admin {
        removed += sqlx::query("DELETE FROM job_results WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .rows_affected();
        removed += sqlx::query("DELETE FROM user_image_records WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .rows_affected();
    } else {
        let bound_newapi = get_newapi_user_id(&state.pool, user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        removed += sqlx::query(
            r#"DELETE FROM job_results jr
               USING jobs j
               WHERE jr.id = ANY($1) AND j.id = jr.job_id AND j.user_id = $2"#,
        )
        .bind(&ids)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .rows_affected();
        removed += sqlx::query(
            r#"DELETE FROM user_image_records
               WHERE id = ANY($1)
                 AND (owner_user_id = $2 OR ($3::bigint IS NOT NULL AND newapi_user_id = $3))"#,
        )
        .bind(&ids)
        .bind(user_id)
        .bind(bound_newapi)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .rows_affected();
    }
    Ok(Json(json!({ "removed": removed })))
}

async fn list_tags(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Value>, (StatusCode, String)> {
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let is_admin = user.claims.role == Role::Admin;
    let bound_newapi = if is_admin {
        None
    } else {
        get_newapi_user_id(&state.pool, user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let rows = if is_admin {
        sqlx::query(
            r#"SELECT keywords FROM job_results WHERE keywords IS NOT NULL
               UNION ALL
               SELECT keywords FROM user_image_records WHERE keywords IS NOT NULL"#,
        )
        .fetch_all(&state.pool)
        .await
    } else {
        sqlx::query(
            r#"SELECT jr.keywords FROM job_results jr
               JOIN jobs j ON j.id = jr.job_id
               WHERE j.user_id = $1 AND jr.keywords IS NOT NULL
               UNION ALL
               SELECT keywords FROM user_image_records
               WHERE keywords IS NOT NULL
                 AND (owner_user_id = $1 OR ($2::bigint IS NOT NULL AND newapi_user_id = $2))"#,
        )
        .bind(user_id)
        .bind(bound_newapi)
        .fetch_all(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut set = std::collections::BTreeSet::new();
    for row in rows {
        let kw: Option<serde_json::Value> = row.get("keywords");
        for tag in parse_tags(kw) {
            set.insert(tag);
        }
    }
    Ok(Json(json!({ "tags": set.into_iter().collect::<Vec<_>>() })))
}

async fn set_image_tags(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<SetImageTagsBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let id = Uuid::parse_str(body.path.trim())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid path".into()))?;
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let is_admin = user.claims.role == Role::Admin;
    let tags = body
        .tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    let tags_json = tags_to_json(&tags);

    if is_admin {
        let jr = sqlx::query("UPDATE job_results SET keywords = $2 WHERE id = $1")
            .bind(id)
            .bind(&tags_json)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if jr.rows_affected() > 0 {
            return Ok(Json(json!({ "ok": true, "tags": tags })));
        }
        sqlx::query("UPDATE user_image_records SET keywords = $2 WHERE id = $1")
            .bind(id)
            .bind(&tags_json)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(json!({ "ok": true, "tags": tags })));
    }

    let bound_newapi = get_newapi_user_id(&state.pool, user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let jr = sqlx::query(
        r#"UPDATE job_results jr SET keywords = $3
           FROM jobs j
           WHERE jr.id = $1 AND j.id = jr.job_id AND j.user_id = $2"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(&tags_json)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if jr.rows_affected() > 0 {
        return Ok(Json(json!({ "ok": true, "tags": tags })));
    }
    sqlx::query(
        r#"UPDATE user_image_records SET keywords = $4
           WHERE id = $1
             AND (owner_user_id = $2 OR ($3::bigint IS NOT NULL AND newapi_user_id = $3))"#,
    )
    .bind(id)
    .bind(user_id)
    .bind(bound_newapi)
    .bind(&tags_json)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "ok": true, "tags": tags })))
}

#[derive(Debug, Deserialize)]
struct ThumbQuery {
    #[serde(default = "default_thumb_width")]
    w: u32,
}

fn default_thumb_width() -> u32 {
    240
}

async fn load_stored_bytes(
    state: &AppState,
    row: &sqlx::postgres::PgRow,
    prefer_thumb: bool,
) -> Result<Option<Vec<u8>>, (StatusCode, String)> {
    let key: Option<String> = if prefer_thumb {
        row.get::<Option<String>, _>("r2_key_thumb")
            .or_else(|| row.get("r2_key_preview"))
            .filter(|s| !s.trim().is_empty())
    } else {
        row.get::<Option<String>, _>("r2_key_original")
            .filter(|s| !s.trim().is_empty())
    };
    if let (Some(store), Some(key)) = (state.image_store.as_ref(), key) {
        let bytes = store
            .read_bytes(&key)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Some(bytes));
    }
    Ok(None)
}

async fn fetch_result_image_row(
    state: &AppState,
    id: Uuid,
    user_id: Uuid,
    is_admin: bool,
) -> Result<sqlx::postgres::PgRow, (StatusCode, String)> {
    let job_row = if is_admin {
        sqlx::query(
            "SELECT inline_preview_b64, r2_key_original, r2_key_thumb, r2_key_preview, source_url FROM job_results WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await
    } else {
        sqlx::query(
            "SELECT jr.inline_preview_b64, jr.r2_key_original, jr.r2_key_thumb, jr.r2_key_preview, jr.source_url
             FROM job_results jr
             JOIN jobs j ON j.id = jr.job_id
             WHERE jr.id = $1 AND j.user_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(row) = job_row {
        return Ok(row);
    }

    let bound_newapi = if is_admin {
        None
    } else {
        get_newapi_user_id(&state.pool, user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let api_row = if is_admin {
        sqlx::query(
            "SELECT inline_preview_b64, r2_key_original, r2_key_thumb, r2_key_preview, source_url
             FROM user_image_records WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await
    } else {
        sqlx::query(
            "SELECT inline_preview_b64, r2_key_original, r2_key_thumb, r2_key_preview, source_url
             FROM user_image_records
             WHERE id = $1
               AND (owner_user_id = $2 OR ($3::bigint IS NOT NULL AND newapi_user_id = $3))",
        )
        .bind(id)
        .bind(user_id)
        .bind(bound_newapi)
        .fetch_optional(&state.pool)
        .await
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or((StatusCode::NOT_FOUND, "image not found".into()))?;
    Ok(api_row)
}

async fn get_image_thumb(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<ThumbQuery>,
) -> Result<Response, (StatusCode, String)> {
    let id = Uuid::parse_str(id.trim())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid id".into()))?;
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let is_admin = user.claims.role == Role::Admin;
    let row = fetch_result_image_row(&state, id, user_id, is_admin).await?;

    let b64: Option<String> = row.get("inline_preview_b64");
    if let Some(raw) = b64.filter(|s| !s.trim().is_empty()) {
        return serve_resized_thumb(&raw, &headers, q.w);
    }

    if let Some(bytes) = load_stored_bytes(&state, &row, true).await? {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("decode image: {e}")))?;
        return encode_thumbnail(&img, &headers, q.w);
    }

    if let Some(url) = row
        .get::<Option<String>, _>("source_url")
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
    {
        return Ok(axum::response::Redirect::temporary(&url).into_response());
    }

    Err((StatusCode::NOT_FOUND, "no preview available".into()))
}

async fn get_image_original(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let id = Uuid::parse_str(id.trim())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid id".into()))?;
    let user_id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))?;
    let is_admin = user.claims.role == Role::Admin;
    let row = fetch_result_image_row(&state, id, user_id, is_admin).await?;

    let b64: Option<String> = row.get("inline_preview_b64");
    if let Some(raw) = b64.filter(|s| !s.trim().is_empty()) {
        return serve_original_bytes(&raw);
    }

    if let Some(bytes) = load_stored_bytes(&state, &row, false).await? {
        let content_type = sniff_image_content_type(&bytes);
        return Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response());
    }

    if let Some(url) = row
        .get::<Option<String>, _>("source_url")
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
    {
        return Ok(axum::response::Redirect::temporary(&url).into_response());
    }

    Err((StatusCode::NOT_FOUND, "no original available".into()))
}

fn serve_original_bytes(raw: &str) -> Result<Response, (StatusCode, String)> {
    let bytes = if raw.starts_with("data:") {
        raw.split_once(',').map(|(_, p)| p).unwrap_or(raw)
    } else {
        raw
    };
    let input = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("b64 decode: {e}")))?;
    let content_type = sniff_image_content_type(&input);
    Ok(([(header::CONTENT_TYPE, content_type)], input).into_response())
}

fn sniff_image_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && bytes[8..12] == *b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

fn serve_resized_thumb(raw: &str, headers: &HeaderMap, width: u32) -> Result<Response, (StatusCode, String)> {
    let bytes = if raw.starts_with("data:") {
        raw.split_once(',').map(|(_, p)| p).unwrap_or(raw)
    } else {
        raw
    };
    let input = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        bytes,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("b64 decode: {e}")))?;
    let img = image::load_from_memory(&input)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("decode image: {e}")))?;
    encode_thumbnail(&img, headers, width)
}

fn encode_thumbnail(
    img: &image::DynamicImage,
    headers: &HeaderMap,
    width: u32,
) -> Result<Response, (StatusCode, String)> {
    let max_w = width.clamp(64, 2048);
    let thumb = img.thumbnail(max_w, max_w);
    let use_webp = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("image/webp"))
        .unwrap_or(true);
    let mut buf = Vec::new();
    if use_webp {
        let mut cursor = std::io::Cursor::new(&mut buf);
        thumb
            .write_to(&mut cursor, image::ImageFormat::WebP)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(([(header::CONTENT_TYPE, "image/webp")], buf).into_response())
    } else {
        let mut cursor = std::io::Cursor::new(&mut buf);
        thumb
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(([(header::CONTENT_TYPE, "image/jpeg")], buf).into_response())
    }
}
