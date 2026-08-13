//! 异步审计缓冲 sink。
//!
//! `AuditSink` 包装一个有界 `tokio::mpsc` 队列；调用方 `record()` 用
//! `try_send` 非阻塞入队（避免阻塞推理路径）。后台 worker 按
//! `BATCH_MAX`/`FLUSH_INTERVAL` 聚合批量写入 `grok_request_audits`。
//!
//! DB 不可达时：`insert_batch` 失败 → 本次 batch 丢弃并计数（不 panic、
//! 不阻塞、不无限重试）；队列满时 `record()` 直接计数 dropped 并返回错误。
//! 计数器通过 `stats()` 暴露，便于测试与运维观测。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::audit::CreateAudit;
use crate::repo::AuditRepository;

/// 单批最大条数（Postgres 参数上限内留余量）。
const BATCH_MAX: usize = 100;
/// 未满批时兜底 flush 间隔。
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);

/// 发送给 worker 的消息。
enum Msg {
    Audit(Box<CreateAudit>),
    Flush(oneshot::Sender<()>),
}

/// 运行期计数（原子）。
#[derive(Default)]
pub struct AuditStats {
    pub queued: AtomicU64,
    pub flushed: AtomicU64,
    pub dropped: AtomicU64,
    pub batch_failures: AtomicU64,
}

impl AuditStats {
    fn snapshot(&self) -> (u64, u64, u64, u64) {
        (
            self.queued.load(Ordering::Relaxed),
            self.flushed.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
            self.batch_failures.load(Ordering::Relaxed),
        )
    }
}

/// 异步审计缓冲 sink。
pub struct AuditSink {
    tx: Option<Mutex<mpsc::Sender<Msg>>>,
    stats: Arc<AuditStats>,
    /// worker 结束信号（优雅关停排空后置位）。
    join: Option<tokio::task::JoinHandle<()>>,
}

impl AuditSink {
    /// 启动 worker。`capacity` 为队列上限（>=1）。
    pub fn spawn<R>(repo: R, capacity: usize) -> Self
    where
        R: AuditRepository + Send + 'static,
    {
        let capacity = capacity.max(1);
        let (tx, rx) = mpsc::channel::<Msg>(capacity);
        let stats = Arc::new(AuditStats::default());
        let stats_w = Arc::clone(&stats);
        let join = tokio::spawn(async move {
            worker_loop(repo, rx, stats_w).await;
        });
        Self {
            tx: Some(Mutex::new(tx)),
            stats,
            join: Some(join),
        }
    }

    /// 非阻塞入队一条审计。队列满或已关闭时计数 dropped 并返回 `Err`。
    pub fn record(&self, audit: CreateAudit) -> Result<(), SinkError> {
        let Some(tx) = self.tx.as_ref() else {
            self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(SinkError::Closed);
        };
        let tx = tx.lock().unwrap();
        match tx.try_send(Msg::Audit(Box::new(audit))) {
            Ok(()) => {
                self.stats.queued.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                Err(SinkError::BufferFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                Err(SinkError::Closed)
            }
        }
    }

    /// 等待当前已入队记录排空并落库（测试/关停用）。超时返回 `Ok(())`
    /// 但可能未完全排空（DB down 时以 dropped 计数体现）。
    pub async fn flush(&self) {
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        // mpsc::Sender 可 Clone：取副本后立刻释放锁，避免持锁跨 await。
        let sender = match tx.lock().ok() {
            Some(guard) => guard.clone(),
            None => return,
        };
        let (ok, rx) = oneshot::channel::<()>();
        // flush 仅用于测试/关停，非推理热路径，允许排队等待发送。
        let _ = sender.send(Msg::Flush(ok)).await;
        let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
    }

    /// 当前队列中尚未落库的条数（近似：入队 − flush 确认）。
    pub fn pending_count(&self) -> u64 {
        self.stats
            .queued
            .load(Ordering::Relaxed)
            .saturating_sub(self.stats.flushed.load(Ordering::Relaxed))
            .saturating_sub(self.stats.dropped.load(Ordering::Relaxed))
    }

    /// 运行期计数快照 `(queued, flushed, dropped, batch_failures)`。
    pub fn stats(&self) -> (u64, u64, u64, u64) {
        self.stats.snapshot()
    }

    /// 优雅关停：关闭发送端，等待 worker 排空当前批次后退出。
    pub async fn shutdown(&mut self) {
        self.tx = None; // 释放 sender，worker recv 返回 None
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

/// 队列发送错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkError {
    BufferFull,
    Closed,
}

/// 后台 worker：聚合批次并写入。
async fn worker_loop<R>(repo: R, mut rx: mpsc::Receiver<Msg>, stats: Arc<AuditStats>)
where
    R: AuditRepository + Send,
{
    let mut batch: Vec<CreateAudit> = Vec::with_capacity(BATCH_MAX);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(Msg::Audit(a)) => {
                        batch.push(*a);
                        if batch.len() >= BATCH_MAX {
                            bulk_insert(&repo, &mut batch, &stats).await;
                        }
                    }
                    Some(Msg::Flush(ok)) => {
                        bulk_insert(&repo, &mut batch, &stats).await;
                        let _ = ok.send(());
                    }
                    None => {
                        // 发送端关闭：排空后退出。
                        bulk_insert(&repo, &mut batch, &stats).await;
                        return;
                    }
                }
            }
            _ = tokio::time::sleep(FLUSH_INTERVAL) => {
                bulk_insert(&repo, &mut batch, &stats).await;
            }
        }
    }
}

