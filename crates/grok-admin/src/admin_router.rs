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
use crate::audits::AuditAdminService;
use crate::chrome_tickets::ChromeTicketService;
use crate::client_keys::ClientKeyAdminService;
use crate::dashboard::DashboardService;
use crate::error::{AdminError, AdminResult};
use crate::guard::authenticate_bearer;
use crate::media::MediaService;
use crate::models::{ModelAdminService, ModelRouteInput};
use crate::service::AdminAuthService;
use crate::settings::{SettingsInput, SettingsService};
use crate::system::SystemService;

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
    domains: AdminDomains,
}

/// 非账号域服务集合（G4-A1 缺口域；`new` 默认全部未接线）。
#[derive(Default)]
pub struct AdminDomains {
    pub models: Option<ModelAdminService>,
    pub client_keys: Option<ClientKeyAdminService>,
    pub audits: Option<AuditAdminService>,
    pub dashboard: Option<DashboardService>,
    pub settings: Option<SettingsService>,
    pub chrome_tickets: Option<ChromeTicketService>,
    pub media: Option<MediaService>,
    pub system: Option<SystemService>,
}

impl AdminRouter {
    pub fn new(auth: AdminAuthService, accounts: AccountAdminService) -> Self {
        Self { auth, accounts, domains: AdminDomains::default() }
    }

