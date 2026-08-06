//! grok-admin — 管理员认证（G4-P1）。
//!
//! 移植 Go `internal/application/adminauth` + `internal/infra/security`：
//! - [`security::TokenService`]：HS256 管理员 JWT（签发 / 校验 / 过期）
//! - [`service::AdminAuthService`]：登录 / refresh 轮换 / 注销 / 改密 / Bootstrap
//! - [`repos`]：持久化抽象（测试注入内存 fake，SQL 实现后续由 grok-storage 提供）
//! - [`guard`]：HTTP Bearer 认证 guard 形态

pub mod accounts;
pub mod admin_router;
pub mod audits;
pub mod chrome_tickets;
pub mod client_keys;
pub mod dashboard;
pub mod domain;
pub mod error;
pub mod guard;
pub mod media;
pub mod models;
pub mod repos;
pub mod security;
pub mod service;
pub mod settings;
pub mod system;

pub use accounts::{
    AccountAdminService, AccountAnalytics, AccountDetail, AccountListFilter, AccountPage,
    AccountSummary, AccountView, AdminStore, ProviderSummary, QuotaWindowInput,
    UpdateAccountInput,
};
pub use admin_router::{AdminDomains, AdminHttpResponse, AdminRouter};
pub use audits::{AuditAdminService, AuditEntryView, AuditStore, AuditSummaryView};
pub use chrome_tickets::{ChromeTicketService, ChromeTicketStats, ChromeTicketStore, ChromeTicketView};
pub use client_keys::{ClientKeyAdminService, ClientKeyInput, ClientKeyStore, ClientKeyView};
pub use dashboard::{DashboardService, DashboardStore, DashboardView};
pub use domain::{Admin, Session};
pub use error::{AdminError, AdminResult};
pub use guard::{authenticate_bearer, bearer_token, AuthContext};
pub use media::{ImageTimelineEntry, MediaImageView, MediaService, MediaStatsView, MediaStore};
pub use models::{ModelAdminService, ModelBindingView, ModelRoute, ModelRouteInput, ModelStore};
pub use repos::{AdminRepository, AdminSessionRepository, RateLimiter};
pub use security::{hash_password, hash_token, new_opaque_token, verify_password, TokenService};
pub use service::{AdminAuthService, Tokens};
pub use settings::{SettingsInput, SettingsService, SettingsStore, SettingsView};
pub use system::{SystemService, SystemView};