//! Admin API 路由形态（G4-P2）。
//!
//! [`AdminRouter::handle`] 统一入口：先过 [`crate::guard::authenticate_bearer`]
//! （无 / 坏 token → 401），再按 method + path 分发到账号管理 handler。
//! 返回 [`AdminHttpResponse`]（status + JSON body），后续 HTTP 层直接序列化即可。
//!
//! 路由表（对齐 Go `transport/http/account/handler.go` 的账号管理子集）：
//! - `GET    /admin/accounts`                  列表（page/pageSize/provider/enabled/authStatus 查询）
//! - `GET    /admin/accounts/{id}`             详情（账号 + 额度窗口 + 模型状态）
//! - `PATCH  /admin/accounts/{id}`             更新（enabled/auth_status/priority/cooldownUntil）
//! - `DELETE /admin/accounts/{id}`             删除
//! - `GET    /admin/accounts/{id}/quota`       额度窗口列表
//! - `PUT    /admin/accounts/{id}/quota`       写回额度窗口（body: mode/remaining/total/...）
//! - `GET    /admin/accounts/{id}/model-states` 模型状态列表

use std::collections::HashMap;

use grok_domain::{AuthStatus, Provider};
use serde_json::json;

use crate::accounts::{
    AccountAdminService, AccountListFilter, QuotaWindowInput, UpdateAccountInput,
};
use crate::error::{AdminError, AdminResult};
use crate::guard::authenticate_bearer;
use crate::service::AdminAuthService;

/// 统一响应形态（status + JSON body）。
#[derive(Debug, Clone)]
pub struct AdminHttpResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

impl AdminHttpResponse {
    pub fn ok(value: serde_json::Value) -> Self {
        Self { status: 200, body: value }
    }
    pub fn created(value: serde_json::Value) -> Self {
        Self { status: 201, body: value }
    }
    pub fn unauthorized() -> Self {
        Self {
            status: 401,
            body: json!({ "error": "invalidSession", "message": "管理员会话无效" }),
        }
    }
    pub fn not_found(message: &str) -> Self {
        Self {
            status: 404,
            body: json!({ "error": "notFound", "message": message }),
        }
    }
    pub fn bad_request(code: &str, message: &str) -> Self {
        Self {
            status: 400,
            body: json!({ "error": code, "message": message }),
        }
    }
    pub fn internal(message: &str) -> Self {
        Self {
            status: 500,
            body: json!({ "error": "internal", "message": message }),
        }
    }
}

/// Admin API 路由器：guard + 账号管理 handler 组合。
pub struct AdminRouter {
    auth: AdminAuthService,
    accounts: AccountAdminService,
}

impl AdminRouter {
    pub fn new(auth: AdminAuthService, accounts: AccountAdminService) -> Self {
        Self { auth, accounts }
    }

    /// 统一入口：guard → 路由。
    pub async fn handle(
        &self,
        method: &str,
        path: &str,
        authorization_header: Option<&str>,
        body: Option<&str>,
    ) -> AdminHttpResponse {
        let header = authorization_header.unwrap_or("");
        if authenticate_bearer(&self.auth, header).await.is_err() {
            return AdminHttpResponse::unauthorized();
        }
        self.route(method, path, body).await
    }

    async fn route(&self, method: &str, path: &str, body: Option<&str>) -> AdminHttpResponse {
        let (path_only, query) = split_query(path);
        let segments: Vec<&str> = path_only
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        match segments.as_slice() {
            ["admin", "accounts"] if method.eq_ignore_ascii_case("GET") => {
                self.list(query).await
            }
            ["admin", "accounts", id] => match method.to_ascii_uppercase().as_str() {
                "GET" => self.get(id).await,
                "PATCH" => self.patch(id, body).await,
                "DELETE" => self.delete(id).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },
            ["admin", "accounts", id, "quota"] if method.eq_ignore_ascii_case("GET") => {
                self.quota_list(id).await
            }
            ["admin", "accounts", id, "quota"] if method.eq_ignore_ascii_case("PUT") => {
                self.quota_put(id, body).await
            }
            ["admin", "accounts", id, "model-states"] if method.eq_ignore_ascii_case("GET") => {
                self.model_states(id).await
            }
            _ => AdminHttpResponse::bad_request("routeNotFound", "未知路由"),
        }
    }

    // ── handler 组（request → Result<Response, AdminError>）────────────────

