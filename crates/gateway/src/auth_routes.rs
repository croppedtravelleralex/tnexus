//! Auth HTTP routes and JWT extraction for gateway.

use crate::state::AppState;
use gateway_auth::{AuthError, AuthMode, AuthService, Claims, Role, User};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterBody {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub claims: Claims,
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or_else(|| auth_err(StatusCode::UNAUTHORIZED, "login required"))
    }
}

pub async fn login(
    State(st): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    let svc = &st.auth;
    let user = match svc.authenticate(&body.username, &body.password) {
        Ok(u) => u,
        Err(AuthError::InvalidCredentials) => {
            return auth_err(StatusCode::UNAUTHORIZED, "invalid credentials");
        }
        Err(AuthError::Disabled) => {
            return auth_err(StatusCode::FORBIDDEN, "account disabled");
        }
        Err(e) => return auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let token = match svc.issue_token(&user) {
        Ok(t) => t,
        Err(e) => return auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let cookie = build_session_cookie(svc, &token);
    // Token goes out only in the HttpOnly cookie; echoing it in the body would
    // put it back within reach of JS.
    let body = json!({
        "ok": true,
        "user": user_public(&user),
    });
    (jar.add(cookie), Json(body)).into_response()
}

pub async fn register(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    let svc = &st.auth;
    let cfg = svc.config();
    let caller = extract_claims(svc, &headers).ok();
    let role = if cfg.allow_public_register {
        Role::Member
    } else {
        match caller.as_ref().map(|c| c.role) {
            Some(Role::Admin) => {
                Role::parse(body.role.as_deref().unwrap_or("member")).unwrap_or(Role::Member)
            }
            _ => return auth_err(StatusCode::FORBIDDEN, "admin only registration"),
        }
    };
    match svc.create_user(&body.username, &body.password, role) {
        Ok(u) => (
            StatusCode::CREATED,
            Json(json!({"ok": true, "user": user_public(&u)})),
        )
            .into_response(),
        Err(AuthError::UserExists) => auth_err(StatusCode::CONFLICT, "username taken"),
        Err(e) => auth_err(StatusCode::BAD_REQUEST, &e.to_string()),
    }
}

pub async fn logout(
    State(st): State<Arc<AppState>>,
    jar: CookieJar,
    headers: HeaderMap,
) -> impl IntoResponse {
    if matches!(st.auth.config().mode, gateway_auth::AuthMode::Jwt) {
        if let Ok(claims) = extract_claims(&st.auth, &headers) {
            if let Some(ref jti) = claims.jti {
                let _ = st.auth.revoke_jti(jti, claims.exp);
            }
        }
    }
    let svc = &st.auth;
    let mut cookie = Cookie::build((svc.config().cookie_name.clone(), ""))
        .http_only(true)
        .path("/")
        .max_age(time::Duration::ZERO)
        .same_site(SameSite::Lax);
    if svc.config().cookie_secure {
        cookie = cookie.secure(true);
    }
    (jar.add(cookie), Json(json!({"ok": true}))).into_response()
}

pub async fn me(State(st): State<Arc<AppState>>, user: AuthUser) -> impl IntoResponse {
    if matches!(st.auth.config().mode, AuthMode::Disabled | AuthMode::ApiKey) {
        return Json(json!({
            "ok": true,
            "user": user_public_from_claims(&user.claims),
        }))
        .into_response();
    }
    match st.auth.get_user_by_id(&user.claims.sub) {
        Ok(u) => Json(json!({"ok": true, "user": user_public(&u)})).into_response(),
        Err(_) => auth_err(StatusCode::UNAUTHORIZED, "user not found"),
    }
}

pub async fn list_users(State(st): State<Arc<AppState>>, user: AuthUser) -> impl IntoResponse {
    if user.claims.role != Role::Admin {
        return auth_err(StatusCode::FORBIDDEN, "admin only");
    }
    match st.auth.list_users() {
        Ok(users) => {
            let list: Vec<Value> = users.iter().map(user_public).collect();
            Json(json!({"ok": true, "users": list})).into_response()
        }
        Err(e) => auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct DisableBody {
    pub disabled: bool,
}

pub async fn set_user_disabled(
    State(st): State<Arc<AppState>>,
    user: AuthUser,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(body): Json<DisableBody>,
) -> impl IntoResponse {
    if user.claims.role != Role::Admin {
        return auth_err(StatusCode::FORBIDDEN, "admin only");
    }
    match st.auth.set_disabled(&user_id, body.disabled) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(AuthError::NotFound) => auth_err(StatusCode::NOT_FOUND, "user not found"),
        Err(e) => auth_err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub async fn require_auth(
    State(st): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    match st.auth.config().mode {
        AuthMode::Disabled => {
            req.extensions_mut().insert(AuthUser {
                claims: Claims {
                    sub: "auth-disabled".into(),
                    username: "dev".into(),
                    role: Role::Admin,
                    exp: usize::MAX,
                    jti: None,
                },
            });
            return next.run(req).await;
        }
        AuthMode::ApiKey => {
            let expected = match st.auth.config().gateway_auth_key.as_deref() {
                Some(k) => k,
                None => {
                    return auth_err(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "api_key mode misconfigured",
                    )
                }
            };
            let token = match bearer_token(req.headers()) {
                Some(t) => t,
                None => return auth_err(StatusCode::UNAUTHORIZED, "missing bearer token"),
            };
            if !constant_time_eq(&token, expected) {
                return auth_err(StatusCode::UNAUTHORIZED, "invalid api key");
            }
            req.extensions_mut().insert(AuthUser {
                claims: Claims {
                    sub: "api-key".into(),
                    username: "admin".into(),
                    role: Role::Admin,
                    exp: usize::MAX,
                    jti: None,
                },
            });
            return next.run(req).await;
        }
        AuthMode::Jwt => {}
    }

    let claims = match extract_claims(&st.auth, req.headers()) {
        Ok(c) => c,
        Err(resp) => return *resp,
    };

    // A valid signature only proves the token was issued once. Re-read the row
    // so disabling a user or demoting their role takes effect immediately
    // instead of at token expiry.
    let user = match st.auth.get_user_by_id(&claims.sub) {
        Ok(u) => u,
        Err(_) => return auth_err(StatusCode::UNAUTHORIZED, "user no longer exists"),
    };
    if user.disabled {
        return auth_err(StatusCode::FORBIDDEN, "account disabled");
    }

    req.extensions_mut().insert(AuthUser {
        claims: Claims {
            role: user.role,
            username: user.username,
            ..claims
        },
    });
    next.run(req).await
}

pub async fn require_member(req: Request, next: Next) -> Response {
    let user = match req.extensions().get::<AuthUser>() {
        Some(u) => u.clone(),
        None => return auth_err(StatusCode::UNAUTHORIZED, "login required"),
    };
    if user.claims.role != Role::Admin && user.claims.role != Role::Member {
        return auth_err(StatusCode::FORBIDDEN, "forbidden");
    }
    next.run(req).await
}

pub async fn require_admin(req: Request, next: Next) -> Response {
    let user = match req.extensions().get::<AuthUser>() {
        Some(u) => u.clone(),
        None => return auth_err(StatusCode::UNAUTHORIZED, "login required"),
    };
    if user.claims.role != Role::Admin {
        return auth_err(StatusCode::FORBIDDEN, "admin only");
    }
    next.run(req).await
}

/// Extract and verify session claims.
///
/// The error is boxed: `Response` is large, and this sits on the hot path of
/// every authenticated request.
pub fn extract_claims(svc: &AuthService, headers: &HeaderMap) -> Result<Claims, Box<Response>> {
    let token = bearer_token(headers).or_else(|| cookie_token(svc, headers));
    let token =
        token.ok_or_else(|| Box::new(auth_err(StatusCode::UNAUTHORIZED, "login required")))?;
    svc.verify_token(&token)
        .map_err(|_| Box::new(auth_err(StatusCode::UNAUTHORIZED, "invalid session")))
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn cookie_token(svc: &AuthService, headers: &HeaderMap) -> Option<String> {
    let cookie_hdr = headers.get(header::COOKIE)?.to_str().ok()?;
    let name = &svc.config().cookie_name;
    cookie_hdr.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&format!("{name}="))
            .map(|v| v.to_string())
    })
}

fn build_session_cookie(svc: &AuthService, token: &str) -> Cookie<'static> {
    let mut c = Cookie::build((svc.config().cookie_name.clone(), token.to_string()))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax);
    if svc.config().cookie_secure {
        c = c.secure(true);
    }
    if let Ok(max_age) = svc.config().jwt_ttl_secs.try_into() {
        c = c.max_age(time::Duration::seconds(max_age));
    }
    c.into()
}

fn user_public(u: &User) -> Value {
    json!({
        "id": u.id,
        "username": u.username,
        "role": u.role,
        "created_at": u.created_at,
        "disabled": u.disabled,
    })
}

fn user_public_from_claims(claims: &Claims) -> Value {
    json!({
        "id": claims.sub,
        "username": claims.username,
        "role": claims.role,
        "created_at": "",
        "disabled": false,
    })
}

pub fn auth_err(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({
            "ok": false,
            "error": message,
        })),
    )
        .into_response()
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}
