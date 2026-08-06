use crate::middleware::AuthUser;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRow {
    pub id: Uuid,
    pub title: String,
    pub state: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
pub struct CreateConversationBody {
    pub title: Option<String>,
    pub state: Option<Value>,
}

#[derive(Deserialize)]
pub struct PatchConversationBody {
    pub title: Option<String>,
    pub state: Option<Value>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_handler).post(create_handler))
        .route(
            "/{id}",
            get(get_handler).patch(patch_handler).delete(delete_handler),
        )
}

async fn list_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<Vec<ConversationRow>>, (StatusCode, String)> {
    let uid = parse_uid(&user)?;
    let rows = sqlx::query(
        "SELECT id, title, state, created_at, updated_at FROM conversations WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 100",
    )
    .bind(uid)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(rows.into_iter().filter_map(row_to_conv).collect()))
}

async fn create_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<CreateConversationBody>,
) -> Result<Json<ConversationRow>, (StatusCode, String)> {
    let uid = parse_uid(&user)?;
    let title = body.title.unwrap_or_else(|| "新对话".into());
    let state_json = body.state.unwrap_or_else(|| serde_json::json!({}));
    let row = sqlx::query(
        r#"INSERT INTO conversations (user_id, title, state) VALUES ($1, $2, $3)
           RETURNING id, title, state, created_at, updated_at"#,
    )
    .bind(uid)
    .bind(title)
    .bind(state_json)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    row_to_conv(row).ok_or(internal("row")).map(Json)
}

async fn get_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ConversationRow>, (StatusCode, String)> {
    let uid = parse_uid(&user)?;
    let row = sqlx::query(
        "SELECT id, title, state, created_at, updated_at FROM conversations WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(uid)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    row_to_conv(row).ok_or(internal("row")).map(Json)
}

async fn delete_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let uid = parse_uid(&user)?;
    let result = sqlx::query("DELETE FROM conversations WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(uid)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "not found".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn patch_handler(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchConversationBody>,
) -> Result<Json<ConversationRow>, (StatusCode, String)> {
    let uid = parse_uid(&user)?;
    let existing = sqlx::query(
        "SELECT id, title, state, created_at, updated_at FROM conversations WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(uid)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .ok_or((StatusCode::NOT_FOUND, "not found".into()))?;
    let existing = row_to_conv(existing).ok_or(internal("row"))?;

    let title = body.title.unwrap_or(existing.title);
    let state_json = body.state.unwrap_or(existing.state);

    let row = sqlx::query(
        r#"UPDATE conversations SET title = $3, state = $4, updated_at = NOW()
           WHERE id = $1 AND user_id = $2
           RETURNING id, title, state, created_at, updated_at"#,
    )
    .bind(id)
    .bind(uid)
    .bind(title)
    .bind(state_json)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    row_to_conv(row).ok_or(internal("row")).map(Json)
}

fn row_to_conv(row: sqlx::postgres::PgRow) -> Option<ConversationRow> {
    Some(ConversationRow {
        id: row.get("id"),
        title: row.get("title"),
        state: row.get("state"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn parse_uid(user: &AuthUser) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(&user.claims.sub).map_err(|_| (StatusCode::BAD_REQUEST, "bad user".into()))
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
