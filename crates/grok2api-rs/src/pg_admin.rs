//! `/admin/*` PG 数据面（管理台真实号池 + 管理员/会话持久化）。
//!
//! - [`PgAdminStore`]：`AdminStore` for PG——账号列表/详情/更新/删除、额度窗口、
//!   模型状态、池汇总/分析、导入、时间序列/Top 账号（直接 SQL，表见
//!   `migrations/010_grok_core.sql` + `011` + `013`）
//! - [`PgAdminRepo`] / [`PgSessionRepo`]：`AdminRepository` + `AdminSessionRepository`
//!   读写 `grok_admins` / `grok_admin_sessions`
//!
//! 降级：无 DB（`GROK_DATABASE_URL` 未配置）→ 主流程保持内存实现
//! （`admin.rs` 的 `build_admin_bundle`）；配置后走 [`build_admin_bundle_pg`]。
//!
//! 注：`grok-storage` 的行映射辅助均为 `pub(crate)`，本 crate 无法复用，故此处
//! 内联最小映射（provider / auth_status / quota_source / model_status 字符串 ↔ 枚举）。

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use grok_admin::{
    AccountAnalytics, AccountListFilter, AccountPage, AccountSummary, AccountView, Admin,
    AdminError, AdminRepository, AdminResult, AdminSessionRepository, AdminStore,
    ImportAccountInput, ImportError, ImportResult, Session, TimeseriesPoint, TopAccountView,
    UpdateAccountInput,
};
use grok_domain::{Account, AuthStatus, ModelState, ModelStatus, Provider, QuotaSource, QuotaWindow};
use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::admin::{build_bundle, AdminHttpBundle};

// ── 字符串 ↔ 枚举 最小映射（grok-storage 的为 pub(crate)，无法跨 crate 复用）──

fn auth_status_to_db_str(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::ReauthRequired => "reauthRequired",
        AuthStatus::Active => "active",
        AuthStatus::Restricted => "restricted",
        AuthStatus::Banned => "banned",
        AuthStatus::Unknown => "unknown",
    }
}

fn provider_from_str(s: &str) -> Provider {
    match s {
        "grok_build" => Provider::GrokBuild,
        "grok_console" => Provider::GrokConsole,
        _ => Provider::GrokWeb,
    }
}

fn auth_status_from_str(s: &str) -> AuthStatus {
    match s {
        "active" => AuthStatus::Active,
        "restricted" => AuthStatus::Restricted,
        "banned" => AuthStatus::Banned,
        "reauthRequired" | "reauth_required" => AuthStatus::ReauthRequired,
        _ => AuthStatus::Unknown,
    }
}

fn quota_source_from_str(s: &str) -> QuotaSource {
    match s {
        "estimated" => QuotaSource::Estimated,
        "upstream" => QuotaSource::Upstream,
        _ => QuotaSource::Default,
    }
}

fn model_status_from_str(s: &str) -> ModelStatus {
    match s {
        "quota_available" => ModelStatus::QuotaAvailable,
        "available" => ModelStatus::Available,
        "soft_stop" => ModelStatus::SoftStop,
        "quota_exhausted" => ModelStatus::QuotaExhausted,
        "auth_failed" => ModelStatus::AuthFailed,
        "signature_failed" => ModelStatus::SignatureFailed,
        _ => ModelStatus::Unknown,
    }
}

fn admin_err(e: impl std::fmt::Display) -> AdminError {
    AdminError::RuntimeUnavailable(e.to_string())
}

const ACCOUNT_COLS: &str = "id, identity_key, provider, enabled, auth_status, priority, \
     observed_model, name, email, user_id, team_id, source_key, observed_model_at, \
     max_concurrent, minimum_remaining, failure_count, cooldown_until, last_error, \
     last_used_at, created_at, updated_at";

