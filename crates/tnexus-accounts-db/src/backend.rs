//! Unified account pool backend: shared sqlite (gptimage) or TNexus Postgres.

use crate::{AccountsDb, AccountsPg};
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;

#[derive(Clone)]
pub enum AccountsBackend {
    Sqlite(AccountsDb),
    Postgres(AccountsPg),
}

impl AccountsBackend {
    pub fn from_env(pool: Option<PgPool>) -> Result<Self> {
        match std::env::var("ACCOUNTS_BACKEND").as_deref() {
            Ok("postgres") => {
                let pool = pool.context("ACCOUNTS_BACKEND=postgres requires DATABASE_URL pool")?;
                Ok(Self::Postgres(AccountsPg::new(pool)))
            }
            _ => Ok(Self::Sqlite(AccountsDb::from_env()?)),
        }
    }

    pub fn list_account_values(&self) -> Result<Vec<Value>> {
        match self {
            Self::Sqlite(db) => db.list_account_values(),
            Self::Postgres(pg) => block_on_pg(pg.list_account_values()),
        }
    }

    pub fn upsert_account_value(&self, value: &Value) -> Result<()> {
        match self {
            Self::Sqlite(db) => db.upsert_account_value(value),
            Self::Postgres(pg) => block_on_pg(pg.upsert_account_value(value)),
        }
    }

    pub fn delete_by_access_token(&self, token: &str) -> Result<bool> {
        match self {
            Self::Sqlite(db) => db.delete_by_access_token(token),
            Self::Postgres(pg) => block_on_pg(pg.delete_by_access_token(token)),
        }
    }

    pub fn accounts_by_email(&self) -> Result<HashMap<String, Value>> {
        match self {
            Self::Sqlite(db) => db.accounts_by_email(),
            Self::Postgres(pg) => block_on_pg(pg.accounts_by_email()),
        }
    }

    pub fn touch_inflight(&self, email: &str, delta: i64) -> Result<()> {
        match self {
            Self::Sqlite(db) => db.touch_inflight(email, delta),
            Self::Postgres(pg) => block_on_pg(pg.touch_inflight(email, delta)),
        }
    }

    pub fn decrement_quota(&self, email: &str, amount: i64) -> Result<Option<(i64, i64)>> {
        match self {
            Self::Sqlite(db) => db.decrement_quota(email, amount),
            Self::Postgres(pg) => block_on_pg(pg.decrement_quota(email, amount)),
        }
    }

    pub fn reconcile_inflight_above(&self, ceiling: i64) -> Result<usize> {
        match self {
            Self::Sqlite(db) => db.reconcile_inflight_above(ceiling),
            Self::Postgres(pg) => block_on_pg(pg.reconcile_inflight_above(ceiling)),
        }
    }

    /// Underlying sqlite when backend is sqlite.
    pub fn sqlite_db(&self) -> Option<&AccountsDb> {
        match self {
            Self::Sqlite(db) => Some(db),
            Self::Postgres(_) => None,
        }
    }
}

fn block_on_pg<T, F>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}
