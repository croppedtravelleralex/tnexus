//! grok-image-pipeline — 生图流水线槽位 + trace/segment 元数据落库（G2）。
//!
//! 范围（39e G2-P1 + 39c §3 生图矩阵 + 39d §5）：
//! - `slots`：imagine 自身并发槽（PS / SS 等具名池，RAII 归还）。
//! - `trace`：`grok_pipeline_traces` / `grok_pipeline_segments` 结构 + 落库。
//! - `ImagePipeline` 门面：`reserve_slot` / `begin_trace`（含 `record_segment`）。
//!
//! **不实现** PS/SS 实际调度逻辑（与上游 provider 交互，属 grok-provider-web）；
//! 本 crate 只负责并发槽 + 阶段耗时元数据落库（G2-A2：耗时字段存在）。

pub mod slots;
pub mod trace;

use std::sync::Arc;

use chrono::Utc;

pub use slots::{SlotGuard, SlotManager, SlotPool};
pub use trace::{
    InMemoryTraceRepository, PgTraceRepository, PipelineSegment, PipelineTrace, Stage, Status,
    TraceRepository,
};

/// grok-image-pipeline 错误。
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("unknown slot pool `{0}`")]
    UnknownPool(String),
    #[error("slot pool `{pool}` full")]
    PoolFull { pool: String },
    #[error("slot pool `{0}` closed")]
    SlotPoolClosed(String),
    #[error("slot acquisition timed out after {timeout:?} on pool `{pool}`")]
    SlotTimeout {
        pool: String,
        timeout: std::time::Duration,
    },
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// fake/memory 仓储错误兜底。
    #[error("storage error: {0}")]
    Storage(String),
}

/// 生图流水线门面。
#[derive(Clone)]
pub struct ImagePipeline {
    slots: Arc<SlotManager>,
    repo: Arc<dyn TraceRepository>,
}

impl ImagePipeline {
    /// 构建门面。`slots` 给出具名槽位池，`repo` 提供持久化后端。
    pub fn new(slots: SlotManager, repo: Arc<dyn TraceRepository>) -> Self {
        Self {
            slots: Arc::new(slots),
            repo,
        }
    }