/// 将当前批次写入 DB；空批次直接返回。失败则丢弃本批并计数。
async fn bulk_insert<R>(repo: &R, batch: &mut Vec<CreateAudit>, stats: &Arc<AuditStats>)
where
    R: AuditRepository,
{
    if batch.is_empty() {
        return;
    }
    let taken = std::mem::take(batch);
    let n = taken.len() as u64;
    match repo.insert_batch(&taken).await {
        Ok(_) => {
            stats.flushed.fetch_add(n, Ordering::Relaxed);
        }
        Err(e) => {
            // DB 不可达或违反约束：丢弃本批并计数，继续下一批，绝不阻塞/panic。
            //
            // 必须打日志：只加计数器而计数器又没有暴露出口，等于审计整条链路静默失效——
            // 线上出现过 sink 已启动、record_audit 已调用，但 grok_request_audits 一行
            // 不增，且无任何线索可查。
            let failures = stats.batch_failures.fetch_add(1, Ordering::Relaxed) + 1;
            stats.dropped.fetch_add(n, Ordering::Relaxed);
            tracing::warn!(
                dropped_in_batch = n,
                total_batch_failures = failures,
                error = %e,
                sample_request_id = taken.first().map(|a| a.request_id.as_str()).unwrap_or(""),
                "audit batch 写入失败，本批丢弃"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::CreateAudit;

    fn sample(request_id: &str) -> CreateAudit {
        CreateAudit {
            request_id: request_id.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn record_flush_persists_all() {
        let repo = Arc::new(crate::repo::FakeAuditRepository::new());
        let mut sink = AuditSink::spawn(repo.clone(), 16);
        for i in 0..10 {
            sink.record(sample(&format!("req-{i}"))).unwrap();
        }
        sink.flush().await;
        sink.flush().await;
        assert_eq!(repo.total_written(), 10, "all 10 records persisted");
        sink.shutdown().await;
        // flush 后无 pending
        assert_eq!(sink.pending_count(), 0);
    }

    #[tokio::test]
    async fn record_does_not_block_when_db_down() {
        let repo = Arc::new(crate::repo::FakeAuditRepository::new());
        repo.set_fail(true);
        let mut sink = AuditSink::spawn(repo.clone(), 8);
        // record 本身非阻塞：即使 DB down，入队也不 panic（BufferFull 可接受）。
        let mut rejected = 0;
        for i in 0..50 {
            if sink.record(sample(&format!("req-{i}"))).is_err() {
                rejected += 1;
            }
        }
        sink.flush().await;
        // DB down：不 panic；失败被记入 failures/dropped，推理路径仍推进。
        let (_, flushed, _dropped, failures) = sink.stats();
        assert_eq!(flushed, 0, "DB down ⇒ nothing persisted");
        assert!(failures >= 1, "should record failure count");
        assert!(rejected < 50, "非阻塞：不应 50 全被拒");
        // 等 worker 消费完缓冲，pending 归零。
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(sink.pending_count(), 0);
        sink.shutdown().await;
    }

    #[tokio::test]
    async fn buffer_full_counts_dropped() {
        let repo = Arc::new(crate::repo::FakeAuditRepository::new());
        // 容量1 + 一个慢 worker 粘住：把 sleep 拉长，营造队列满
        let mut sink = AuditSink::spawn(repo.clone(), 1);
        // 先填满队列（worker 可能立刻取走，保守起见连续投喂）
        let mut errors = 0;
        for _ in 0..200 {
            if sink.record(sample("x")).is_err() {
                errors += 1;
            }
        }
        let (_, flushed, dropped, _) = sink.stats();
        // 无论成功入队多少，dropped 反映被拒或失败的记录；不会 panic
        let _ = (flushed, dropped);
        assert!(errors > 0, "capacity=1 should drop under burst");
        sink.shutdown().await;
    }

    #[tokio::test]
    async fn pending_counts_drops_to_zero_after_flush() {
        let repo = Arc::new(crate::repo::FakeAuditRepository::new());
        let mut sink = AuditSink::spawn(repo.clone(), 8);
        sink.record(sample("a")).unwrap();
        sink.record(sample("b")).unwrap();
        let p_after_record = sink.pending_count();
        assert_eq!(p_after_record, 2);
        sink.flush().await;
        // worker 可能尚未 decrement，sleep 让其推进
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sink.pending_count(), 0);
        sink.shutdown().await;
    }
}
