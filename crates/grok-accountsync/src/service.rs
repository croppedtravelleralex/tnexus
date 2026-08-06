//! 账号同步服务（对齐 Go `internal/application/accountsync/service.go`）。
//!
//! 对新接入账号执行一次性「额度 + 模型」补齐，并用固定 Worker 数限制批量同步并发
//! （G4-A2：Web import → accountsync → 可 chat）。
//!
//! 语义：
//! - `Sync(ids...)` / `SyncStream(input)` 用 `workers` 个 `tokio` task 消费账号流，
//!   每账号做 billing **或** quota（按 Provider 额度策略）+ models 两路补齐；
//!   已同步的快照跳过（`has_*`），同账号在流内去重，0 id 忽略。
//! - `sync_stream_observed` 在每个去重账号完成后回调 `observer(completed, total)`。
//! - 每账号三路任何一路失败都计入 `failed`（部分成功仍计失败，对齐 Go）。

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::error::Error;

/// 默认 Worker 数（Go `defaultWorkerCount`）。
pub const DEFAULT_WORKER_COUNT: usize = 25;
/// 单次上游操作超时（Go `operationTimeout`）。
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(120);

/// 上游 Provider（独立定义，避免依赖 grok-domain 的 workspace 继承）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GrokBuild,
    GrokWeb,
    GrokConsole,
}

/// 额度策略（Go `provider.QuotaKind`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaKind {
    /// Build：统一账单快照（billing 路径）。
    Billing,
    /// Web：上游远程窗口额度（quota 路径）。
    RemoteWindow,
    /// Console：本地窗口额度（quota 路径）。
    LocalWindow,
}

/// 本次初始同步的结果汇总（Go `Result`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncResult {
    pub succeeded: usize,
    pub failed: usize,
}

/// 账号同步 IO 抽象（对齐 Go `billingSynchronizer`/`quotaSynchronizer`/
/// `modelSynchronizer`/`accountReader` + `providerPolicy` 的组合）。
///
/// `get_provider` 返回账号的 Provider 与额度策略（Go `Get` + `ProviderDefinition`）。
#[async_trait]
pub trait SyncBackend: Send + Sync {
    async fn get_provider(&self, account_id: i64) -> Result<(Provider, QuotaKind), Error>;
    async fn has_billing(&self, account_id: i64) -> Result<bool, Error>;
    async fn refresh_billing(&self, account_id: i64) -> Result<(), Error>;
    async fn has_quota(&self, account_id: i64) -> Result<bool, Error>;
    async fn refresh_quota(&self, account_id: i64) -> Result<(), Error>;
    async fn has_models(&self, account_id: i64) -> Result<bool, Error>;
    async fn sync_models(&self, account_id: i64) -> Result<(), Error>;
}

/// 账号同步服务。
#[derive(Clone)]
pub struct AccountSyncService {
    backend: Arc<dyn SyncBackend>,
    workers: usize,
}

impl AccountSyncService {
    pub fn new(backend: Arc<dyn SyncBackend>) -> Self {
        Self {
            backend,
            workers: DEFAULT_WORKER_COUNT,
        }
    }

    /// 自定义并发 Worker 数（`< 1` 回退到默认；Go `UpdateConcurrency`）。
    pub fn with_workers(backend: Arc<dyn SyncBackend>, workers: usize) -> Self {
        Self {
            backend,
            workers: workers.max(1),
        }
    }

    pub fn update_concurrency(&mut self, value: usize) {
        self.workers = value.max(1);
    }

    /// 等待指定账号完成座位补齐（Go `Sync`）。
    pub async fn sync(&self, ids: &[i64]) -> SyncResult {
        let (tx, rx) = mpsc::unbounded_channel();
        for &id in ids {
            let _ = tx.send(id);
        }
        drop(tx);
        self.sync_stream(rx).await
    }

    /// 以固定 Worker 数消费持续到达的账号流（Go `SyncStream`）。
    pub async fn sync_stream(&self, input: mpsc::UnboundedReceiver<i64>) -> SyncResult {
        self.sync_stream_observed(input, Arc::new(Mutex::new(|_c: usize, _t: usize| {})))
            .await
    }