    /// 内存后端构建（测试/无 PG），PS=2 / SS=1。
    pub fn with_in_memory(pools: &[(&'static str, usize)]) -> (Self, Arc<InMemoryTraceRepository>) {
        let repo = Arc::new(InMemoryTraceRepository::new());
        let pipeline = Self::new(SlotManager::new(pools), repo.clone());
        (pipeline, repo)
    }

    /// 非阻塞获取一个 imagine 槽位。
    pub fn reserve_slot(&self, pool: &str) -> Result<SlotGuard, PipelineError> {
        self.slots.try_reserve(pool)
    }

    /// 阻塞式获取一个 imagine 槽位（带超时）。
    pub async fn reserve_slot_timeout(
        &self,
        pool: &str,
        timeout: std::time::Duration,
    ) -> Result<SlotGuard, PipelineError> {
        self.slots.reserve(pool, timeout).await
    }

    /// 当前具名池占用数。
    pub fn active_slots(&self, pool: &str) -> usize {
        self.slots.active(pool)
    }

    /// 直接落一条 trace 主记录（不自动生成 id 时使用现有）。
    pub async fn record_trace(&self, trace: &PipelineTrace) -> Result<(), PipelineError> {
        if trace.id.trim().len() < 16 {
            return Err(PipelineError::InvalidInput(format!(
                "trace id must be 16..64 chars, got {}",
                trace.id.len()
            )));
        }
        self.repo.upsert_trace(trace).await
    }

    /// 直接落一段 segment。
    pub async fn record_segment(&self, seg: &PipelineSegment) -> Result<(), PipelineError> {
        if seg.trace_id.trim().len() < 16 {
            return Err(PipelineError::InvalidInput(format!(
                "segment trace_id must be 16..64 chars, got {}",
                seg.trace_id.len()
            )));
        }
        self.repo.insert_segment(seg).await
    }

    /// 开启一条 trace，返回 `TraceRecorder` 用于追加分段与收尾。
    ///
    /// - `trace_id` 可省略（传空）→ 自动生成 `g2-<uuid>`。
    /// - 立即以 `Running` 状态落库一条主记录。
    pub async fn begin_trace(
        &self,
        mut trace: PipelineTrace,
    ) -> Result<TraceRecorder, PipelineError> {
        if trace.id.trim().is_empty() {
            trace.id = crate::trace::generate_trace_id(&trace.id);
        }
        if trace.id.trim().len() < 16 {
            return Err(PipelineError::InvalidInput("trace id too short".into()));
        }
        trace.status = Status::Running;
        trace.started_at = trace.started_at.max(Utc::now());
        self.repo.upsert_trace(&trace).await?;
        Ok(TraceRecorder {
            repo: self.repo.clone(),
            trace,
            segments: Vec::new(),
            seq: 0,
        })
    }
}

/// 进行中的 trace 记录器：追加分段（耗时/槽位）并收尾。
pub struct TraceRecorder {
    repo: Arc<dyn TraceRepository>,
    trace: PipelineTrace,
    segments: Vec<PipelineSegment>,
    seq: i32,
}

impl TraceRecorder {
    /// 追加一段（自动递增 sequence）。`stage` 写库时归一（`expand`→`ps`）。
    pub fn add_segment(&mut self, stage: Stage, duration_ms: i64, slot: i32) {
        let now = Utc::now();
        let started = now - chrono::Duration::milliseconds(duration_ms.max(0));
        self.segments.push(PipelineSegment {
            id: None,
            trace_id: self.trace.id.clone(),
            stage,
            slot,
            sequence: self.seq,
            started_at: started,
            ended_at: Some(now),
            outcome: String::new(),
        });
        self.seq += 1;
    }

    /// 收尾：写终态耗时并落 trace + 全部分段。
    pub async fn finish(mut self, status: Status) -> Result<(), PipelineError> {
        let ended = Utc::now();
        let total = (ended - self.trace.started_at).num_milliseconds();
        self.trace.status = status;
        self.trace.ended_at = Some(ended);
        self.trace.total_ms = total;
        self.repo.upsert_trace(&self.trace).await?;
        for seg in &self.segments {
            self.repo.insert_segment(seg).await?;
        }
        Ok(())
    }

    pub fn trace_id(&self) -> &str {
        &self.trace.id
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{Stage, Status};

    fn sample_trace(trace_id: &str) -> PipelineTrace {
        PipelineTrace {
            id: trace_id.to_string(),
            request_id: "req-1".into(),
            lane: 0,
            status: Status::Queued,
            model: "grok-imagine-image".into(),
            account_id: Some(42),
            account_name: "acct".into(),
            error_code: String::new(),
            started_at: Utc::now(),
            ended_at: None,
            queue_ms: 12,
            upload_queue_ms: 0,
            ps_queue_ms: 0,
            ss_queue_ms: 0,
            download_queue_ms: 0,
            expand_ms: 0,
            ssems: 0,
            download_ms: 0,
            total_ms: 0,
            soft_stop: false,
        }
    }

    #[tokio::test]
    async fn begin_trace_generates_id_and_records() {
        let (pipeline, repo) = ImagePipeline::with_in_memory(&[("ps", 2)]);
        let rec = pipeline.begin_trace(sample_trace("")).await.expect("begin");
        assert_eq!(rec.trace_id().len(), 3 + 32); // "g2-" (3) + 32-hex uuid
        assert_eq!(rec.segment_count(), 0);
        let n_before = repo.traces.lock().unwrap().len();
        assert_eq!(n_before, 1); // running trace persisted
        rec.finish(Status::Succeeded).await.expect("finish");
        let traces = repo.traces.lock().unwrap();
        let t = traces.last().unwrap();
        assert_eq!(t.status, Status::Succeeded);
        assert!(t.ended_at.is_some());
        assert!(t.total_ms >= 0, "total_ms 耗时字段聚合");
        assert_eq!(t.id.len(), 35, "g2- + 32-hex uuid trace id length");
    }

    #[tokio::test]
    async fn add_segments_and_finish_persists_all() {
        let (pipeline, repo) = ImagePipeline::with_in_memory(&[("ps", 2)]);
        let mut rec = pipeline.begin_trace(sample_trace("")).await.expect("begin");
        rec.add_segment(Stage::Queue, 5, -1);
        rec.add_segment(Stage::Ps, 30, 0);
        rec.add_segment(Stage::Sse, 100, 0);
        assert_eq!(rec.segment_count(), 3);
        rec.finish(Status::Succeeded).await.expect("finish");

        let segs = repo.segments.lock().unwrap();
        assert_eq!(segs.len(), 3);
        // sequence 递增
        assert_eq!(segs[0].sequence, 0);
        assert_eq!(segs[1].sequence, 1);
        assert_eq!(segs[2].sequence, 2);
        assert_eq!(segs[1].stage, Stage::Ps);
        assert_eq!(segs[1].slot, 0);
        assert!(segs[1].ended_at.is_some());
        // 耗时字段可推导（started≈ended−ms）
        let dur = (segs[1].ended_at.unwrap() - segs[1].started_at).num_milliseconds();
        assert!(dur >= 0);
    }

    #[tokio::test]
    async fn record_segment_normalizes_expand_to_ps() {
        let (pipeline, repo) = ImagePipeline::with_in_memory(&[("ps", 2)]);
        let tid = "g2-123456789012345678901234".to_string();
        pipeline
            .record_segment(&PipelineSegment {
                id: None,
                trace_id: tid.clone(),
                stage: Stage::Expand,
                slot: 1,
                sequence: 0,
                started_at: Utc::now(),
                ended_at: None,
                outcome: "x".into(),
            })
            .await
            .expect("segment");
        // 落库时归一为 ps（此处内存 fake 也归一，保证与 PG 一致）
        let segs = repo.segments.lock().unwrap();
        assert_eq!(segs[0].stage, Stage::Ps);
    }

    #[tokio::test]
    async fn short_trace_id_rejected() {
        let (pipeline, _repo) = ImagePipeline::with_in_memory(&[("ps", 2)]);
        let r = pipeline.record_trace(&sample_trace("short")).await;
        assert!(matches!(r, Err(PipelineError::InvalidInput(_))));
    }

    #[tokio::test]
    async fn slots_and_trace_integration() {
        let (pipeline, _repo) = ImagePipeline::with_in_memory(&[("ps", 2), ("ss", 1)]);
        let g = pipeline.reserve_slot("ps").expect("ps slot");
        assert_eq!(pipeline.active_slots("ps"), 1);
        drop(g);
        assert_eq!(pipeline.active_slots("ps"), 0);
    }
}
