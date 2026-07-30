use crate::state::AppState;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_extra::extract::cookie::CookieJar;
use std::sync::Arc;
use tnexus_auth::{Claims, Role};

pub struct AuthUser {
    pub claims: Claims,
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = (StatusCode, String);

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let cookie_name = state.config.cookie_name.clone();
        let auth = state.auth.clone();
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        async move {
            let jar = CookieJar::from_headers(&parts.headers);
            let token = jar
                .get(&cookie_name)
                .map(|c| c.value().to_string())
                .or_else(|| auth_header.and_then(|v| v.strip_prefix("Bearer ").map(str::to_string)))
                .ok_or((StatusCode::UNAUTHORIZED, "login required".into()))?;

            let claims = auth
                .verify_token(&token)
                .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid token".into()))?;

            Ok(AuthUser { claims })
        }
    }
}

pub struct AdminUser(pub AuthUser);

impl FromRequestParts<Arc<AppState>> for AdminUser {
    type Rejection = (StatusCode, String);

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            let user = AuthUser::from_request_parts(parts, state).await?;
            if user.claims.role != Role::Admin {
                return Err((StatusCode::FORBIDDEN, "admin only".into()));
            }
            Ok(AdminUser(user))
        }
    }
}