    /// 挂载全部非账号域（G4-A1 补齐后的构造入口）。
    pub fn with_domains(mut self, domains: AdminDomains) -> Self {
        self.domains = domains;
        self
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
            // ── 账号域（新增：summary/analytics 必须在 {id} 通配之前）──
            ["admin", "accounts", "summary"] if method.eq_ignore_ascii_case("GET") => {
                self.accounts_summary().await
            }
            ["admin", "accounts", "analytics"] if method.eq_ignore_ascii_case("GET") => {
                self.accounts_analytics().await
            }
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
            ["admin", "accounts", id, "refresh-billing"]
                if method.eq_ignore_ascii_case("POST") =>
            {
                self.accounts_refresh(id, "billing").await
            }
            ["admin", "accounts", id, "refresh-quota"]
                if method.eq_ignore_ascii_case("POST") =>
            {
                self.accounts_refresh(id, "quota").await
            }
            ["admin", "accounts", id, "refresh-token"]
                if method.eq_ignore_ascii_case("POST") =>
            {
                self.accounts_refresh(id, "token").await
            }
            ["admin", "accounts", id, "reauth"] if method.eq_ignore_ascii_case("POST") => {
                self.accounts_refresh(id, "reauth").await
            }

            // ── 模型域 ──
            ["admin", "models", "accounts"] if method.eq_ignore_ascii_case("GET") => {
                self.models_bindings().await
            }
            ["admin", "models"] => match method.to_ascii_uppercase().as_str() {
                "GET" => self.models_list(query).await,
                "POST" => self.models_create(body).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },
            ["admin", "models", id] => match method.to_ascii_uppercase().as_str() {
                "PATCH" => self.models_update(id, body).await,
                "DELETE" => self.models_delete(id).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },

            // ── 客户端密钥域 ──
            ["admin", "client-keys"] => match method.to_ascii_uppercase().as_str() {
                "GET" => self.keys_list(query).await,
                "POST" => self.keys_create(body).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },
            ["admin", "client-keys", id] => match method.to_ascii_uppercase().as_str() {
                "PATCH" => self.keys_update(id, body).await,
                "DELETE" => self.keys_delete(id).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },

            // ── 审计域 ──
            ["admin", "request-audits", "summary"]
                if method.eq_ignore_ascii_case("GET") =>
            {
                self.audits_summary().await
            }
            ["admin", "request-audits"] if method.eq_ignore_ascii_case("GET") => {
                self.audits_list(query).await
            }

            // ── 仪表盘 / 设置 / 系统 ──
            ["admin", "dashboard"] if method.eq_ignore_ascii_case("GET") => {
                self.dashboard().await
            }
            ["admin", "settings"] => match method.to_ascii_uppercase().as_str() {
                "GET" => self.settings_get().await,
                "PUT" => self.settings_put(body).await,
                _ => AdminHttpResponse::bad_request("methodNotAllowed", "不支持的请求方法"),
            },
            ["admin", "system"] if method.eq_ignore_ascii_case("GET") => self.system().await,

            // ── Chrome 票据域 ──
            ["admin", "chrome-tickets", "stats"] if method.eq_ignore_ascii_case("GET") => {
                self.tickets_stats().await
            }
            ["admin", "chrome-tickets", "sweep"] if method.eq_ignore_ascii_case("POST") => {
                self.tickets_sweep().await
            }
            ["admin", "chrome-tickets"] if method.eq_ignore_ascii_case("GET") => {
                self.tickets_list().await
            }

            // ── 媒体 / 时间线域 ──
            ["admin", "media", "images", "stats"] if method.eq_ignore_ascii_case("GET") => {
                self.media_stats().await
            }
            ["admin", "media", "images"] if method.eq_ignore_ascii_case("GET") => {
                self.media_list(query).await
            }
            ["admin", "image-timeline"] if method.eq_ignore_ascii_case("GET") => {
                self.image_timeline(query).await
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

    // ── G4-A1 新域 handler ─────────────────────────────────────

    async fn accounts_summary(&self) -> AdminHttpResponse {
        match self.accounts.summary().await {
            Ok(summary) => AdminHttpResponse::ok(json!(summary)),
            Err(e) => map_error(e),
        }
    }

    async fn accounts_analytics(&self) -> AdminHttpResponse {
        match self.accounts.analytics().await {
            Ok(analytics) => AdminHttpResponse::ok(json!(analytics)),
            Err(e) => map_error(e),
        }
    }

    async fn accounts_refresh(&self, id: &str, kind: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let result = match kind {
            "billing" => self.accounts.refresh_billing(id).await,
            "quota" => self.accounts.refresh_quota(id).await,
            "token" => self.accounts.refresh_token(id).await,
            "reauth" => self.accounts.reauth(id).await,
            _ => unreachable!("路由已限定 kind"),
        };
        match result {
            Ok(()) => AdminHttpResponse::ok(json!({ "refreshed": kind })),
            Err(e) => map_error(e),
        }
    }

    async fn models_list(&self, query: &str) -> AdminHttpResponse {
        let (page, page_size) = page_params(query);
        let Some(service) = &self.domains.models else {
            return domain_not_wired("models");
        };
        match service.list(page, page_size).await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items, "page": page, "pageSize": page_size, "total": items.len() as i64 })),
            Err(e) => map_error(e),
        }
    }

    async fn models_create(&self, body: Option<&str>) -> AdminHttpResponse {
        let Some(service) = &self.domains.models else {
            return domain_not_wired("models");
        };
        let input: ModelRouteInput = match parse_json(body) {
            Some(input) => input,
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效"),
        };
        match service.create(&input).await {
            Ok(route) => AdminHttpResponse::created(json!(route)),
            Err(e) => map_error(e),
        }
    }

    async fn models_update(&self, id: &str, body: Option<&str>) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let Some(service) = &self.domains.models else {
            return domain_not_wired("models");
        };
        let input: ModelRouteInput = match parse_json(body) {
            Some(input) => input,
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效"),
        };
        match service.update(id, &input).await {
            Ok(route) => AdminHttpResponse::ok(json!(route)),
            Err(e) => map_error(e),
        }
    }

    async fn models_delete(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let Some(service) = &self.domains.models else {
            return domain_not_wired("models");
        };
        match service.delete(id).await {
            Ok(()) => AdminHttpResponse::ok(json!({ "deleted": true })),
            Err(e) => map_error(e),
        }
    }

    async fn models_bindings(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.models else {
            return domain_not_wired("models");
        };
        match service.bindings().await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items })),
            Err(e) => map_error(e),
        }
    }

    async fn keys_list(&self, query: &str) -> AdminHttpResponse {
        let (page, page_size) = page_params(query);
        let Some(service) = &self.domains.client_keys else {
            return domain_not_wired("client-keys");
        };
        match service.list(page, page_size).await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items, "page": page, "pageSize": page_size, "total": items.len() as i64 })),
            Err(e) => map_error(e),
        }
    }

    async fn keys_create(&self, body: Option<&str>) -> AdminHttpResponse {
        let Some(service) = &self.domains.client_keys else {
            return domain_not_wired("client-keys");
        };
        let input: crate::client_keys::ClientKeyInput = match parse_json(body) {
            Some(input) => input,
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效"),
        };
        match service.create(&input).await {
            Ok((view, secret)) => AdminHttpResponse::created(json!({ "key": view, "secret": secret })),
            Err(e) => map_error(e),
        }
    }

    async fn keys_update(&self, id: &str, body: Option<&str>) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let Some(service) = &self.domains.client_keys else {
            return domain_not_wired("client-keys");
        };
        let input: crate::client_keys::ClientKeyInput = match parse_json(body) {
            Some(input) => input,
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效"),
        };
        match service.update(id, &input).await {
            Ok(view) => AdminHttpResponse::ok(json!(view)),
            Err(e) => map_error(e),
        }
    }

    async fn keys_delete(&self, id: &str) -> AdminHttpResponse {
        let Some(id) = parse_id(id) else {
            return AdminHttpResponse::bad_request("invalidId", "ID 无效");
        };
        let Some(service) = &self.domains.client_keys else {
            return domain_not_wired("client-keys");
        };
        match service.delete(id).await {
            Ok(()) => AdminHttpResponse::ok(json!({ "deleted": true })),
            Err(e) => map_error(e),
        }
    }

    async fn audits_list(&self, query: &str) -> AdminHttpResponse {
        let (page, page_size) = page_params(query);
        let Some(service) = &self.domains.audits else {
            return domain_not_wired("request-audits");
        };
        match service.list(page, page_size).await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items, "page": page, "pageSize": page_size, "total": items.len() as i64 })),
            Err(e) => map_error(e),
        }
    }

    async fn audits_summary(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.audits else {
            return domain_not_wired("request-audits");
        };
        match service.summary().await {
            Ok(summary) => AdminHttpResponse::ok(json!(summary)),
            Err(e) => map_error(e),
        }
    }

    async fn dashboard(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.dashboard else {
            return domain_not_wired("dashboard");
        };
        match service.view().await {
            Ok(view) => AdminHttpResponse::ok(json!(view)),
            Err(e) => map_error(e),
        }
    }

    async fn settings_get(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.settings else {
            return domain_not_wired("settings");
        };
        match service.get().await {
            Ok(view) => AdminHttpResponse::ok(json!(view)),
            Err(e) => map_error(e),
        }
    }

    async fn settings_put(&self, body: Option<&str>) -> AdminHttpResponse {
        let Some(service) = &self.domains.settings else {
            return domain_not_wired("settings");
        };
        let input: SettingsInput = match parse_json(body) {
            Some(input) => input,
            None => return AdminHttpResponse::bad_request("invalidRequest", "请求参数无效"),
        };
        match service.put(&input).await {
            Ok(view) => AdminHttpResponse::ok(json!(view)),
            Err(e) => map_error(e),
        }
    }

    async fn tickets_list(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.chrome_tickets else {
            return domain_not_wired("chrome-tickets");
        };
        match service.list().await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items })),
            Err(e) => map_error(e),
        }
    }

    async fn tickets_stats(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.chrome_tickets else {
            return domain_not_wired("chrome-tickets");
        };
        match service.stats().await {
            Ok(stats) => AdminHttpResponse::ok(json!(stats)),
            Err(e) => map_error(e),
        }
    }

    async fn tickets_sweep(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.chrome_tickets else {
            return domain_not_wired("chrome-tickets");
        };
        match service.sweep().await {
            Ok(swept) => AdminHttpResponse::ok(json!({ "swept": swept })),
            Err(e) => map_error(e),
        }
    }

    async fn media_list(&self, query: &str) -> AdminHttpResponse {
        let (page, page_size) = page_params(query);
        let Some(service) = &self.domains.media else {
            return domain_not_wired("media");
        };
        match service.list_images(page, page_size).await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items, "page": page, "pageSize": page_size, "total": items.len() as i64 })),
            Err(e) => map_error(e),
        }
    }

    async fn media_stats(&self) -> AdminHttpResponse {
        let Some(service) = &self.domains.media else {
            return domain_not_wired("media");
        };
        match service.media_stats().await {
            Ok(stats) => AdminHttpResponse::ok(json!(stats)),
            Err(e) => map_error(e),
        }
    }

    async fn image_timeline(&self, query: &str) -> AdminHttpResponse {
        let limit = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("limit="))
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(50);
        let Some(service) = &self.domains.media else {
            return domain_not_wired("media");
        };
        match service.timeline(limit).await {
            Ok(items) => AdminHttpResponse::ok(json!({ "items": items })),
            Err(e) => map_error(e),
        }
    }

    async fn system(&self) -> AdminHttpResponse {
        let view = match &self.domains.system {
            Some(service) => service.view(),
            None => crate::system::SystemService::new().view(),
        };
        AdminHttpResponse::ok(json!(view))
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

/// 分页参数（对齐 list 的默认 page=1 / pageSize=20）。
fn page_params(query: &str) -> (i64, i64) {
    let params = parse_query(query);
    let page = params.get("page").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1);
    let page_size = params
        .get("pageSize")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20);
    (page.max(1), page_size.clamp(1, 100))
}

/// 解析 JSON body（空/坏 → None）。
fn parse_json<T: serde::de::DeserializeOwned>(body: Option<&str>) -> Option<T> {
    match body {
        Some(raw) => serde_json::from_str(raw).ok(),
        None => None,
    }
}

/// 域未接线（`AdminRouter::new` 未挂 domains）→ 503。
fn domain_not_wired(domain: &str) -> AdminHttpResponse {
    AdminHttpResponse {
        status: 503,
        body: json!({ "error": "domainNotWired", "message": format!("{domain} 域未接线") }),
    }
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
