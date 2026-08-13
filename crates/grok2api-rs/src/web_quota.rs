//! Web 额度刷新：直连 `/rest/rate-limits` → 写 `grok_quota_windows`。

use std::sync::Arc;

use chrono::{Duration, Utc};
use grok_domain::{ProviderError, QuotaSource, QuotaWindow, SsoTokenProvider};
use grok_provider_web::HttpDirectClient;
use serde_json::Value;
use sqlx::PgPool;

/// 候选账号查询：按 fast 窗口 synced_at 升序，未同步账号优先（NULLS FIRST）。
/// 546 账号 / batch 32 / 60s → 约 17 分钟完成全轮。
pub(crate) const BATCH_CANDIDATE_SQL: &str = "SELECT a.id \
     FROM grok_accounts a \
     LEFT JOIN grok_quota_windows w ON w.account_id = a.id AND w.mode = 'fast' \
     WHERE a.provider = 'grok_web' AND a.enabled = true \
     ORDER BY w.synced_at ASC NULLS FIRST \
     LIMIT $1";

/// 单账号额度上游刷新 + PG upsert。
#[derive(Clone)]
pub struct WebQuotaService {
    direct: Arc<HttpDirectClient>,
    sso: Arc<dyn SsoTokenProvider>,
    pool: PgPool,
}

impl WebQuotaService {
    pub fn new(
        direct: Arc<HttpDirectClient>,
        sso: Arc<dyn SsoTokenProvider>,
        pool: PgPool,
    ) -> Self {
        Self { direct, sso, pool }
    }

    /// 拉上游 rate-limits 并 upsert `fast` 窗口；成功后清除过期错误状态。
    pub async fn refresh_account(&self, account_id: i64) -> Result<QuotaWindow, ProviderError> {
        let token = self.sso.sso_token(account_id).await?;
        let data = self
            .direct
            .fetch_rate_limits(Some(&token), Some(account_id))
            .await?;
        let window = quota_window_from_rate_limits(account_id, &data)?;
        self.upsert_window(&window)
            .await
            .map_err(|e| ProviderError::NotConfigured(format!("quota upsert: {e}")))?;
        // 成功后清除过期的 last_error / cooldown_until，避免永久滞留。
        if let Err(e) = self.clear_stale_account_state(account_id).await {
            tracing::warn!(
                account_id,
                "quota refresh 后清理账号状态失败（非致命）: {e}"
            );
        }
        Ok(window)
    }

    async fn upsert_window(&self, window: &QuotaWindow) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO grok_quota_windows \
             (account_id, mode, remaining, total, reset_at, synced_at, source, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,now()) \
             ON CONFLICT (account_id, mode) DO UPDATE SET \
               remaining = EXCLUDED.remaining, total = EXCLUDED.total, \
               reset_at = EXCLUDED.reset_at, synced_at = EXCLUDED.synced_at, \
               source = EXCLUDED.source, updated_at = now()",
        )
        .bind(window.account_id)
        .bind(&window.mode)
        .bind(window.remaining)
        .bind(window.total)
        .bind(window.reset_at)
        .bind(window.synced_at)
        .bind(window.source.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 清除已过期的错误状态：last_error 置 NULL，cooldown_until <= now() 时置 NULL。
    /// 未来冷却期不受影响；仅在有实际需要清除时才执行 UPDATE。
    async fn clear_stale_account_state(&self, account_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE grok_accounts \
             SET last_error = NULL, \
                 cooldown_until = CASE \
                     WHEN cooldown_until IS NOT NULL AND cooldown_until <= now() THEN NULL \
                     ELSE cooldown_until \
                 END \
             WHERE id = $1 \
               AND (last_error IS NOT NULL \
                    OR (cooldown_until IS NOT NULL AND cooldown_until <= now()))",
        )
        .bind(account_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 批量刷新 enabled grok_web 账号额度（单轮失败不中断）。
    pub async fn refresh_enabled_batch(&self, limit: i64) -> (usize, usize) {
        let ids = match sqlx::query_scalar::<_, i64>(BATCH_CANDIDATE_SQL)
            .bind(limit.max(1))
            .fetch_all(&self.pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("quota batch list failed: {e}");
                return (0, 0);
            }
        };
        let mut ok = 0usize;
        let mut fail = 0usize;
        for id in ids {
            match self.refresh_account(id).await {
                Ok(_) => ok += 1,
                Err(e) => {
                    fail += 1;
                    tracing::debug!(account_id = id, "quota refresh skip/fail: {e}");
                }
            }
        }
        (ok, fail)
    }
}

/// synced_at 为空或早于 24 小时 → 视为过期。
pub(crate) fn is_quota_stale(synced_at: Option<chrono::DateTime<chrono::Utc>>) -> bool {
    match synced_at {
        None => true,
        Some(t) => t < Utc::now() - Duration::hours(24),
    }
}

fn quota_window_from_rate_limits(
    account_id: i64,
    data: &Value,
) -> Result<QuotaWindow, ProviderError> {
    let remaining = data
        .get("remainingQueries")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let total = data
        .get("totalQueries")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let wait_secs = data
        .get("waitTimeSeconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let reset_at = if wait_secs > 0 {
        Some(Utc::now() + Duration::seconds(wait_secs))
    } else {
        None
    };
    let now = Utc::now();
    Ok(QuotaWindow {
        account_id,
        mode: "fast".into(),
        remaining,
        total,
        reset_at,
        synced_at: Some(now),
        source: QuotaSource::Upstream,
        updated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_rate_limits_json() {
        let w = quota_window_from_rate_limits(
            42,
            &json!({
                "remainingQueries": 7,
                "totalQueries": 80,
                "waitTimeSeconds": 120
            }),
        )
        .unwrap();
        assert_eq!(w.account_id, 42);
        assert_eq!(w.mode, "fast");
        assert_eq!(w.remaining, 7);
        assert_eq!(w.total, 80);
        assert!(w.reset_at.is_some());
        assert_eq!(w.source, QuotaSource::Upstream);
    }

    #[test]
    fn batch_candidate_sql_uses_left_join_and_fast_mode() {
        let sql = BATCH_CANDIDATE_SQL.to_ascii_lowercase();
        assert!(sql.contains("left join"), "应使用 LEFT JOIN 保留无窗口账号");
        assert!(sql.contains("grok_quota_windows"), "应 JOIN 额度窗口表");
        assert!(
            sql.contains("mode = 'fast'"),
            "应限定 fast 窗口以匹配刷新目标"
        );
        assert!(
            sql.contains("nulls first"),
            "未同步账号（synced_at IS NULL）应排最前"
        );
        assert!(
            sql.contains("provider = 'grok_web'"),
            "应过滤 grok_web provider"
        );
        assert!(sql.contains("enabled = true"), "应过滤启用账号");
    }

    #[test]
    fn is_quota_stale_none_is_stale() {
        assert!(is_quota_stale(None), "无 synced_at 视为过期");
    }

    #[test]
    fn is_quota_stale_old_is_stale() {
        let old = Utc::now() - Duration::hours(25);
        assert!(is_quota_stale(Some(old)), "25h 前同步视为过期");
    }

    #[test]
    fn is_quota_stale_just_over_boundary_is_stale() {
        // 24h+1s 前 → 过期
        let boundary = Utc::now() - Duration::hours(24) - Duration::seconds(1);
        assert!(is_quota_stale(Some(boundary)));
    }

    #[test]
    fn is_quota_stale_fresh_is_not_stale() {
        let fresh = Utc::now() - Duration::hours(1);
        assert!(!is_quota_stale(Some(fresh)), "1h 前同步不视为过期");
    }
}
