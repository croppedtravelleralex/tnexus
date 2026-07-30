use crate::middleware::{AdminUser, AuthUser};
use crate::state::AppState;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::Duration;
use tnexus_auth::User;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub display_name: String,
    pub disabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<User> for UserResponse {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            email: u.email,
            role: u.role.as_str().to_string(),
            display_name: u.display_name,
            disabled: u.disabled,
            created_at: u.created_at,
        }
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/preferences", get(get_preferences).patch(patch_preferences))
        .route("/users", get(list_users))
        .route("/users/{id}/disabled", post(set_disabled))
}

async fn register(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let display_name = body
        .display_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| body.email.split('@').next().unwrap_or("user").to_string());
    let user = state
        .auth
        .register(&body.email, &body.password, &display_name)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let token = state
        .auth
        .login(&body.email, &body.password)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .1;
    let jar = set_session_cookie(&state, jar, &token);
    Ok((jar, Json(UserResponse::from(user))))
}

async fn login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (user, token) = state
        .auth
        .login(&body.email, &body.password)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid credentials".into()))?;
    let jar = set_session_cookie(&state, jar, &token);
    Ok((jar, Json(UserResponse::from(user))))
}

async fn logout(jar: CookieJar, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut cookie = Cookie::new(state.config.cookie_name.clone(), "");
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_max_age(Duration::seconds(0));
    (jar.remove(cookie), Json(serde_json::json!({"ok": true})))
}

async fn me(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<UserResponse>, (StatusCode, String)> {
    let id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let u = state
        .auth
        .get_user(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "user not found".into()))?;
    Ok(Json(UserResponse::from(u)))
}

async fn get_preferences(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let row: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT preferences FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(row.unwrap_or_else(|| serde_json::json!({}))))
}

#[derive(Deserialize)]
struct PatchPreferencesBody {
    preferences: serde_json::Value,
}

async fn patch_preferences(
    State(state): State<Arc<AppState>>,
    user: AuthUser,
    Json(body): Json<PatchPreferencesBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let id = Uuid::parse_str(&user.claims.sub)
        .map_err(|_| (StatusCode::BAD_REQUEST, "bad user id".into()))?;
    let updated: serde_json::Value = sqlx::query_scalar(
        "UPDATE users SET preferences = $2 WHERE id = $1 RETURNING preferences",
    )
    .bind(id)
    .bind(body.preferences)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(updated))
}

async fn list_users(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserResponse>>, (StatusCode, String)> {
    let users = state
        .auth
        .list_users()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(users.into_iter().map(UserResponse::from).collect()))
}

#[derive(Deserialize)]
struct DisabledBody {
    disabled: bool,
}

async fn set_disabled(
    State(state): State<Arc<AppState>>,
    _admin: AdminUser,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<DisabledBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .auth
        .set_disabled(id, body.disabled)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({"ok": true})))
}

fn set_session_cookie(state: &AppState, jar: CookieJar, token: &str) -> CookieJar {
    let mut cookie = Cookie::new(state.config.cookie_name.clone(), token.to_string());
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(state.config.cookie_secure);
    cookie.set_max_age(Duration::seconds(state.config.jwt_ttl_secs as i64));
    jar.add(cookie)
}
