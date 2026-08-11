//! Web 额度刷新：直连 `/rest/rate-limits` → 写 `grok_quota_windows`。

use std::sync::Arc;

use chrono::{Duration, Utc};
use grok_domain::{ProviderError, QuotaSource, QuotaWindow, SsoTokenProvider};
use grok_provider_web::HttpDirectClient;
use serde_json::Value;
use sqlx::PgPool;

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

    /// 拉上游 rate-limits 并 upsert `fast` 窗口。
    pub async fn refresh_account(&self, account_id: i64) -> Result<QuotaWindow, ProviderError> {
        let token = self.sso.sso_token(account_id).await?;
        let data = self
            .direct
            .fetch_rate_limits(Some(&token), Some(account_id))
            .await?;
        let window = quota_window_from_rate_limits(account_id, &data)?;
        self.upsert_window(&window).await.map_err(|e| {
            ProviderError::NotConfigured(format!("quota upsert: {e}"))
        })?;
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

    /// 批量刷新 enabled grok_web 账号额度（单轮失败不中断）。
    pub async fn refresh_enabled_batch(&self, limit: i64) -> (usize, usize) {
        let ids = match sqlx::query_scalar::<_, i64>(
            "SELECT id FROM grok_accounts WHERE provider = 'grok_web' AND enabled = true \
             ORDER BY updated_at ASC NULLS FIRST LIMIT $1",
        )
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
}