fn map_account_row(row: &sqlx::postgres::PgRow) -> AdminResult<Account> {
    Ok(Account {
        id: row.try_get("id").map_err(admin_err)?,
        identity_key: row.try_get("identity_key").map_err(admin_err)?,
        provider: provider_from_str(&row.try_get::<String, _>("provider").map_err(admin_err)?),
        enabled: row.try_get("enabled").map_err(admin_err)?,
        auth_status: auth_status_from_str(
            &row.try_get::<String, _>("auth_status").map_err(admin_err)?,
        ),
        priority: row.try_get("priority").map_err(admin_err)?,
        observed_model: row.try_get("observed_model").map_err(admin_err)?,
        name: row.try_get("name").map_err(admin_err)?,
        email: row.try_get("email").map_err(admin_err)?,
        user_id: row.try_get("user_id").map_err(admin_err)?,
        team_id: row.try_get("team_id").map_err(admin_err)?,
        source_key: row.try_get("source_key").map_err(admin_err)?,
        observed_model_at: row.try_get("observed_model_at").map_err(admin_err)?,
        max_concurrent: row.try_get("max_concurrent").map_err(admin_err)?,
        minimum_remaining: row.try_get("minimum_remaining").map_err(admin_err)?,
        failure_count: row.try_get("failure_count").map_err(admin_err)?,
        cooldown_until: row.try_get("cooldown_until").map_err(admin_err)?,
        last_error: row.try_get("last_error").map_err(admin_err)?,
        last_used_at: row.try_get("last_used_at").map_err(admin_err)?,
        created_at: row.try_get("created_at").map_err(admin_err)?,
        updated_at: row.try_get("updated_at").map_err(admin_err)?,
        ..Default::default()
    })
}

/// PG 账号数据面（`AdminStore`）。
pub struct PgAdminStore {
    pool: PgPool,
}

impl PgAdminStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn account_exists(&self, id: i64) -> AdminResult<bool> {
        let row = sqlx::query("SELECT EXISTS(SELECT 1 FROM grok_accounts WHERE id = $1) AS ok")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(admin_err)?;
        row.try_get("ok").map_err(admin_err)
    }

    async fn fetch_account_by_id(&self, id: i64) -> AdminResult<Option<Account>> {
        let sql = format!("SELECT {ACCOUNT_COLS} FROM grok_accounts WHERE id = $1");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        row.as_ref().map(map_account_row).transpose()
    }
}

#[async_trait]
impl AdminStore for PgAdminStore {
    async fn list_accounts(
        &self,
        filter: &AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<AccountPage> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 200);
        let provider = filter.provider.map(|p| p.as_str());
        let auth_status = filter.auth_status.map(auth_status_to_db_str);
        let sql = format!(
            "SELECT {ACCOUNT_COLS} FROM grok_accounts \
             WHERE ($1::text IS NULL OR provider = $1) \
               AND ($2::bool IS NULL OR enabled = $2) \
               AND ($3::text IS NULL OR auth_status = $3) \
             ORDER BY id ASC LIMIT $4 OFFSET $5"
        );
        let rows = sqlx::query(&sql)
            .bind(provider)
            .bind(filter.enabled)
            .bind(auth_status)
            .bind(page_size)
            .bind((page - 1) * page_size)
            .fetch_all(&self.pool)
            .await
            .map_err(admin_err)?;
        let items = rows
            .iter()
            .map(map_account_row)
            .collect::<AdminResult<Vec<_>>>()?;

        let count_sql = "SELECT count(*) AS total FROM grok_accounts \
             WHERE ($1::text IS NULL OR provider = $1) \
               AND ($2::bool IS NULL OR enabled = $2) \
               AND ($3::text IS NULL OR auth_status = $3)";
        let row = sqlx::query(count_sql)
            .bind(filter.provider.map(|p| p.as_str()))
            .bind(filter.enabled)
            .bind(filter.auth_status.map(auth_status_to_db_str))
            .fetch_one(&self.pool)
            .await
            .map_err(admin_err)?;
        let total: i64 = row.try_get("total").map_err(admin_err)?;

