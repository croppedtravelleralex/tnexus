//! 只读仓储（G0）。
//!
//! 每个 trait 只提供 G0 需要的 SELECT 路径；写方法（insert/update/delete）
//! 全部留待对应 Phase，避免在本 Phase 引入未用写路径（39e / 39c L1）。

pub mod account;
pub mod accounts_ops;
pub mod credential;
pub mod quota;
pub mod routing;

// Re-export traits at repo root for convenience.
pub use account::{AccountRepository, PgAccountRepository};
pub use credential::{
    decrypt_primary, parse_credential_key, CredentialRepository, PgCredentialRepository,
    PgSsoTokenProvider,
};
pub use quota::{PgQuotaRepository, QuotaRepository};
