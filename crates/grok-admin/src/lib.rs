//! grok-admin — 管理员认证（G4-P1）。
//!
//! 移植 Go `internal/application/adminauth` + `internal/infra/security`：
//! - [`security::TokenService`]：HS256 管理员 JWT（签发 / 校验 / 过期）
//! - [`service::AdminAuthService`]：登录 / refresh 轮换 / 注销 / 改密 / Bootstrap
//! - [`repos`]：持久化抽象（测试注入内存 fake，SQL 实现后续由 grok-storage 提供）
//! - [`guard`]：HTTP Bearer 认证 guard 形态

pub mod domain;
pub mod error;
pub mod guard;
pub mod repos;
pub mod security;
pub mod service;

pub use domain::{Admin, Session};
pub use error::{AdminError, AdminResult};
pub use guard::{authenticate_bearer, bearer_token, AuthContext};
pub use repos::{AdminRepository, AdminSessionRepository, RateLimiter};
pub use security::{hash_password, hash_token, new_opaque_token, verify_password, TokenService};
pub use service::{AdminAuthService, Tokens};