        Ok(AccountPage {
            items: items.into_iter().map(|a| AccountView::from(&a)).collect(),
            page,
            page_size,
            total,
        })
    }

    async fn get_account(&self, id: i64) -> AdminResult<Option<Account>> {
        self.fetch_account_by_id(id).await
    }

    async fn update_account(
        &self,
        id: i64,
        input: &UpdateAccountInput,
    ) -> AdminResult<Option<Account>> {
        let auth_status = input.auth_status.as_deref().map(|s| {
            let lower = s.to_ascii_lowercase();
            match lower.as_str() {
                "reauth_required" | "reauthrequired" => "reauthRequired".to_string(),
                other => other.to_string(),
            }
        });
        let sql = format!(
            "UPDATE grok_accounts SET \
               enabled = COALESCE($2, enabled), \
               auth_status = COALESCE($3, auth_status), \
               priority = COALESCE($4, priority), \
               cooldown_until = COALESCE($5, cooldown_until), \
               updated_at = now() \
             WHERE id = $1 RETURNING {ACCOUNT_COLS}"
        );
        let row = sqlx::query(&sql)
            .bind(id)
            .bind(input.enabled)
            .bind(auth_status)
            .bind(input.priority)
            .bind(input.cooldown_until)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        row.as_ref().map(map_account_row).transpose()
    }

    async fn delete_account(&self, id: i64) -> AdminResult<bool> {
        let row = sqlx::query("DELETE FROM grok_accounts WHERE id = $1 RETURNING id")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        Ok(row.is_some())
    }

    async fn list_quota_windows(&self, account_id: i64) -> AdminResult<Vec<QuotaWindow>> {
        let rows = sqlx::query(
            "SELECT account_id, mode, remaining, total, reset_at, synced_at, source, updated_at \
             FROM grok_quota_windows WHERE account_id = $1 ORDER BY mode ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(QuotaWindow {
                    account_id: row.try_get("account_id").map_err(admin_err)?,
                    mode: row.try_get("mode").map_err(admin_err)?,
                    remaining: row.try_get("remaining").map_err(admin_err)?,
                    total: row.try_get("total").map_err(admin_err)?,
                    reset_at: row.try_get("reset_at").map_err(admin_err)?,
                    synced_at: row.try_get("synced_at").map_err(admin_err)?,
                    source: quota_source_from_str(
                        &row.try_get::<String, _>("source").map_err(admin_err)?,
                    ),
                    updated_at: row.try_get("updated_at").map_err(admin_err)?,
                })
            })
            .collect()
    }

    async fn upsert_quota_window(&self, window: QuotaWindow) -> AdminResult<QuotaWindow> {
        let source = window.source.as_str();
        let row = sqlx::query(
            "INSERT INTO grok_quota_windows \
             (account_id, mode, remaining, total, reset_at, synced_at, source, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,now()) \
             ON CONFLICT (account_id, mode) DO UPDATE SET \
               remaining = EXCLUDED.remaining, total = EXCLUDED.total, \
               reset_at = EXCLUDED.reset_at, synced_at = EXCLUDED.synced_at, \
               source = EXCLUDED.source, updated_at = now() \
             RETURNING account_id, mode, remaining, total, reset_at, synced_at, source, updated_at",
        )
        .bind(window.account_id)
        .bind(window.mode)
        .bind(window.remaining)
        .bind(window.total)
        .bind(window.reset_at)
        .bind(window.synced_at)
        .bind(source)
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        Ok(QuotaWindow {
            account_id: row.try_get("account_id").map_err(admin_err)?,
            mode: row.try_get("mode").map_err(admin_err)?,
            remaining: row.try_get("remaining").map_err(admin_err)?,
            total: row.try_get("total").map_err(admin_err)?,
            reset_at: row.try_get("reset_at").map_err(admin_err)?,
            synced_at: row.try_get("synced_at").map_err(admin_err)?,
            source: quota_source_from_str(&row.try_get::<String, _>("source").map_err(admin_err)?),
            updated_at: row.try_get("updated_at").map_err(admin_err)?,
        })
    }

    async fn list_model_states(&self, account_id: i64) -> AdminResult<Vec<ModelState>> {
        let rows = sqlx::query(
            "SELECT account_id, upstream_model, status, reason, consecutive_failures, \
                    last_attempt_at, last_success_at, cooldown_until, updated_at \
             FROM grok_model_states WHERE account_id = $1 ORDER BY upstream_model ASC",
        )
        .bind(account_id)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(ModelState {
                    account_id: row.try_get("account_id").map_err(admin_err)?,
                    upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
                    status: model_status_from_str(
                        &row.try_get::<String, _>("status").map_err(admin_err)?,
                    ),
                    reason: row.try_get("reason").map_err(admin_err)?,
                    consecutive_failures: row.try_get("consecutive_failures").map_err(admin_err)?,
                    last_attempt_at: row.try_get("last_attempt_at").map_err(admin_err)?,
                    last_success_at: row.try_get("last_success_at").map_err(admin_err)?,
                    cooldown_until: row.try_get("cooldown_until").map_err(admin_err)?,
                    updated_at: row.try_get("updated_at").map_err(admin_err)?,
                })
            })
            .collect()
    }

    async fn pool_summary(&self) -> AdminResult<AccountSummary> {
        let rows = sqlx::query(
            "SELECT provider, enabled, auth_status, cooldown_until, failure_count \
             FROM grok_accounts",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        let now = Utc::now();
        let mut summary = AccountSummary::default();
        for row in &rows {
            let provider = row.try_get::<String, _>("provider").map_err(admin_err)?;
            let enabled: bool = row.try_get("enabled").map_err(admin_err)?;
            let status = row
                .try_get::<String, _>("auth_status")
                .map_err(admin_err)?
                .to_ascii_lowercase();
            let cooldown_until: Option<DateTime<Utc>> =
                row.try_get("cooldown_until").map_err(admin_err)?;
            let failure_count: i32 = row.try_get("failure_count").map_err(admin_err)?;

            summary.total += 1;
            let p = summary.by_provider.entry(provider).or_default();
            p.total += 1;

            let cooling = cooldown_until.is_some_and(|c| c > now);
            let reauth = status == "reauthrequired" || status == "reauth_required";
            if reauth {
                summary.reauth_required += 1;
                p.reauth_required += 1;
            } else if !enabled {
                summary.disabled += 1;
                p.disabled += 1;
            } else if cooling {
                summary.cooldown += 1;
                p.cooldown += 1;
            } else if failure_count > 0 {
                summary.probing += 1;
            } else {
                summary.available += 1;
                p.available += 1;
            }
        }
        // 额度已耗尽：remaining<=0 且 total>0 的窗口（按账号去重）。
        let exhausted = sqlx::query(
            "SELECT count(DISTINCT account_id) AS n FROM grok_quota_windows \
             WHERE remaining <= 0 AND total > 0",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        summary.quota_exhausted = exhausted.try_get("n").map_err(admin_err)?;
        Ok(summary)
    }

    async fn analytics(&self) -> AdminResult<AccountAnalytics> {
        let rows = sqlx::query(
            "SELECT a.id AS account_id, a.observed_model, w.remaining, w.total, \
                    (b.account_id IS NOT NULL) AS has_billing \
             FROM grok_accounts a \
             LEFT JOIN grok_quota_windows w ON w.account_id = a.id AND w.mode = 'imagine' \
             LEFT JOIN grok_billing_snapshots b ON b.account_id = a.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        let mut out = AccountAnalytics::default();
        for row in &rows {
            let remaining: i64 = row.try_get("remaining").map_err(admin_err)?;
            let total: i64 = row.try_get("total").map_err(admin_err)?;
            if total > 0 && remaining <= 0 {
                out.quota_exhausted += 1;
            } else if total == 0 && remaining == 0 {
                out.quota_unknown += 1;
            } else {
                out.quota_known += 1;
            }
            let has_billing: bool = row.try_get("has_billing").map_err(admin_err)?;
            if has_billing {
                out.billing_count += 1;
            }
            let model: Option<String> = row.try_get("observed_model").map_err(admin_err)?;
            if let Some(m) = model {
                if !m.trim().is_empty() {
                    *out.by_model.entry(m).or_default() += 1;
                }
            }
        }
        Ok(out)
    }

    async fn refresh_billing(&self, account_id: i64) -> AdminResult<bool> {
        let exists = self.account_exists(account_id).await?;
        if exists {
            tracing::warn!(
                "admin refresh-billing 未接上游 sidecar（TODO），仅确认账号存在: {account_id}"
            );
        }
        Ok(exists)
    }

    async fn refresh_quota(&self, account_id: i64) -> AdminResult<bool> {
        let exists = self.account_exists(account_id).await?;
        if exists {
            tracing::warn!(
                "admin refresh-quota 未接上游 sidecar（TODO），仅确认账号存在: {account_id}"
            );
        }
        Ok(exists)
    }

    async fn refresh_token(&self, account_id: i64) -> AdminResult<bool> {
        let exists = self.account_exists(account_id).await?;
        if exists {
            tracing::warn!(
                "admin refresh-token 未接上游 sidecar（TODO），仅确认账号存在: {account_id}"
            );
        }
        Ok(exists)
    }

    async fn reauth(&self, account_id: i64) -> AdminResult<bool> {
        let row = sqlx::query(
            "UPDATE grok_accounts SET auth_status = 'reauthRequired', updated_at = now() \
             WHERE id = $1 RETURNING id",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        Ok(row.is_some())
    }

    async fn import_accounts(&self, inputs: &[ImportAccountInput]) -> AdminResult<ImportResult> {
        let mut result = ImportResult::default();
        for (index, input) in inputs.iter().enumerate() {
            let provider = match input.provider.trim() {
                "grok_build" | "grok_web" | "grok_console" => input.provider.trim(),
                other => {
                    result.failed += 1;
                    result.errors.push(ImportError {
                        index,
                        reason: format!("provider 无效: {other}"),
                    });
                    continue;
                }
            };
            let name = input.name.as_deref().unwrap_or(&input.identity_key);
            let priority = input.priority.unwrap_or(1);
            let max_concurrent = input.max_concurrent.unwrap_or(8);
            let row = sqlx::query(
                "INSERT INTO grok_accounts \
                 (identity_key, provider, name, source_key, priority, max_concurrent, enabled) \
                 VALUES ($1,$2,$3,$1,$4,$5,true) \
                 ON CONFLICT (identity_key) DO NOTHING RETURNING id",
            )
            .bind(&input.identity_key)
            .bind(provider)
            .bind(name)
            .bind(priority)
            .bind(max_concurrent)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
            if row.is_some() {
                result.imported += 1;
            } else {
                result.failed += 1;
                result.errors.push(ImportError {
                    index,
                    reason: "identity_key 已存在".into(),
                });
            }
        }
        Ok(result)
    }

    async fn timeseries(&self, days: i64) -> AdminResult<Vec<TimeseriesPoint>> {
        let days = days.clamp(1, 90);
        let rows = sqlx::query(
            "SELECT date_trunc('day', created_at)::date AS d, \
                    count(*) AS requests, \
                    count(*) FILTER (WHERE status_code BETWEEN 200 AND 299) AS succeeded, \
                    count(*) FILTER (WHERE status_code NOT BETWEEN 200 AND 299) AS failed \
             FROM grok_request_audits \
             WHERE created_at >= now() - ($1::int || ' days')::interval \
             GROUP BY d ORDER BY d ASC",
        )
        .bind(days)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                let date: chrono::NaiveDate = row.try_get("d").map_err(admin_err)?;
                Ok(TimeseriesPoint {
                    date: date.to_string(),
                    requests: row.try_get("requests").map_err(admin_err)?,
                    succeeded: row.try_get("succeeded").map_err(admin_err)?,
                    failed: row.try_get("failed").map_err(admin_err)?,
                    latency_p50_ms: 0, // grok_request_audits 无耗时列；留 0 待 G6 加列
                })
            })
            .collect()
    }

    async fn top_accounts(&self, limit: i64) -> AdminResult<Vec<TopAccountView>> {
        let limit = limit.clamp(1, 100);
        let rows = sqlx::query(
            "SELECT account_id, max(account_name) AS name, \
                    count(*) AS requests, \
                    count(*) FILTER (WHERE status_code NOT BETWEEN 200 AND 299) AS failed \
             FROM grok_request_audits \
             WHERE account_id IS NOT NULL \
             GROUP BY account_id ORDER BY requests DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                let requests: i64 = row.try_get("requests").map_err(admin_err)?;
                let failed: i64 = row.try_get("failed").map_err(admin_err)?;
                Ok(TopAccountView {
                    account_id: row.try_get("account_id").map_err(admin_err)?,
                    name: row.try_get("name").map_err(admin_err)?,
                    requests,
                    failed,
                    failure_rate: if requests > 0 {
                        failed as f64 / requests as f64
                    } else {
                        0.0
                    },
                })
            })
            .collect()
    }
}

