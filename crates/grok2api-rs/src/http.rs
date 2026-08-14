//! grok2api-rs HTTP 路由（G0 最小集）。
//!
//! `GET /healthz`：进程存活，不探依赖。
//! `GET /readyz`：DB 可达才 200，否则 503。响应附带号池/额度/审计四项关键指标，
//! 让「现在好不好」可以用一条 curl 回答。错误响应脱敏：不回传 DB DSN / 内部错误 detail。

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;

use grok_audit::AuditSink;
use grok_pool::SharedPool;

/// `/readyz` 判定额度「过旧」的阈值。刷新全池约 64 分钟一轮，两小时仍未同步才算 degraded。
const QUOTA_STALE_AFTER_SECS: i64 = 2 * 60 * 60;

/// Axum 共享状态：DB 池 + 运行时观测句柄。
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub grok_pool: Option<SharedPool>,
    pub audit: Option<Arc<AuditSink>>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            grok_pool: None,
            audit: None,
        }
    }

    pub fn with_grok_pool(mut self, grok_pool: SharedPool) -> Self {
        self.grok_pool = Some(grok_pool);
        self
    }

    pub fn with_audit_opt(mut self, audit: Option<Arc<AuditSink>>) -> Self {
        self.audit = audit;
        self
    }
}

/// 构建最小路由。
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

/// 存活探针：进程活着即 200（不探依赖）。
async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

/// 就绪探针：DB 可达才 200，否则 503。指标在两种情况下都尽量带上。
async fn readyz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let runtime = collect_runtime_snapshot(&state).await;

    let db = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        sqlx::query("SELECT 1").execute(&state.pool),
    )
    .await
    {
        Ok(Ok(_)) => DbProbe::Ok,
        Ok(Err(_)) => DbProbe::Error,
        Err(_) => DbProbe::Timeout,
    };

    let db_metrics = if matches!(db, DbProbe::Ok) {
        collect_db_metrics(&state.pool).await
    } else {
        DbMetrics::default()
    };

    let body = ReadyBody::from_parts(db, runtime, db_metrics);
    let status = if matches!(db, DbProbe::Ok) {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body))
}

#[derive(Clone, Copy)]
enum DbProbe {
    Ok,
    Error,
    Timeout,
}

impl DbProbe {
    fn as_str(self) -> &'static str {
        match self {
            DbProbe::Ok => "ok",
            DbProbe::Error => "error",
            DbProbe::Timeout => "timeout",
        }
    }
}

#[derive(Default)]
struct RuntimeSnapshot {
    pool_size: Option<usize>,
    pool_reconciled_at: Option<DateTime<Utc>>,
    audit: Option<AuditBody>,
}

#[derive(Default)]
struct DbMetrics {
    quota_oldest_synced_at: Option<DateTime<Utc>>,
    quota_windows: Option<i64>,
    credential_missing: Option<i64>,
}

#[derive(Serialize, Clone, Copy)]
struct AuditBody {
    queued: u64,
    flushed: u64,
    dropped: u64,
    batch_failures: u64,
}

#[derive(Serialize)]
struct ReadyBody {
    status: &'static str,
    db: &'static str,
    pool_size: Option<usize>,
    pool_reconciled_at: Option<String>,
    quota_oldest_synced_at: Option<String>,
    quota_windows: Option<i64>,
    credential_missing: Option<i64>,
    audit: Option<AuditBody>,
    degraded: bool,
}

impl ReadyBody {
    fn from_parts(db: DbProbe, runtime: RuntimeSnapshot, db_metrics: DbMetrics) -> Self {
        let quota_stale = db_metrics
            .quota_oldest_synced_at
            .map(|t| (Utc::now() - t).num_seconds() > QUOTA_STALE_AFTER_SECS)
            .unwrap_or(false);
        let degraded = is_degraded(
            runtime.pool_size,
            runtime.audit,
            db_metrics.credential_missing,
            quota_stale,
        );
        Self {
            status: if matches!(db, DbProbe::Ok) {
                "ready"
            } else {
                "not_ready"
            },
            db: db.as_str(),
            pool_size: runtime.pool_size,
            pool_reconciled_at: runtime.pool_reconciled_at.map(|t| t.to_rfc3339()),
            quota_oldest_synced_at: db_metrics.quota_oldest_synced_at.map(|t| t.to_rfc3339()),
            quota_windows: db_metrics.quota_windows,
            credential_missing: db_metrics.credential_missing,
            audit: runtime.audit,
            degraded,
        }
    }
}

