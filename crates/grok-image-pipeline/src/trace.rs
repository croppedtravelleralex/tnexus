//! 生图流水线 trace / segment 落库。
//!
//! 结构映射 Go `domain/imagepipeline/types.go` 与
//! `relational/image_pipeline_models.go`；表 `grok_pipeline_traces` /
//! `grok_pipeline_segments`（migrations/015）。
//!
//! `Stage` / `Status` 与 DB CHECK 完全一致。`expand` 是 `ps` 的历史读别名
//! （Go `NormalizeStage`），写入前统一归一为 `ps`。

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::PipelineError;

/// 流水线阶段，对应 `grok_pipeline_segments.stage` CHECK。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Queue,
    QueueUpload,
    Upload,
    QueuePs,
    Ps,
    /// 历史读别名；写入时归一为 `Ps`（Go `NormalizeStage`）。
    Expand,
    QueueSs,
    Sse,
    QueueDownload,
    Download,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Queue => "queue",
            Stage::QueueUpload => "queue_upload",
            Stage::Upload => "upload",
            Stage::QueuePs => "queue_ps",
            Stage::Ps => "ps",
            Stage::Expand => "expand",
            Stage::QueueSs => "queue_ss",
            Stage::Sse => "sse",
            Stage::QueueDownload => "queue_download",
            Stage::Download => "download",
        }
    }

    /// 归一化：`Expand` → `Ps`；其余原样（Go `NormalizeStage`）。
    pub fn normalize(self) -> Self {
        match self {
            Stage::Expand => Stage::Ps,
            other => other,
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Trace 状态，对应 `grok_pipeline_traces.status` CHECK。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Running => "running",
            Status::Succeeded => "succeeded",
            Status::Failed => "failed",
            Status::Canceled => "canceled",
        }
    }
}

/// 单张生图请求的 trace 主记录。
#[derive(Debug, Clone)]
pub struct PipelineTrace {
    pub id: String,
    pub request_id: String,
    pub lane: i32,
    pub status: Status,
    pub model: String,
    pub account_id: Option<i64>,
    pub account_name: String,
    pub error_code: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub queue_ms: i64,
    pub upload_queue_ms: i64,
    pub ps_queue_ms: i64,
    pub ss_queue_ms: i64,
    pub download_queue_ms: i64,
    pub expand_ms: i64,
    pub ssems: i64,
    pub download_ms: i64,
    pub total_ms: i64,
    pub soft_stop: bool,
}

/// 单阶段 segment 记录。
#[derive(Debug, Clone)]
pub struct PipelineSegment {
    pub id: Option<i64>,
    pub trace_id: String,
    pub stage: Stage,
    pub slot: i32,
    pub sequence: i32,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: String,
}

/// stable trace id：会话内先用内存 id；落库时若为空则生成。
pub(crate) fn generate_trace_id(existing: &str) -> String {
    if !existing.trim().is_empty() {
        return existing.to_string();
    }
    // g2-<uuid> 满足长度 16..64 CHECK 且可读。
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("g2-{u}")
}

/// trace / segment 持久化仓储 trait（可注入 PG 或内存 fake）。
#[async_trait]
pub trait TraceRepository: Send + Sync {
    /// 写入（或更新）一条 trace 主记录。
    async fn upsert_trace(&self, trace: &PipelineTrace) -> Result<(), PipelineError>;
    /// 写入一段 segment。
    async fn insert_segment(&self, seg: &PipelineSegment) -> Result<(), PipelineError>;
}

/// SQLite / 内存 fake 仓储，供单测与无 PG 环境使用。
#[derive(Debug, Default)]
pub struct InMemoryTraceRepository {
    pub traces: std::sync::Mutex<Vec<PipelineTrace>>,
    pub segments: std::sync::Mutex<Vec<PipelineSegment>>,
}

impl InMemoryTraceRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TraceRepository for InMemoryTraceRepository {
    async fn upsert_trace(&self, trace: &PipelineTrace) -> Result<(), PipelineError> {
        self.traces.lock().unwrap().push(trace.clone());
        Ok(())
    }

    async fn insert_segment(&self, seg: &PipelineSegment) -> Result<(), PipelineError> {
        // 与 PG 实现一致：写入前归一化 stage（expand→ps），保证 fake 行为忠实。
        let mut seg = seg.clone();
        seg.stage = seg.stage.normalize();
        self.segments.lock().unwrap().push(seg);
        Ok(())
    }
}

/// PostgreSQL 仓储（`grok_*` 表）。
#[derive(Debug, Clone)]
pub struct PgTraceRepository {
    pool: sqlx::PgPool,
}

impl PgTraceRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TraceRepository for PgTraceRepository {
    async fn upsert_trace(&self, trace: &PipelineTrace) -> Result<(), PipelineError> {
        sqlx::query(
            "INSERT INTO grok_pipeline_traces (\
                 id, request_id, lane, status, model, account_id, account_name, error_code, \
                 started_at, ended_at, queue_ms, upload_queue_ms, ps_queue_ms, ss_queue_ms, \
                 download_queue_ms, expand_ms, ssems, download_ms, total_ms, soft_stop) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20) \
             ON CONFLICT (id) DO UPDATE SET \
                 status = EXCLUDED.status, ended_at = EXCLUDED.ended_at, \
                 queue_ms = EXCLUDED.queue_ms, upload_queue_ms = EXCLUDED.upload_queue_ms, \
                 ps_queue_ms = EXCLUDED.ps_queue_ms, ss_queue_ms = EXCLUDED.ss_queue_ms, \
                 download_queue_ms = EXCLUDED.download_queue_ms, expand_ms = EXCLUDED.expand_ms, \
                 ssems = EXCLUDED.ssems, download_ms = EXCLUDED.download_ms, \
                 total_ms = EXCLUDED.total_ms, soft_stop = EXCLUDED.soft_stop",
        )
        .bind(&trace.id)
        .bind(&trace.request_id)
        .bind(trace.lane)
        .bind(trace.status.as_str())
        .bind(&trace.model)
        .bind(trace.account_id)
        .bind(&trace.account_name)
        .bind(&trace.error_code)
        .bind(trace.started_at)
        .bind(trace.ended_at)
        .bind(trace.queue_ms)
        .bind(trace.upload_queue_ms)
        .bind(trace.ps_queue_ms)
        .bind(trace.ss_queue_ms)
        .bind(trace.download_queue_ms)
        .bind(trace.expand_ms)
        .bind(trace.ssems)
        .bind(trace.download_ms)
        .bind(trace.total_ms)
        .bind(trace.soft_stop)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert_segment(&self, seg: &PipelineSegment) -> Result<(), PipelineError> {
        sqlx::query(
            "INSERT INTO grok_pipeline_segments \
                 (trace_id, stage, slot, sequence, started_at, ended_at, outcome) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(&seg.trace_id)
        .bind(seg.stage.normalize().as_str())
        .bind(seg.slot)
        .bind(seg.sequence)
        .bind(seg.started_at)
        .bind(seg.ended_at)
        .bind(&seg.outcome)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
