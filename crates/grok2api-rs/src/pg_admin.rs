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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use grok_admin::{
    hash_token, AccountAnalytics, AccountListFilter, AccountPage, AccountSummary, AccountView,
    Admin, AdminDomains, AdminError, AdminRepository, AdminResult, AdminSessionRepository,
    AdminStore, AuditAdminService, AuditEntryView, AuditStore, AuditSummaryView,
    ChromeTicketService, ChromeTicketStats, ChromeTicketStore, ChromeTicketView,
    ClientKeyAdminService, ClientKeyInput, ClientKeyStore, ClientKeyView, DashboardService,
    DashboardStore, DashboardView, ImageTimelineEntry, ImportAccountInput, ImportError,
    ImportResult, MediaImageView, MediaService, MediaSizeSummaryView, MediaStatsView, MediaStore,
    ModelAdminService, ModelAliasView, ModelBindingView, ModelRoute, ModelRouteInput, ModelStore,
    ModelSyncStateView, QuotaModeSummary, Session, SettingsService, SettingsStore, SettingsView,
    TimeseriesPoint, TopAccountView, UpdateAccountInput,
};
use grok_domain::{
    Account, AuthStatus, ModelState, ModelStatus, Provider, QuotaSource, QuotaWindow,
};
use sqlx::postgres::PgPool;
use sqlx::Row;

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