    async fn list(&self, query: &str) -> AdminHttpResponse {
        let params = parse_query(query);
        let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
        let page_size = params
            .get("pageSize")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(20);
        let provider = match params.get("provider").map(|s| s.as_str()) {
            None | Some("") => None,
            Some("grok_build") => Some(Provider::GrokBuild),
            Some("grok_web") => Some(Provider::GrokWeb),
            Some("grok_console") => Some(Provider::GrokConsole),
            Some(other) => {
                return AdminHttpResponse::bad_request(
                    "invalidFilter",
                    &format!("无效 provider: {other}"),
                )
            }
        };
        let enabled = match params.get("enabled").map(|s| s.as_str()) {
            None | Some("") => None,
            Some("true") | Some("1") => Some(true),
            Some("false") | Some("0") => Some(false),
            Some(other) => {
                return AdminHttpResponse::bad_request(
                    "invalidFilter",
                    &format!("无效 enabled: {other}"),
                )
            }
        };
        let auth_status = match params.get("authStatus").map(|s| s.as_str()) {
            None | Some("") => None,
            Some(raw) => match crate::accounts::parse_auth_status(raw) {
                Ok(status) => Some(status),
                Err(e) => return map_error(e),
            },
        };
        let result = self
            .accounts
            .list(
                AccountListFilter { provider, enabled, auth_status },
                page,
                page_size,
            )
            .await;
        match result {
            Ok(page) => AdminHttpResponse::ok(json!({
                "items": page.items,
                "page": page.page,
                "pageSize": page.page_size,
                "total": page.total,
            })),
            Err(e) => map_error(e),
        }
    }

    async fn get(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        match self.accounts.get(id).await {
            Ok(detail) => AdminHttpResponse::ok(json!(detail)),
            Err(e) => map_error(e),
        }
    }

    async fn patch(&self, id: &str, body: Option<&str>) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let input: UpdateAccountInput = match body {
            Some(raw) => match serde_json::from_str(raw) {
                Ok(input) => input,
                Err(_) => {
                    return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效")
                }
            },
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求体不能为空"),
        };
        match self.accounts.update(id, &input).await {
            Ok(view) => AdminHttpResponse::ok(json!(view)),
            Err(e) => map_error(e),
        }
    }

    async fn delete(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        match self.accounts.delete(id).await {
            Ok(()) => AdminHttpResponse::ok(json!({ "deleted": true })),
            Err(e) => map_error(e),
        }
    }

    async fn quota_list(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        match self.accounts.quota_windows(id).await {
            Ok(windows) => AdminHttpResponse::ok(json!({ "items": windows })),
            Err(e) => map_error(e),
        }
    }

    async fn quota_put(&self, id: &str, body: Option<&str>) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let input: QuotaWindowInput = match body {
            Some(raw) => match serde_json::from_str(raw) {
                Ok(input) => input,
                Err(_) => {
                    return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效")
                }
            },
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求体不能为空"),
        };
        match self.accounts.upsert_quota(id, &input).await {
            Ok(window) => AdminHttpResponse::created(json!(window)),
            Err(e) => map_error(e),
        }
    }

    async fn model_states(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        match self.accounts.model_states(id).await {
            Ok(states) => AdminHttpResponse::ok(json!({ "items": states })),
            Err(e) => map_error(e),
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────

fn split_query(path: &str) -> (&str, &str) {
    match path.split_once('?') {
        Some((p, q)) => (p, q),
        None => (path, ""),
    }
}

/// 极简 query 解析（`a=b&c=d`；无依赖，重复键取首个）。
fn parse_query(query: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        out.entry(key.to_string()).or_insert_with(|| value.to_string());
    }
    out
}

fn parse_id(raw: &str) -> Option<i64> {
    let id = raw.parse::<i64>().ok()?;
    if id <= 0 {
        return None;
    }
    Some(id)
}

fn map_error(err: AdminError) -> AdminHttpResponse {
    match err {
        AdminError::NotFound(message) => AdminHttpResponse::not_found(&message),
        AdminError::InvalidFilter(message) | AdminError::InvalidRequest(message) => {
            AdminHttpResponse::bad_request("invalidRequest", &message)
        }
        _ => AdminHttpResponse::internal(&err.to_string()),
    }
}

/// 过滤 parse 辅助（供 handler 复用；返回 None = 未指定）。
#[allow(dead_code)]
pub(crate) fn parse_auth_status_opt(raw: &str) -> AdminResult<Option<AuthStatus>> {
    if raw.is_empty() {
        return Ok(None);
    }
    crate::accounts::parse_auth_status(raw).map(Some)
}