/// 号池空、审计在丢行、缺凭据、额度过旧 → degraded。不改变 HTTP 状态码。
fn is_degraded(
    pool_size: Option<usize>,
    audit: Option<AuditBody>,
    credential_missing: Option<i64>,
    quota_stale: bool,
) -> bool {
    matches!(pool_size, Some(0))
        || audit
            .map(|a| a.batch_failures > 0 || a.dropped > 0)
            .unwrap_or(false)
        || credential_missing.unwrap_or(0) > 0
        || quota_stale
}

async fn collect_runtime_snapshot(state: &AppState) -> RuntimeSnapshot {
    let mut snap = RuntimeSnapshot::default();
    if let Some(pool) = &state.grok_pool {
        snap.pool_size = Some(pool.len().await);
        snap.pool_reconciled_at = pool.last_reconciled_at().await;
    }
    if let Some(sink) = &state.audit {
        let (queued, flushed, dropped, batch_failures) = sink.stats();
        snap.audit = Some(AuditBody {
            queued,
            flushed,
            dropped,
            batch_failures,
        });
    }
    snap
}

async fn collect_db_metrics(pool: &PgPool) -> DbMetrics {
    let mut metrics = DbMetrics::default();
    if let Ok(row) = sqlx::query_as::<_, (Option<DateTime<Utc>>, i64)>(
        "SELECT MIN(synced_at), COUNT(*)::bigint FROM grok_quota_windows",
    )
    .fetch_one(pool)
    .await
    {
        metrics.quota_oldest_synced_at = row.0;
        metrics.quota_windows = Some(row.1);
    }
    if let Ok((missing,)) = sqlx::query_as::<_, (i64,)>(
        "SELECT COUNT(*)::bigint
         FROM grok_accounts a
         WHERE a.enabled
           AND a.provider = 'grok_web'
           AND NOT EXISTS (
             SELECT 1 FROM grok_credentials c WHERE c.account_id = a.id
           )",
    )
    .fetch_one(pool)
    .await
    {
        metrics.credential_missing = Some(missing);
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pool_is_degraded() {
        assert!(is_degraded(Some(0), None, None, false));
        assert!(!is_degraded(Some(537), None, None, false));
        assert!(!is_degraded(None, None, None, false));
    }

    #[test]
    fn audit_failures_are_degraded() {
        let bad = AuditBody {
            queued: 10,
            flushed: 0,
            dropped: 10,
            batch_failures: 3,
        };
        assert!(is_degraded(Some(10), Some(bad), None, false));
        let ok = AuditBody {
            queued: 10,
            flushed: 10,
            dropped: 0,
            batch_failures: 0,
        };
        assert!(!is_degraded(Some(10), Some(ok), None, false));
    }

    #[test]
    fn missing_credentials_or_stale_quota_are_degraded() {
        assert!(is_degraded(Some(10), None, Some(4), false));
        assert!(is_degraded(Some(10), None, None, true));
        assert!(!is_degraded(Some(10), None, Some(0), false));
    }

    #[test]
    fn ready_body_omits_dsn_and_detail() {
        let body = ReadyBody::from_parts(DbProbe::Error, RuntimeSnapshot::default(), DbMetrics::default());
        let text = serde_json::to_string(&body).unwrap();
        assert!(!text.contains("postgres://"));
        assert!(!text.contains("detail"));
        assert_eq!(body.status, "not_ready");
        assert_eq!(body.db, "error");
    }

    #[test]
    fn ready_body_includes_pool_metrics_even_when_db_down() {
        let runtime = RuntimeSnapshot {
            pool_size: Some(537),
            pool_reconciled_at: Some(Utc::now()),
            audit: Some(AuditBody {
                queued: 1,
                flushed: 1,
                dropped: 0,
                batch_failures: 0,
            }),
        };
        let body = ReadyBody::from_parts(DbProbe::Timeout, runtime, DbMetrics::default());
        assert_eq!(body.pool_size, Some(537));
        assert!(body.pool_reconciled_at.is_some());
        assert_eq!(body.audit.unwrap().flushed, 1);
        assert_eq!(body.status, "not_ready");
        assert!(!body.degraded);
    }
}
