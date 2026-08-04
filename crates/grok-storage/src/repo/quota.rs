//! 额度只读仓储（grok_quota_windows）。
//!
//! G0 最小骨架：仅提供按账号读取额度窗口（G1 fast 额度验收的前置）。
//! `mode`（fast/auto/imagine）拆分、配额扣减写路径留待 G1+。

use async_trait::async_trait;
use grok_domain::QuotaWindow;
use sqlx::{PgPool, Row};

use crate::StorageError;

/// 额度只读 repository（G1+ 再补写路径）。
#[async_trait]
pub trait QuotaRepository {
    /// 账号全部额度窗口（fast/auto/imagine）。
    async fn get_windows(&self, account_id: i64) -> Result<Vec<QuotaWindow>, StorageError>;
}

/// PG 额度只读实现（G0 骨架）。
pub struct PgQuotaRepository {
    pool: PgPool,
}

impl PgQuotaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl QuotaRepository for PgQuotaRepository {
    async fn get_windows(&self, account_id: i64) -> Result<Vec<QuotaWindow>, StorageError> {
        let rows = sqlx::query(
            "SELECT account_id, remaining, reset_at \
             FROM grok_quota_windows WHERE account_id = $1",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(QuotaWindow {
                account_id: row.try_get("account_id")?,
                remaining: row.try_get("remaining")?,
                reset_at: row.try_get("reset_at")?,
            });
        }
        Ok(out)
    }
}
