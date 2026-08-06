//! 审计写仓储：`AuditRepository` trait + PG 批量实现 + 测试 fake。

use async_trait::async_trait;
use sqlx::QueryBuilder;
use thiserror::Error;

use crate::audit::CreateAudit;

/// 审计仓储错误。
#[derive(Debug, Error)]
pub enum AuditRepoError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("poisoned shared state: {0}")]
    Lock(String),
}

/// 批量写入对象，可由测试 fake 实现（无 DB 时验证缓冲行为）。
#[async_trait]
pub trait AuditRepository: Send + Sync + 'static {
    /// 批量写入审计记录。返回成功写入条数。
    async fn insert_batch(&self, audits: &[CreateAudit]) -> Result<usize, AuditRepoError>;
}

/// 使 `Arc<R>` 亦满足 trait（sink 可共享/克隆持有仓储）。
#[async_trait]
impl<T: AuditRepository> AuditRepository for std::sync::Arc<T> {
    async fn insert_batch(&self, audits: &[CreateAudit]) -> Result<usize, AuditRepoError> {
        (**self).insert_batch(audits).await
    }
}

/// PostgreSQL 实现：单条多行 INSERT 写入 `grok_request_audits`。
pub struct PgAuditRepository {
    pool: sqlx::PgPool,
}

impl PgAuditRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AuditRepository for PgAuditRepository {
    async fn insert_batch(&self, audits: &[CreateAudit]) -> Result<usize, AuditRepoError> {
        if audits.is_empty() {
            return Ok(0);
        }
        let mut qb = QueryBuilder::new(
            "INSERT INTO grok_request_audits (\
             event_id, request_id, client_key_id, client_key_name, model_route_id, \
             model_public_id, model_upstream_model, provider, operation, usage_source, \
             account_id, account_name, status_code, streaming, media_input_images, \
             media_output_images, media_output_seconds, input_tokens, cached_input_tokens, \
             output_tokens, reasoning_tokens, total_tokens, cost_in_usd_ticks, \
             estimated_cost_in_usd_ticks, pricing_model, pricing_version, num_sources_used, \
             num_server_side_tools_used, context_input_tokens, context_output_tokens, \
             duration_ms, error_code, created_at) ",
        );
        qb.push_values(audits, |mut b, a| {
            b.push_bind(a.event_id.as_str())
                .push_bind(a.request_id.as_str())
                .push_bind(a.client_key_id)
                .push_bind(a.client_key_name.as_deref())
                .push_bind(a.model_route_id)
                .push_bind(a.model_public_id.as_deref())
                .push_bind(a.model_upstream_model.as_deref())
                .push_bind(a.provider.as_str())
                .push_bind(a.operation.as_str())
                .push_bind(a.usage_source.as_str())
                .push_bind(a.account_id)
                .push_bind(a.account_name.as_deref())
                .push_bind(i32::try_from(a.status_code).unwrap_or(0))
                .push_bind(a.streaming)
                .push_bind(a.media_input_images)
                .push_bind(a.media_output_images)
                .push_bind(a.media_output_seconds)
                .push_bind(a.input_tokens)
                .push_bind(a.cached_input_tokens)
                .push_bind(a.output_tokens)
                .push_bind(a.reasoning_tokens)
                .push_bind(a.total_tokens)
                .push_bind(a.cost_in_usd_ticks)
                .push_bind(a.estimated_cost_in_usd_ticks)
                .push_bind(a.pricing_model.as_deref())
                .push_bind(a.pricing_version.as_deref())
                .push_bind(a.num_sources_used)
                .push_bind(a.num_server_side_tools_used)
                .push_bind(a.context_input_tokens)
                .push_bind(a.context_output_tokens)
                .push_bind(a.duration_ms)
                .push_bind(a.error_code.as_deref())
                .push_bind(a.created_at);
        });
        let rows = qb.build().execute(&self.pool).await?;
        Ok(rows.rows_affected() as usize)
    }
}

/// 内存 fake 仓储（测试用）：记录写入批次与累计条数，可选模拟 DB down。
#[derive(Default)]
pub struct FakeAuditRepository {
    inner: std::sync::Mutex<FakeState>,
}

#[derive(Default)]
struct FakeState {
    batches: Vec<usize>,
    total: usize,
    fail: bool,
    last_flush_seen: usize,
}

impl FakeAuditRepository {
    pub fn new() -> Self {
        Self::default()
    }

    /// 使后续 `insert_batch` 返回 Database 错误（模拟 DB 不可达）。
    pub fn set_fail(&self, fail: bool) {
        self.inner.lock().unwrap().fail = fail;
    }

    pub fn total_written(&self) -> usize {
        self.inner.lock().unwrap().total
    }

    pub fn batch_sizes(&self) -> Vec<usize> {
        self.inner.lock().unwrap().batches.clone()
    }

    pub fn flush_seen_count(&self) -> usize {
        self.inner.lock().unwrap().last_flush_seen
    }
}

#[async_trait]
impl AuditRepository for FakeAuditRepository {
    async fn insert_batch(&self, audits: &[CreateAudit]) -> Result<usize, AuditRepoError> {
        let mut st = self.inner.lock().unwrap();
        if st.fail {
            return Err(AuditRepoError::Database(sqlx::Error::Io(
                std::io::Error::other("fake db down"),
            )));
        }
        st.batches.push(audits.len());
        st.total += audits.len();
        Ok(audits.len())
    }
}