// ── 管理员 / 会话 PG 仓储 ─────────────────────────────────────────

/// PG 管理员仓储（`grok_admins`）。
pub struct PgAdminRepo {
    pool: PgPool,
}

impl PgAdminRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminRepository for PgAdminRepo {
    async fn count(&self) -> AdminResult<i64> {
        let row = sqlx::query("SELECT count(*) AS n FROM grok_admins")
            .fetch_one(&self.pool)
            .await
            .map_err(admin_err)?;
        row.try_get("n").map_err(admin_err)
    }

    async fn create(&self, admin: Admin) -> AdminResult<Admin> {
        let row = sqlx::query(
            "INSERT INTO grok_admins (username, password_hash) VALUES ($1, $2) \
             RETURNING id, username, password_hash, created_at, updated_at",
        )
        .bind(&admin.username)
        .bind(&admin.password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        map_admin_row(&row)
    }

    async fn get_by_username(&self, username: &str) -> AdminResult<Option<Admin>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, created_at, updated_at \
             FROM grok_admins WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        row.as_ref().map(map_admin_row).transpose()
    }

    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Admin>> {
        let row = sqlx::query(
            "SELECT id, username, password_hash, created_at, updated_at \
             FROM grok_admins WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        row.as_ref().map(map_admin_row).transpose()
    }

    async fn update_password_and_revoke_sessions(
        &self,
        admin_id: i64,
        password_hash: &str,
    ) -> AdminResult<()> {
        sqlx::query(
            "UPDATE grok_admins SET password_hash = $2, updated_at = now() WHERE id = $1",
        )
        .bind(admin_id)
        .bind(password_hash)
        .execute(&self.pool)
        .await
        .map_err(admin_err)?;
        sqlx::query("DELETE FROM grok_admin_sessions WHERE admin_id = $1")
            .bind(admin_id)
            .execute(&self.pool)
            .await
            .map_err(admin_err)?;
        Ok(())
    }
}

fn map_admin_row(row: &sqlx::postgres::PgRow) -> AdminResult<Admin> {
    Ok(Admin {
        id: row.try_get("id").map_err(admin_err)?,
        username: row.try_get("username").map_err(admin_err)?,
        password_hash: row.try_get("password_hash").map_err(admin_err)?,
        created_at: row.try_get("created_at").map_err(admin_err)?,
        updated_at: row.try_get("updated_at").map_err(admin_err)?,
    })
}

/// PG 会话仓储（`grok_admin_sessions`）。
pub struct PgSessionRepo {
    pool: PgPool,
}

impl PgSessionRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AdminSessionRepository for PgSessionRepo {
    async fn get_by_token_hash(&self, token_hash: &str) -> AdminResult<Option<Session>> {
        let row = sqlx::query(
            "SELECT id, admin_id, refresh_token_hash, expires_at, last_used_at, created_at \
             FROM grok_admin_sessions WHERE refresh_token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        row.as_ref().map(map_session_row).transpose()
    }

    async fn get_by_id(&self, id: i64) -> AdminResult<Option<Session>> {
        let row = sqlx::query(
            "SELECT id, admin_id, refresh_token_hash, expires_at, last_used_at, created_at \
             FROM grok_admin_sessions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        row.as_ref().map(map_session_row).transpose()
    }

    async fn create(&self, session: Session) -> AdminResult<Session> {
        let row = sqlx::query(
            "INSERT INTO grok_admin_sessions (admin_id, refresh_token_hash, expires_at) \
             VALUES ($1, $2, $3) \
             RETURNING id, admin_id, refresh_token_hash, expires_at, last_used_at, created_at",
        )
        .bind(session.admin_id)
        .bind(&session.refresh_token_hash)
        .bind(session.expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        map_session_row(&row)
    }

    async fn rotate(
        &self,
        session_id: i64,
        old_token_hash: &str,
        new_token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> AdminResult<bool> {
        let row = sqlx::query(
            "UPDATE grok_admin_sessions \
             SET refresh_token_hash = $3, expires_at = $4, last_used_at = now() \
             WHERE id = $1 AND refresh_token_hash = $2 RETURNING id",
        )
        .bind(session_id)
        .bind(old_token_hash)
        .bind(new_token_hash)
        .bind(expires_at)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        Ok(row.is_some())
    }

    async fn revoke(&self, id: i64) -> AdminResult<()> {
        sqlx::query("DELETE FROM grok_admin_sessions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(admin_err)?;
        Ok(())
    }
}

fn map_session_row(row: &sqlx::postgres::PgRow) -> AdminResult<Session> {
    Ok(Session {
        id: row.try_get("id").map_err(admin_err)?,
        admin_id: row.try_get("admin_id").map_err(admin_err)?,
        refresh_token_hash: row.try_get("refresh_token_hash").map_err(admin_err)?,
        expires_at: row.try_get("expires_at").map_err(admin_err)?,
        last_used_at: row.try_get("last_used_at").map_err(admin_err)?,
        created_at: row.try_get("created_at").map_err(admin_err)?,
    })
}

/// 构造带 PG 数据面的 admin bundle（`GROK_DATABASE_URL` 已配置时使用）。
///
/// 复用 [`build_bundle`] 的鉴权/路由组装，仅替换 store 与 admin/session 仓储。
pub async fn build_admin_bundle_pg(
    pool: PgPool,
    username: &str,
    password: Option<&str>,
    secret: &str,
) -> AdminHttpBundle {
    let repo = Arc::new(PgAdminRepo::new(pool.clone()));
    let sessions = Arc::new(PgSessionRepo::new(pool.clone()));
    let store: Arc<dyn AdminStore> = Arc::new(PgAdminStore::new(pool));
    build_bundle(repo, sessions, store, username, password, secret).await
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 无 DB 连接也能组装 PG bundle：connect_lazy 不建连，bootstrap 失败仅告警不 panic。
    #[tokio::test]
    async fn pg_bundle_constructs_without_db_connection() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy(
                "postgres://nobody:secret@127.0.0.1:1/grok?connect_timeout=1",
            )
            .expect("lazy pool never connects");
        // password=None → 跳过 bootstrap（不做 DB 查询），仅验证 PG store/repo 组装。
        let bundle = build_admin_bundle_pg(pool, "admin", None, "test-secret").await;
        let _ = bundle;
    }
}
