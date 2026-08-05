//! 账号只读仓储（grok_accounts）。
//!
//! 表列与 migrations/010_grok_core.sql `grok_accounts` 对齐：
//! id / identity_key / provider / enabled / auth_status / priority / observed_model。

use async_trait::async_trait;
use grok_domain::{Account, AuthStatus, Provider};
use sqlx::{postgres::PgRow, PgPool, Row};

use crate::StorageError;

/// 账号只读 repository（G0）。
#[async_trait]
pub trait AccountRepository {
    /// provider 下 enabled 的账号集合（按 priority 降序、id 升序，与 Go 排序一致）。
    async fn list_pool(
        &self,
        provider: Provider,
        enabled: bool,
    ) -> Result<Vec<Account>, StorageError>;

    /// 按主键读单个账号。
    async fn get(&self, account_id: i64) -> Result<Account, StorageError>;

    /// 按 identity_key 读单个账号（唯一）。
    async fn by_identity_key(&self, key: &str) -> Result<Account, StorageError>;
}

/// PG 账号只读实现。
pub struct PgAccountRepository {
    pool: PgPool,
}

impl PgAccountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn provider_from_str(s: &str) -> Result<Provider, StorageError> {
    match s {
        "grok_build" => Ok(Provider::GrokBuild),
        "grok_web" => Ok(Provider::GrokWeb),
        "grok_console" => Ok(Provider::GrokConsole),
        other => Err(StorageError::Decode(format!("unknown provider: {other}"))),
    }
}

fn auth_status_from_str(s: &str) -> Result<AuthStatus, StorageError> {
    match s {
        "unknown" => Ok(AuthStatus::Unknown),
        "active" => Ok(AuthStatus::Active),
        "restricted" => Ok(AuthStatus::Restricted),
        "banned" => Ok(AuthStatus::Banned),
        other => Err(StorageError::Decode(format!(
            "unknown auth_status: {other}"
        ))),
    }
}

fn map_row(row: &PgRow) -> Result<Account, StorageError> {
    let provider_str: String = row.try_get("provider")?;
    let status_str: String = row.try_get("auth_status")?;
    Ok(Account {
        id: row.try_get("id")?,
        identity_key: row.try_get("identity_key")?,
        provider: provider_from_str(&provider_str)?,
        enabled: row.try_get("enabled")?,
        auth_status: auth_status_from_str(&status_str)?,
        priority: row.try_get("priority")?,
        observed_model: row.try_get("observed_model")?,
        ..Default::default()
    })
}

const ACCOUNT_COLS: &str =
    "id, identity_key, provider, enabled, auth_status, priority, observed_model";
const ACCOUNT_ORDER: &str = "ORDER BY priority DESC, id ASC";

#[async_trait]
impl AccountRepository for PgAccountRepository {
    async fn list_pool(
        &self,
        provider: Provider,
        enabled: bool,
    ) -> Result<Vec<Account>, StorageError> {
        let sql = format!(
            "SELECT {ACCOUNT_COLS} FROM grok_accounts \
             WHERE provider = $1 AND enabled = $2 {ACCOUNT_ORDER}"
        );
        let rows = sqlx::query(&sql)
            .bind(provider.as_str())
            .bind(enabled)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(map_row).collect()
    }

    async fn get(&self, account_id: i64) -> Result<Account, StorageError> {
        let sql = format!("SELECT {ACCOUNT_COLS} FROM grok_accounts WHERE id = $1");
        let row = sqlx::query(&sql)
            .bind(account_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound(format!("account {account_id}")));
        };
        map_row(&row)
    }

    async fn by_identity_key(&self, key: &str) -> Result<Account, StorageError> {
        let sql = format!("SELECT {ACCOUNT_COLS} FROM grok_accounts WHERE identity_key = $1");
        let row = sqlx::query(&sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound(format!("identity_key {key}")));
        };
        map_row(&row)
    }
}