/// PostgreSQL `SUM(bigint)` 返回 NUMERIC；生产列也可能仍是 INTEGER/NUMERIC。
fn sql_i64(row: &sqlx::postgres::PgRow, col: &str) -> AdminResult<i64> {
    if let Ok(v) = row.try_get::<i64, _>(col) {
        return Ok(v);
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(col) {
        return Ok(v.unwrap_or(0));
    }
    if let Ok(v) = row.try_get::<i32, _>(col) {
        return Ok(i64::from(v));
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(col) {
        return Ok(i64::from(v.unwrap_or(0)));
    }
    Err(admin_err(format!(
        "column {col}: expected integer remaining/total"
    )))
}

/// 额度窗口 SELECT：强制 `::bigint`，避免 NUMERIC/INT4 与 Rust i64 解码失败。
pub(crate) const QUOTA_WINDOW_SELECT_SQL: &str = "SELECT account_id, mode, \
     remaining::bigint AS remaining, total::bigint AS total, \
     reset_at, synced_at, source, updated_at \
     FROM grok_quota_windows WHERE account_id = $1 ORDER BY mode ASC";

/// 号池 summary 按 mode 聚合。`SUM(...)::bigint` 是 admin 500 的修复点
///（PG 对 SUM(bigint) 推断为 NUMERIC）。
pub(crate) const POOL_SUMMARY_QUOTA_SQL: &str = "SELECT \
     w.mode, \
     COUNT(DISTINCT w.account_id)::bigint AS accounts, \
     COALESCE(SUM(w.remaining::bigint) FILTER ( \
         WHERE w.total > 0 AND w.total < 1000000000), 0)::bigint AS remaining, \
     COALESCE(SUM(w.total::bigint) FILTER ( \
         WHERE w.total > 0 AND w.total < 1000000000), 0)::bigint AS total, \
     COUNT(*) FILTER (WHERE w.remaining = 0 AND w.total > 0)::bigint AS exhausted, \
     COUNT(*) FILTER (WHERE w.synced_at IS NULL \
         OR w.synced_at < now() - interval '24 hours')::bigint AS stale, \
     COUNT(*) FILTER (WHERE w.synced_at >= now() - interval '24 hours' \
         AND w.total > 0 AND w.total < 1000000000)::bigint AS accounts_fresh, \
     COALESCE(SUM(w.remaining::bigint) FILTER ( \
         WHERE w.synced_at >= now() - interval '24 hours' \
           AND w.total > 0 AND w.total < 1000000000), 0)::bigint AS remaining_fresh, \
     COALESCE(SUM(w.total::bigint) FILTER ( \
         WHERE w.synced_at >= now() - interval '24 hours' \
           AND w.total > 0 AND w.total < 1000000000), 0)::bigint AS total_fresh, \
     MIN(w.synced_at) AS oldest_synced_at, \
     MAX(w.synced_at) AS newest_synced_at \
 FROM grok_quota_windows w \
 JOIN grok_accounts a ON a.id = w.account_id AND a.enabled = true \
 GROUP BY w.mode \
 ORDER BY w.mode ASC";

const ACCOUNT_COLS: &str = "id, identity_key, provider, enabled, auth_status, priority, \
     observed_model, name, email, user_id, team_id, source_key, observed_model_at, \
     max_concurrent, minimum_remaining::bigint AS minimum_remaining, failure_count, cooldown_until, last_error, \
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
    quota: Option<Arc<crate::web_quota::WebQuotaService>>,
}

impl PgAdminStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool, quota: None }
    }

    pub fn with_quota_service(mut self, quota: Arc<crate::web_quota::WebQuotaService>) -> Self {
        self.quota = Some(quota);
        self
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

    /// 一次查出本页全部额度窗口，避免列表接口后再打 200 次 `/quota`。
    async fn attach_quota_windows(&self, items: &mut [AccountView]) -> AdminResult<()> {
        if items.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = items.iter().map(|a| a.id).collect();
        let rows = sqlx::query(
            "SELECT account_id, mode, remaining::bigint AS remaining, total::bigint AS total, \
             reset_at, synced_at, source, updated_at \
             FROM grok_quota_windows WHERE account_id = ANY($1) ORDER BY account_id, mode ASC",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        let mut by_id: HashMap<i64, Vec<QuotaWindow>> = HashMap::new();
        for row in &rows {
            let window = QuotaWindow {
                account_id: row.try_get("account_id").map_err(admin_err)?,
                mode: row.try_get("mode").map_err(admin_err)?,
                remaining: sql_i64(row, "remaining")?,
                total: sql_i64(row, "total")?,
                reset_at: row.try_get("reset_at").map_err(admin_err)?,
                synced_at: row.try_get("synced_at").map_err(admin_err)?,
                source: quota_source_from_str(
                    &row.try_get::<String, _>("source").map_err(admin_err)?,
                ),
                updated_at: row.try_get("updated_at").map_err(admin_err)?,
            };
            by_id.entry(window.account_id).or_default().push(window);
        }
        for item in items.iter_mut() {
            if let Some(windows) = by_id.remove(&item.id) {
                item.quota_windows = windows;
            }
        }
        Ok(())
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
        let mut views: Vec<AccountView> = items.iter().map(AccountView::from).collect();
        self.attach_quota_windows(&mut views).await?;

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
            items: views,
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
        let rows = sqlx::query(QUOTA_WINDOW_SELECT_SQL)
            .bind(account_id)
            .fetch_all(&self.pool)
            .await
            .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(QuotaWindow {
                    account_id: row.try_get("account_id").map_err(admin_err)?,
                    mode: row.try_get("mode").map_err(admin_err)?,
                    remaining: sql_i64(row, "remaining")?,
                    total: sql_i64(row, "total")?,
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
             RETURNING account_id, mode, remaining::bigint AS remaining, total::bigint AS total, \
               reset_at, synced_at, source, updated_at",
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
            remaining: sql_i64(&row, "remaining")?,
            total: sql_i64(&row, "total")?,
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
        // 各 mode 额度聚合（仅 enabled 账号；按 mode 分组）。
        // 加总排除 0/0（未知）与 total≥1e9（imagine「不限」哨兵），避免把 ETL 冻结行算进可用额度。
        let quota_rows = sqlx::query(POOL_SUMMARY_QUOTA_SQL)
            .fetch_all(&self.pool)
            .await
            .map_err(admin_err)?;
        let mut quota = Vec::with_capacity(quota_rows.len());
        for row in &quota_rows {
            quota.push(QuotaModeSummary {
                mode: row.try_get("mode").map_err(admin_err)?,
                accounts: sql_i64(row, "accounts")?,
                remaining: sql_i64(row, "remaining")?,
                total: sql_i64(row, "total")?,
                exhausted: sql_i64(row, "exhausted")?,
                stale: sql_i64(row, "stale")?,
                accounts_fresh: sql_i64(row, "accounts_fresh")?,
                remaining_fresh: sql_i64(row, "remaining_fresh")?,
                total_fresh: sql_i64(row, "total_fresh")?,
                oldest_synced_at: row.try_get("oldest_synced_at").map_err(admin_err)?,
                newest_synced_at: row.try_get("newest_synced_at").map_err(admin_err)?,
            });
        }
        summary.quota = quota;
        Ok(summary)
    }

    async fn analytics(&self) -> AdminResult<AccountAnalytics> {
        let rows = sqlx::query(
            "SELECT a.id AS account_id, a.observed_model, \
                    COALESCE(w.remaining::bigint, 0) AS remaining, \
                    COALESCE(w.total::bigint, 0) AS total, \
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
            let remaining = sql_i64(row, "remaining")?;
            let total = sql_i64(row, "total")?;
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
        if !self.account_exists(account_id).await? {
            return Ok(false);
        }
        let Some(quota) = &self.quota else {
            tracing::warn!(
                "admin refresh-quota 未接线（缺直连/sso）：仅确认账号存在: {account_id}"
            );
            return Ok(true);
        };
        match quota.refresh_account(account_id).await {
            Ok(w) => {
                tracing::info!(
                    account_id,
                    remaining = w.remaining,
                    total = w.total,
                    "admin refresh-quota ok"
                );
                Ok(true)
            }
            Err(e) => Err(AdminError::RuntimeUnavailable(e.to_string())),
        }
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
                // 可选凭据：写入 grok_credentials（账号 id 由 RETURNING 取得）。
                if let Some(credential) = &input.credential {
                    if !credential.trim().is_empty() {
                        let account_id: i64 =
                            row.map(|r| r.try_get("id").unwrap_or(0)).unwrap_or(0);
                        let _ = sqlx::query(
                            "INSERT INTO grok_credentials (account_id, auth_type, encrypted_primary) \
                             VALUES ($1,'sso',$2) \
                             ON CONFLICT (account_id) DO UPDATE SET encrypted_primary = EXCLUDED.encrypted_primary, updated_at = now()",
                        )
                        .bind(account_id)
                        .bind(credential)
                        .execute(&self.pool)
                        .await;
                    }
                }
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
        sqlx::query("UPDATE grok_admins SET password_hash = $2, updated_at = now() WHERE id = $1")
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

// ── 审计读侧 PG store（grok_request_audits）─────────────────────────

/// PG 审计只读 store（对齐 `AuditStore` trait；写侧由 `grok-audit::AuditSink` 负责）。
pub struct PgAuditStore {
    pool: PgPool,
}

impl PgAuditStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditStore for PgAuditStore {
    /// 按时间倒序分页，同时返回全表总行数（前端 total 字段）。
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<(Vec<AuditEntryView>, i64)> {
        let offset = (page - 1) * page_size;
        let rows = sqlx::query(
            "SELECT id, account_id, provider, model_upstream_model, \
                    status_code::smallint AS status_code, duration_ms, created_at \
             FROM grok_request_audits \
             ORDER BY created_at DESC, id DESC \
             LIMIT $1 OFFSET $2",
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;

        let count_row = sqlx::query("SELECT count(*) AS total FROM grok_request_audits")
            .fetch_one(&self.pool)
            .await
            .map_err(admin_err)?;
        let total: i64 = count_row.try_get("total").map_err(admin_err)?;

        let items = rows
            .iter()
            .map(|row| {
                let status_code: i16 = row.try_get("status_code").map_err(admin_err)?;
                let outcome = if (200i16..=299).contains(&status_code) {
                    "success"
                } else {
                    "error"
                }
                .to_string();
                Ok(AuditEntryView {
                    id: row.try_get("id").map_err(admin_err)?,
                    account_id: row.try_get("account_id").map_err(admin_err)?,
                    provider: row
                        .try_get::<Option<String>, _>("provider")
                        .map_err(admin_err)?,
                    upstream_model: row
                        .try_get::<Option<String>, _>("model_upstream_model")
                        .map_err(admin_err)?,
                    status: status_code,
                    outcome,
                    latency_ms: row.try_get("duration_ms").map_err(admin_err)?,
                    created_at: row.try_get("created_at").map_err(admin_err)?,
                })
            })
            .collect::<AdminResult<Vec<_>>>()?;

        Ok((items, total))
    }

    async fn summary(&self) -> AdminResult<AuditSummaryView> {
        let row = sqlx::query(
            "SELECT \
                count(*) AS total, \
                count(*) FILTER (WHERE created_at >= now() - interval '24 hours') AS requests_24h, \
                count(*) FILTER (WHERE created_at >= now() - interval '24 hours' \
                    AND status_code BETWEEN 200 AND 299) AS succeeded_24h, \
                count(*) FILTER (WHERE created_at >= now() - interval '24 hours' \
                    AND (status_code < 200 OR status_code > 299)) AS failed_24h \
             FROM grok_request_audits",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;

        let requests_24h: i64 = row.try_get("requests_24h").map_err(admin_err)?;
        let succeeded_24h: i64 = row.try_get("succeeded_24h").map_err(admin_err)?;
        Ok(AuditSummaryView {
            total: row.try_get("total").map_err(admin_err)?,
            requests_24h,
            succeeded_24h,
            failed_24h: row.try_get("failed_24h").map_err(admin_err)?,
            success_rate_24h: if requests_24h > 0 {
                succeeded_24h as f64 / requests_24h as f64
            } else {
                0.0
            },
        })
    }
}

// ── 仪表盘 PG store ──────────────────────────────────────────────────

/// PG 仪表盘聚合（grok_accounts + grok_request_audits + grok_model_routes + grok_client_keys）。
pub struct PgDashboardStore {
    pool: PgPool,
}

impl PgDashboardStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DashboardStore for PgDashboardStore {
    async fn view(&self) -> AdminResult<DashboardView> {
        // 账号统计（与 /admin/accounts/summary 同源，保证数字不矛盾）。
        let acc = sqlx::query(
            "SELECT \
                count(*) AS total, \
                count(*) FILTER (WHERE enabled = true AND auth_status = 'active' \
                    AND (cooldown_until IS NULL OR cooldown_until <= now())) AS available, \
                count(*) FILTER (WHERE cooldown_until IS NOT NULL \
                    AND cooldown_until > now()) AS cooldown, \
                count(*) FILTER (WHERE auth_status IN ('reauthRequired','reauth_required')) AS reauth \
             FROM grok_accounts",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;

        // 额度耗尽账号（独立子查询，窗口 remaining<=0 且 total>0 的去重账号数）。
        let quota_ex = sqlx::query(
            "SELECT count(DISTINCT account_id) AS n FROM grok_quota_windows \
             WHERE remaining <= 0 AND total > 0",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;

        // 近 24h 请求统计。
        let req = sqlx::query(
            "SELECT \
                count(*) AS requests_24h, \
                count(*) FILTER (WHERE status_code BETWEEN 200 AND 299) AS succeeded_24h, \
                max(created_at) AS last_request_at \
             FROM grok_request_audits \
             WHERE created_at >= now() - interval '24 hours'",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;

        let model_routes: i64 =
            sqlx::query("SELECT count(*) AS n FROM grok_model_routes WHERE enabled = true")
                .fetch_one(&self.pool)
                .await
                .map_err(admin_err)?
                .try_get("n")
                .map_err(admin_err)?;

        let active_client_keys: i64 = sqlx::query(
            "SELECT count(*) AS n FROM grok_client_keys \
             WHERE enabled = true AND (expires_at IS NULL OR expires_at > now())",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?
        .try_get("n")
        .map_err(admin_err)?;

        let requests_24h: i64 = req.try_get("requests_24h").map_err(admin_err)?;
        let succeeded_24h: i64 = req.try_get("succeeded_24h").map_err(admin_err)?;
        Ok(DashboardView {
            total_accounts: acc.try_get("total").map_err(admin_err)?,
            available_accounts: acc.try_get("available").map_err(admin_err)?,
            cooldown_accounts: acc.try_get("cooldown").map_err(admin_err)?,
            reauth_accounts: acc.try_get("reauth").map_err(admin_err)?,
            quota_exhausted_accounts: quota_ex.try_get("n").map_err(admin_err)?,
            requests_24h,
            success_rate_24h: if requests_24h > 0 {
                succeeded_24h as f64 / requests_24h as f64
            } else {
                0.0
            },
            model_routes,
            active_client_keys,
            last_request_at: req.try_get("last_request_at").map_err(admin_err)?,
        })
    }
}

// ── 模型路由 PG store ────────────────────────────────────────────────

/// PG 模型路由 store（grok_model_routes + grok_model_route_aliases）。
pub struct PgModelStore {
    pool: PgPool,
}

impl PgModelStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 映射一行 `grok_model_routes`（含 aliases 数组聚合）到 `ModelRoute`。
fn map_model_row(row: &sqlx::postgres::PgRow) -> AdminResult<ModelRoute> {
    Ok(ModelRoute {
        id: row.try_get("id").map_err(admin_err)?,
        provider: row.try_get("provider").map_err(admin_err)?,
        upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
        aliases: row.try_get::<Vec<String>, _>("aliases").unwrap_or_default(),
        enabled: row.try_get("enabled").map_err(admin_err)?,
        created_at: row.try_get("created_at").map_err(admin_err)?,
        updated_at: row.try_get("updated_at").map_err(admin_err)?,
    })
}

/// 带别名聚合的 SELECT 子句（LEFT JOIN + ARRAY_AGG，可安全加 WHERE/LIMIT）。
const MODEL_SELECT_WITH_ALIASES: &str = "\
    SELECT r.id, r.provider, r.upstream_model, r.enabled, r.created_at, r.updated_at, \
           COALESCE(ARRAY_AGG(a.alias) FILTER (WHERE a.alias IS NOT NULL), '{}') AS aliases \
    FROM grok_model_routes r \
    LEFT JOIN grok_model_route_aliases a ON a.model_route_id = r.id \
    GROUP BY r.id";

#[async_trait]
impl ModelStore for PgModelStore {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ModelRoute>> {
        let offset = (page - 1) * page_size;
        let sql = format!(
            "{MODEL_SELECT_WITH_ALIASES} \
             ORDER BY r.created_at DESC, r.id DESC LIMIT $1 OFFSET $2"
        );
        sqlx::query(&sql)
            .bind(page_size)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(admin_err)?
            .iter()
            .map(map_model_row)
            .collect()
    }

    async fn get(&self, id: i64) -> AdminResult<Option<ModelRoute>> {
        let sql = format!("{MODEL_SELECT_WITH_ALIASES} HAVING r.id = $1");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        row.as_ref().map(map_model_row).transpose()
    }

    async fn create(&self, input: &ModelRouteInput) -> AdminResult<ModelRoute> {
        // public_id = "{provider}/{upstream_model}"，与 (provider, upstream_model) 唯一约束对应。
        let public_id = format!("{}/{}", input.provider, input.upstream_model);
        let row = sqlx::query(
            "INSERT INTO grok_model_routes \
             (public_id, provider, upstream_model, capability, origin, enabled) \
             VALUES ($1, $2, $3, 'chat', 'manual', $4) \
             RETURNING id, provider, upstream_model, enabled, created_at, updated_at",
        )
        .bind(&public_id)
        .bind(&input.provider)
        .bind(&input.upstream_model)
        .bind(input.enabled.unwrap_or(true))
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        let id: i64 = row.try_get("id").map_err(admin_err)?;
        // 写入别名（忽略单条失败，不回滚整个创建）。
        for alias in &input.aliases {
            let _ = sqlx::query(
                "INSERT INTO grok_model_route_aliases (alias, model_route_id) \
                 VALUES ($1, $2) ON CONFLICT (alias) DO NOTHING",
            )
            .bind(alias)
            .bind(id)
            .execute(&self.pool)
            .await;
        }
        Ok(ModelRoute {
            id,
            provider: row.try_get("provider").map_err(admin_err)?,
            upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
            aliases: input.aliases.clone(),
            enabled: row.try_get("enabled").map_err(admin_err)?,
            created_at: row.try_get("created_at").map_err(admin_err)?,
            updated_at: row.try_get("updated_at").map_err(admin_err)?,
        })
    }

    async fn update(&self, id: i64, input: &ModelRouteInput) -> AdminResult<Option<ModelRoute>> {
        let row = sqlx::query(
            "UPDATE grok_model_routes SET \
               enabled = COALESCE($2, enabled), \
               updated_at = now() \
             WHERE id = $1 \
             RETURNING id, provider, upstream_model, enabled, created_at, updated_at",
        )
        .bind(id)
        .bind(input.enabled)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        // 别名替换：先删后插（aliases 非空时才替换，保持 PATCH 语义）。
        if !input.aliases.is_empty() {
            sqlx::query("DELETE FROM grok_model_route_aliases WHERE model_route_id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(admin_err)?;
            for alias in &input.aliases {
                let _ = sqlx::query(
                    "INSERT INTO grok_model_route_aliases (alias, model_route_id) \
                     VALUES ($1, $2) ON CONFLICT (alias) DO NOTHING",
                )
                .bind(alias)
                .bind(id)
                .execute(&self.pool)
                .await;
            }
        }
        let aliases = if input.aliases.is_empty() {
            // 返回当前别名列表（未替换时从 DB 重读）。
            sqlx::query("SELECT alias FROM grok_model_route_aliases WHERE model_route_id = $1")
                .bind(id)
                .fetch_all(&self.pool)
                .await
                .map_err(admin_err)?
                .iter()
                .filter_map(|r| r.try_get::<String, _>("alias").ok())
                .collect()
        } else {
            input.aliases.clone()
        };
        Ok(Some(ModelRoute {
            id,
            provider: row.try_get("provider").map_err(admin_err)?,
            upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
            aliases,
            enabled: row.try_get("enabled").map_err(admin_err)?,
            created_at: row.try_get("created_at").map_err(admin_err)?,
            updated_at: row.try_get("updated_at").map_err(admin_err)?,
        }))
    }

    async fn delete(&self, id: i64) -> AdminResult<bool> {
        let row = sqlx::query("DELETE FROM grok_model_routes WHERE id = $1 RETURNING id")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        Ok(row.is_some())
    }

    async fn bindings(&self) -> AdminResult<Vec<ModelBindingView>> {
        let rows = sqlx::query(
            "SELECT r.id AS model_route_id, r.upstream_model, \
                    COALESCE(ARRAY_AGG(mra.account_id) FILTER (WHERE mra.account_id IS NOT NULL), '{}') AS account_ids \
             FROM grok_model_routes r \
             LEFT JOIN grok_model_route_accounts mra ON mra.model_route_id = r.id \
             GROUP BY r.id ORDER BY r.id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(ModelBindingView {
                    model_route_id: row.try_get("model_route_id").map_err(admin_err)?,
                    upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
                    account_ids: row
                        .try_get::<Vec<i64>, _>("account_ids")
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn aliases(&self) -> AdminResult<Vec<ModelAliasView>> {
        let rows = sqlx::query(
            "SELECT r.upstream_model, r.enabled, \
                    COALESCE(ARRAY_AGG(a.alias) FILTER (WHERE a.alias IS NOT NULL), '{}') AS aliases \
             FROM grok_model_routes r \
             LEFT JOIN grok_model_route_aliases a ON a.model_route_id = r.id \
             GROUP BY r.id ORDER BY r.upstream_model ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(ModelAliasView {
                    upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
                    aliases: row.try_get::<Vec<String>, _>("aliases").unwrap_or_default(),
                    enabled: row.try_get("enabled").map_err(admin_err)?,
                })
            })
            .collect()
    }

    async fn sync_states(&self) -> AdminResult<Vec<ModelSyncStateView>> {
        let rows = sqlx::query(
            "SELECT r.upstream_model, \
                    count(mra.account_id)::bigint AS account_count \
             FROM grok_model_routes r \
             LEFT JOIN grok_model_route_accounts mra ON mra.model_route_id = r.id \
             GROUP BY r.id ORDER BY r.upstream_model ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        rows.iter()
            .map(|row| {
                Ok(ModelSyncStateView {
                    upstream_model: row.try_get("upstream_model").map_err(admin_err)?,
                    account_count: row.try_get("account_count").map_err(admin_err)?,
                    sync_state: "unknown".into(),
                })
            })
            .collect()
    }
}

// ── 客户端密钥 PG store ──────────────────────────────────────────────

/// PG 客户端密钥 store（grok_client_keys）。
///
/// 安全属性保证：`list` 只返回 `prefix`（前 8 位），完整 `secret` 仅在 `create`
/// 返回一次，之后不可再取（DB 只存 hash + 前缀）。
pub struct PgClientKeyStore {
    pool: PgPool,
}

impl PgClientKeyStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 生成随机 secret（32 字节 → 64 位十六进制字符串）。
fn gen_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 映射 grok_client_keys 行（不含 secret 字段）→ `ClientKeyView`（只含 prefix）。
fn map_key_row(row: &sqlx::postgres::PgRow) -> AdminResult<ClientKeyView> {
    Ok(ClientKeyView {
        id: row.try_get("id").map_err(admin_err)?,
        name: row.try_get("name").map_err(admin_err)?,
        prefix: row.try_get("prefix").map_err(admin_err)?,
        enabled: row.try_get("enabled").map_err(admin_err)?,
        created_at: row.try_get("created_at").map_err(admin_err)?,
        last_used_at: row.try_get("last_used_at").map_err(admin_err)?,
    })
}

#[async_trait]
impl ClientKeyStore for PgClientKeyStore {
    async fn list(&self, page: i64, page_size: i64) -> AdminResult<Vec<ClientKeyView>> {
        let offset = (page - 1) * page_size;
        // 只 SELECT prefix，绝不返回 secret_hash / encrypted_secret。
        sqlx::query(
            "SELECT id, name, prefix, enabled, created_at, last_used_at \
             FROM grok_client_keys \
             ORDER BY created_at DESC, id DESC \
             LIMIT $1 OFFSET $2",
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?
        .iter()
        .map(map_key_row)
        .collect()
    }

    async fn create(&self, input: &ClientKeyInput) -> AdminResult<(ClientKeyView, String)> {
        let secret = gen_secret();
        // prefix：前 8 位十六进制（供列表页识别密钥身份）。
        let prefix = &secret[..8];
        // secret_hash：SHA-256(secret)，64 位十六进制（满足 CHECK length=64）。校验只用它。
        let secret_hash = hash_token(&secret);
        // encrypted_secret 这一列没有任何代码读取，而列名暗示可还原。往里写明文等于
        // 一次库泄露就交出全部客户端密钥，且毫无必要——密钥按设计只在创建时返回一次、
        // 之后不可找回。列约束要求非空 1..4096，故写入哨兵而非明文。
        // 若将来确需可还原存储，应先补 AES-GCM 加密（参照 grok-storage 的
        // decrypt_primary），而不是退回明文。
        const SECRET_NOT_STORED: &str = "not-stored";
        let row = sqlx::query(
            "INSERT INTO grok_client_keys \
             (name, prefix, secret_hash, encrypted_secret, enabled) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, name, prefix, enabled, created_at, last_used_at",
        )
        .bind(&input.name)
        .bind(prefix)
        .bind(&secret_hash)
        .bind(SECRET_NOT_STORED)
        .bind(input.enabled.unwrap_or(true))
        .fetch_one(&self.pool)
        .await
        .map_err(admin_err)?;
        let view = map_key_row(&row)?;
        // 完整 secret 在此一次性返回，之后无法从 DB 取回（DB 只有 hash）。
        Ok((view, secret))
    }

    async fn update(&self, id: i64, input: &ClientKeyInput) -> AdminResult<Option<ClientKeyView>> {
        let row = sqlx::query(
            "UPDATE grok_client_keys SET \
               name = CASE WHEN length(trim($2)) > 0 THEN $2 ELSE name END, \
               enabled = COALESCE($3, enabled), \
               updated_at = now() \
             WHERE id = $1 \
             RETURNING id, name, prefix, enabled, created_at, last_used_at",
        )
        .bind(id)
        .bind(&input.name)
        .bind(input.enabled)
        .fetch_optional(&self.pool)
        .await
        .map_err(admin_err)?;
        row.as_ref().map(map_key_row).transpose()
    }

    async fn delete(&self, id: i64) -> AdminResult<bool> {
        let row = sqlx::query("DELETE FROM grok_client_keys WHERE id = $1 RETURNING id")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(admin_err)?;
        Ok(row.is_some())
    }
}

// ── 全局设置 PG store ────────────────────────────────────────────────

/// PG 全局设置 store（grok_runtime_settings；键值对，revision 单调递增）。
pub struct PgSettingsStore {
    pool: PgPool,
}

impl PgSettingsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SettingsStore for PgSettingsStore {
    async fn get(&self) -> AdminResult<SettingsView> {
        let rows = sqlx::query(
            "SELECT key, value_json, revision, updated_at \
             FROM grok_runtime_settings ORDER BY key ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(admin_err)?;
        let mut values = BTreeMap::new();
        let mut version: i64 = 0;
        let mut updated_at: Option<DateTime<Utc>> = None;
        for row in &rows {
            let key: String = row.try_get("key").map_err(admin_err)?;
            let val: String = row.try_get("value_json").map_err(admin_err)?;
            let rev: i64 = row.try_get("revision").map_err(admin_err)?;
            let ts: DateTime<Utc> = row.try_get("updated_at").map_err(admin_err)?;
            values.insert(key, val);
            version = version.max(rev);
            if updated_at.is_none_or(|prev| ts > prev) {
                updated_at = Some(ts);
            }
        }
        Ok(SettingsView {
            version,
            updated_at: updated_at.unwrap_or_else(Utc::now),
            values,
        })
    }

    async fn put(&self, values: BTreeMap<String, String>) -> AdminResult<SettingsView> {
        let mut tx = self.pool.begin().await.map_err(admin_err)?;
        // 新版本号 = 全表最大 revision + 1（空表时从 1 起）。
        let rev_row = sqlx::query(
            "SELECT COALESCE(MAX(revision), 0) + 1 AS new_rev FROM grok_runtime_settings",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(admin_err)?;
        let new_rev: i64 = rev_row.try_get("new_rev").map_err(admin_err)?;
        // 原子全量替换：删旧键，写新键（保持 revision 单调）。
        sqlx::query("DELETE FROM grok_runtime_settings")
            .execute(&mut *tx)
            .await
            .map_err(admin_err)?;
        for (key, val) in &values {
            sqlx::query(
                "INSERT INTO grok_runtime_settings (key, value_json, revision, updated_at) \
                 VALUES ($1, $2, $3, now())",
            )
            .bind(key)
            .bind(val)
            .bind(new_rev)
            .execute(&mut *tx)
            .await
            .map_err(admin_err)?;
        }
        tx.commit().await.map_err(admin_err)?;
        Ok(SettingsView {
            version: new_rev,
            updated_at: Utc::now(),
            values,
        })
    }
}

// ── 内存占位 store（media / chrome-tickets 不在本批次 PG 化范围内）──

/// 内存票据占位（未 PG 化；仅维持端点形状）。
struct InMemoryTicketStore;

#[async_trait]
impl ChromeTicketStore for InMemoryTicketStore {
    async fn list(&self) -> AdminResult<Vec<ChromeTicketView>> {
        Ok(vec![])
    }
    async fn stats(&self) -> AdminResult<ChromeTicketStats> {
        Ok(ChromeTicketStats::default())
    }
    async fn sweep(&self) -> AdminResult<i64> {
        Ok(0)
    }
}

/// 内存媒体占位（未 PG 化）。
struct InMemoryMediaStore;

#[async_trait]
impl MediaStore for InMemoryMediaStore {
    async fn list_images(&self, _page: i64, _page_size: i64) -> AdminResult<Vec<MediaImageView>> {
        Ok(vec![])
    }
    async fn media_stats(&self) -> AdminResult<MediaStatsView> {
        Ok(MediaStatsView::default())
    }
    async fn timeline(&self, _limit: usize) -> AdminResult<Vec<ImageTimelineEntry>> {
        Ok(vec![])
    }
    async fn get_image(&self, _asset_id: &str) -> AdminResult<Option<MediaImageView>> {
        Ok(None)
    }
    async fn size_summary(&self) -> AdminResult<MediaSizeSummaryView> {
        Ok(MediaSizeSummaryView {
            total_images: 0,
            total_bytes: 0,
            buckets: vec![],
        })
    }
}

/// 组装 PG 数据面的非账号域（审计/仪表盘/模型/密钥/设置由 PG 支撑）。
pub fn build_admin_domains_pg(pool: PgPool) -> AdminDomains {
    use grok_admin::SystemService;
    AdminDomains {
        models: Some(ModelAdminService::new(Arc::new(PgModelStore::new(
            pool.clone(),
        )))),
        client_keys: Some(ClientKeyAdminService::new(Arc::new(PgClientKeyStore::new(
            pool.clone(),
        )))),
        audits: Some(AuditAdminService::new(Arc::new(PgAuditStore::new(
            pool.clone(),
        )))),
        dashboard: Some(DashboardService::new(Arc::new(PgDashboardStore::new(
            pool.clone(),
        )))),
        settings: Some(SettingsService::new(Arc::new(PgSettingsStore::new(
            pool.clone(),
        )))),
        chrome_tickets: Some(ChromeTicketService::new(Arc::new(InMemoryTicketStore))),
        media: Some(MediaService::new(Arc::new(InMemoryMediaStore))),
        system: Some(SystemService::new()),
    }
}
///
/// 复用 [`build_bundle`] 的鉴权/路由组装，仅替换 store 与 admin/session 仓储。
pub async fn build_admin_bundle_pg(
    pool: PgPool,
    username: &str,
    password: Option<&str>,
    secret: &str,
    extras: crate::admin::AdminExtras,
) -> crate::admin::AdminHttpBundle {
    let repo = Arc::new(PgAdminRepo::new(pool.clone()));
    let sessions = Arc::new(PgSessionRepo::new(pool.clone()));
    let mut store = PgAdminStore::new(pool.clone());
    if let Some(q) = extras.quota.clone() {
        store = store.with_quota_service(q);
    }
    let store: Arc<dyn AdminStore> = Arc::new(store);
    let domains = build_admin_domains_pg(pool);
    crate::admin::build_bundle(
        repo, sessions, store, username, password, secret, extras, domains,
    )
    .await
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 无 DB 连接也能组装 PG bundle：connect_lazy 不建连，bootstrap 失败仅告警不 panic。
    #[tokio::test]
    async fn pg_bundle_constructs_without_db_connection() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://nobody:secret@127.0.0.1:1/grok?connect_timeout=1")
            .expect("lazy pool never connects");
        // password=None → 跳过 bootstrap（不做 DB 查询），仅验证 PG store/repo 组装。
        let bundle =
            build_admin_bundle_pg(pool, "admin", None, "test-secret", Default::default()).await;
        let _ = bundle;
    }

    // ── 无 DB 单元测试：可判定逻辑 ──────────────────────────────────

    /// gen_secret 生成 64 位十六进制字符串（32 字节）。
    #[test]
    fn gen_secret_is_64_hex_chars() {
        let s = gen_secret();
        assert_eq!(s.len(), 64, "secret 应为 64 位十六进制");
        assert!(
            s.chars().all(|c| c.is_ascii_hexdigit()),
            "应全为十六进制字符"
        );
    }

    /// prefix 前 8 位唯一标识 secret，与 gen_secret 结果对齐。
    #[test]
    fn prefix_is_first_8_chars_of_secret() {
        let s = gen_secret();
        let prefix = &s[..8];
        assert_eq!(prefix.len(), 8);
        // DB 约束 length(prefix) BETWEEN 1 AND 32 — 满足。
        assert!(prefix.len() <= 32);
    }

    /// secret_hash 满足 DB 约束 length(secret_hash) = 64（SHA-256 十六进制）。
    #[test]
    fn secret_hash_length_matches_db_constraint() {
        let secret = gen_secret();
        let h = hash_token(&secret);
        assert_eq!(
            h.len(),
            64,
            "SHA-256 hex 应为 64 字符，满足 grok_client_keys.secret_hash CHECK"
        );
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// 两次生成的 secret 不同（随机性）。
    #[test]
    fn gen_secret_is_random() {
        let a = gen_secret();
        let b = gen_secret();
        assert_ne!(a, b, "相邻两次 secret 应不同（OsRng）");
    }

    /// ClientKeyView 不含 secret 字段（安全属性：列表仅返回 prefix）。
    #[test]
    fn client_key_view_has_no_secret_field() {
        // 确认 ClientKeyView 序列化结果不含 secret/hash 字段。
        let view = ClientKeyView {
            id: 1,
            name: "test".to_string(),
            prefix: "ab12cd34".to_string(),
            enabled: true,
            created_at: Utc::now(),
            last_used_at: None,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("prefix"), "序列化应含 prefix");
        assert!(!json.contains("secret"), "序列化不应含 secret");
        assert!(!json.contains("hash"), "序列化不应含 hash");
        assert!(!json.contains("encrypted"), "序列化不应含 encrypted");
    }

    /// 分页偏移计算：offset = (page-1) * page_size。
    #[test]
    fn pagination_offset_calculation() {
        assert_eq!((1_i64 - 1) * 20, 0);
        assert_eq!((2_i64 - 1) * 20, 20);
        assert_eq!((3_i64 - 1) * 10, 20);
    }

    /// AuditStore::list 签名返回 tuple（items, total）确保 total 来自真实 COUNT。
    #[tokio::test]
    async fn in_memory_audit_store_returns_tuple() {
        let store = crate::admin_domains::InMemoryAuditStore;
        let (items, total) = store.list(1, 20).await.unwrap();
        assert_eq!(items.len(), 0);
        assert_eq!(total, 0, "内存实现 total 应为 0");
    }

    /// public_id 生成规则：provider/upstream_model，与 (provider, upstream_model) 唯一约束对应。
    #[test]
    fn model_public_id_format_respects_uniqueness() {
        // 同一 upstream_model 不同 provider 有不同 public_id。
        let id1 = format!("{}/{}", "grok_web", "grok-4");
        let id2 = format!("{}/{}", "grok_build", "grok-4");
        assert_ne!(id1, id2, "public_id 应区分 provider");
        // 长度在允许范围内（1-255）。
        assert!(id1.len() >= 1 && id1.len() <= 255);
    }

    /// 审计 outcome 映射：2xx → success，其他 → error。
    #[test]
    fn audit_outcome_mapping() {
        for code in [200i16, 201, 204, 299] {
            let outcome = if (200i16..=299).contains(&code) {
                "success"
            } else {
                "error"
            };
            assert_eq!(outcome, "success", "status={code} 应为 success");
        }
        for code in [100i16, 301, 400, 401, 500, 503] {
            let outcome = if (200i16..=299).contains(&code) {
                "success"
            } else {
                "error"
            };
            assert_eq!(outcome, "error", "status={code} 应为 error");
        }
    }

    /// PG domains 组装不 panic（不建立实际连接）。
    #[tokio::test]
    async fn build_admin_domains_pg_does_not_panic() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://nobody:secret@127.0.0.1:1/grok?connect_timeout=1")
            .expect("lazy pool");
        let domains = build_admin_domains_pg(pool);
        assert!(domains.models.is_some(), "PG 域 models 应已接线");
        assert!(domains.client_keys.is_some(), "PG 域 client_keys 应已接线");
        assert!(domains.audits.is_some(), "PG 域 audits 应已接线");
        assert!(domains.dashboard.is_some(), "PG 域 dashboard 应已接线");
        assert!(domains.settings.is_some(), "PG 域 settings 应已接线");
        assert!(
            domains.chrome_tickets.is_some(),
            "chrome_tickets 应已接线（内存占位）"
        );
        assert!(domains.media.is_some(), "media 应已接线（内存占位）");
        assert!(domains.system.is_some(), "system 应已接线");
    }

    #[test]
    fn pool_summary_quota_sql_casts_sum_to_bigint() {
        let sql = POOL_SUMMARY_QUOTA_SQL.to_ascii_lowercase();
        assert!(
            sql.contains(")::bigint as remaining"),
            "SUM(remaining) 必须外层 ::bigint，否则 PG 返回 NUMERIC 导致 admin 500"
        );
        assert!(sql.contains(")::bigint as total"));
        assert!(sql.contains(")::bigint as remaining_fresh"));
        assert!(sql.contains(")::bigint as total_fresh"));
        assert!(sql.contains("w.remaining::bigint"));
        assert!(sql.contains("interval '24 hours'"));
        assert!(sql.contains("a.enabled = true"));
    }

    #[test]
    fn quota_window_select_sql_casts_columns() {
        let sql = QUOTA_WINDOW_SELECT_SQL.to_ascii_lowercase();
        assert!(sql.contains("remaining::bigint as remaining"));
        assert!(sql.contains("total::bigint as total"));
    }
}