    /// 每个去重账号完成后回调 `observer(completed, total)`（Go `SyncStreamObserved`）。
    pub async fn sync_stream_observed<F>(
        &self,
        input: mpsc::UnboundedReceiver<i64>,
        observer: Arc<Mutex<F>>,
    ) -> SyncResult
    where
        F: FnMut(usize, usize) + Send + 'static,
    {
        let succeeded = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));

        // 内层工作队列（send loop 写入，Worker 消费）。
        let (jobs_tx, jobs_rx) = mpsc::unbounded_channel::<i64>();
        let shared_rx = Arc::new(tokio::sync::Mutex::new(jobs_rx));

        let mut handles = Vec::with_capacity(self.workers);
        for _ in 0..self.workers {
            let shared_rx = Arc::clone(&shared_rx);
            let backend = Arc::clone(&self.backend);
            let observer = Arc::clone(&observer);
            let succeeded = Arc::clone(&succeeded);
            let failed = Arc::clone(&failed);
            let total = Arc::clone(&total);
            let completed = Arc::clone(&completed);
            handles.push(tokio::spawn(async move {
                loop {
                    let item = shared_rx.lock().await.recv().await;
                    let Some(account_id) = item else {
                        break;
                    };
                    if sync_account(backend.as_ref(), account_id).await.is_ok() {
                        succeeded.fetch_add(1, Ordering::Relaxed);
                    } else {
                        failed.fetch_add(1, Ordering::Relaxed);
                    }
                    completed.fetch_add(1, Ordering::Relaxed);
                    let (c, t) = (
                        completed.load(Ordering::Relaxed),
                        total.load(Ordering::Relaxed),
                    );
                    observer.lock().unwrap()(c, t);
                }
            }));
        }

        // 发送循环：外层 input 去重 → 计数 → 推入工作队列。
        let send_loop = tokio::spawn(async move {
            let mut input = input;
            let mut seen = HashSet::new();
            while let Some(id) = input.recv().await {
                if id == 0 || !seen.insert(id) {
                    continue;
                }
                total.fetch_add(1, Ordering::Relaxed);
                if jobs_tx.send(id).is_err() {
                    break; // 所有 Worker 已退出。
                }
            }
            drop(jobs_tx);
        });
        let _ = send_loop.await;
        for handle in handles {
            let _ = handle.await;
        }
        SyncResult {
            succeeded: succeeded.load(Ordering::Relaxed),
            failed: failed.load(Ordering::Relaxed),
        }
    }

    /// 单个账号的节奏：额度策略决定 billing/quota 二选一 + models 必做；
    /// 三路任一失败都不阻断其余路径（Go `syncAccount`）。
    pub async fn sync_account(&self, account_id: i64) -> Result<(), Error> {
        sync_account(self.backend.as_ref(), account_id).await
    }
}

async fn sync_account(backend: &dyn SyncBackend, account_id: i64) -> Result<(), Error> {
    let (_provider, quota_kind) = backend.get_provider(account_id).await?;
    let mut messages: Vec<String> = Vec::new();

    match quota_kind {
        QuotaKind::RemoteWindow | QuotaKind::LocalWindow => {
            match backend.has_quota(account_id).await {
                Ok(true) => {}
                Ok(false) => {
                    match timeout(OPERATION_TIMEOUT, backend.refresh_quota(account_id)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => messages.push(format!("同步 Provider 额度: {e}")),
                        Err(_) => messages.push("同步 Provider 额度: 超时".into()),
                    }
                }
                Err(e) => messages.push(format!("检查 Provider 额度快照: {e}")),
            }
        }
        QuotaKind::Billing => match backend.has_billing(account_id).await {
            Ok(true) => {}
            Ok(false) => {
                match timeout(OPERATION_TIMEOUT, backend.refresh_billing(account_id)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => messages.push(format!("同步额度: {e}")),
                    Err(_) => messages.push("同步额度: 超时".into()),
                }
            }
            Err(e) => messages.push(format!("检查额度快照: {e}")),
        },
    }

    // 模型同步必做；check 失败时如 Go 直接返回（不再尝试 SyncAccount）。
    match backend.has_models(account_id).await {
        Ok(true) => {}
        Ok(false) => match timeout(OPERATION_TIMEOUT, backend.sync_models(account_id)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => messages.push(format!("同步模型: {e}")),
            Err(_) => messages.push("同步模型: 超时".into()),
        },
        Err(e) => messages.push(format!("检查模型快照: {e}")),
    }

    if messages.is_empty() {
        Ok(())
    } else {
        Err(Error::Backend(messages.join("; ")))
    }
}
