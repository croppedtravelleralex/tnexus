//! G4-P4 后台任务编排器（对齐 Go `internal/app/application.go` 的
//! `runSupervisedTask` / `runPeriodicTask` 与 `settings_change_listener`）。
//!
//! 语义：
//! - 每个任务独立 `tokio` task；每轮执行都包一层 `tokio::task::spawn`，
//!   任务 panic 被捕获（`JoinError`）而不会杀死循环（G4-A4 crash restart）。
//! - 成功一轮后按 `interval` 等待下一轮（`runPeriodicTask`）；
//!   panic / 失败后按指数退避重试（1s → 30s 上限，`runSupervisedTask`）。
//! - [`TaskScheduler::status_snapshot`] 提供每任务运行状态快照。
//! - [`SettingsWatcher`]：设置变更订阅的 poll 版抽象（对齐 Redis pubsub
//!   `ListenSettingsChanges` 的「变更 → 重载回调」语义）；以高频任务注册，
//!   轮询到变更立即触发回调。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::AbortHandle;

use crate::error::{OpsError, OpsResult};

/// 重启退避基值（Go `backoff := time.Second`）。
pub const RESTART_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// 重启退避上限（Go `min(backoff*2, 30*time.Second)`）。
pub const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// 一轮后台任务（可 async、可 panic）。
pub type AsyncRun = dyn Fn() -> Pin<Box<dyn Future<Output = OpsResult<()>> + Send>> + Send + Sync;

/// 设置变更轮询源（对齐 Go `ListenSettingsChanges` 的事件流）。
#[async_trait]
pub trait SettingsWatcher: Send + Sync {
    /// 轮询是否发生了设置变更。`Err` 表示订阅异常（如通道断开），
    /// 由调度器记入 `last_error` 并按退避续跑（Go 同样会在 listen 返回错误后重启）。
    async fn poll_change(&self) -> Result<bool, OpsError>;
}

/// 单任务运行状态（快照值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatus {
    pub name: String,
    pub running: bool,
    pub last_started_at: Option<DateTime<Utc>>,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    /// 执行轮数（含失败轮）。
    pub attempts: u64,
    /// panic 次数（`JoinError`）。
    pub panics: u64,
}

#[derive(Clone)]
struct WatcherSpec {
    interval: Duration,
    watcher: Arc<dyn SettingsWatcher>,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

#[derive(Clone)]
struct TaskSpec {
    name: String,
    interval: Duration,
    run: Option<Arc<AsyncRun>>,
    watcher: Option<WatcherSpec>,
}

#[derive(Debug, Default)]
struct Shared {
    status: HashMap<String, TaskStatus>,
}

/// 后台任务编排器。
#[derive(Default)]
pub struct TaskScheduler {
    tasks: Vec<TaskSpec>,
    shared: Arc<Mutex<Shared>>,
}

impl TaskScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册周期性任务（每轮成功 → 等 `interval`）。
    pub fn add_task(&mut self, name: impl Into<String>, interval: Duration, run: Arc<AsyncRun>) {
        self.tasks.push(TaskSpec {
            name: name.into(),
            interval,
            run: Some(run),
            watcher: None,
        });
    }

    /// 注册设置变更监听：按 `interval` 高频轮询，`poll_change()==true` 时触发
    /// `on_change` 回调（对齐 Go `settings_change_listener` → `ReloadPersisted`）。
    pub fn add_settings_watcher(
        &mut self,
        name: impl Into<String>,
        interval: Duration,
        watcher: Arc<dyn SettingsWatcher>,
        on_change: Arc<dyn Fn() + Send + Sync>,
    ) {
        self.tasks.push(TaskSpec {
            name: name.into(),
            interval: Duration::ZERO, // watcher 用自身 interval
            run: None,
            watcher: Some(WatcherSpec {
                interval,
                watcher,
                on_change,
            }),
        });
    }

    /// 启动全部任务。返回各任务循环的 `AbortHandle`，调用方可 `abort()` 停止。
    /// `&self` 保留调度器以便继续查询状态快照。
    pub fn spawn_all(&self) -> Vec<AbortHandle> {
        let mut handles = Vec::with_capacity(self.tasks.len());
        for spec in self.tasks.clone() {
            let shared = Arc::clone(&self.shared);
            let handle = if let Some(watcher) = spec.watcher.clone() {
                tokio::task::spawn(Self::watcher_loop(spec.name, watcher, shared))
            } else if let Some(run) = spec.run.clone() {
                tokio::task::spawn(Self::task_loop(spec.name, spec.interval, run, shared))
            } else {
                continue;
            };
            handles.push(handle.abort_handle());
        }
        handles
    }

    /// 任务运行状态快照。
    pub fn status_snapshot(&self) -> Vec<TaskStatus> {
        let shared = self.shared.lock().unwrap();
        let mut list: Vec<TaskStatus> = shared.status.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// 单任务状态（不存在返回 None）。
    pub fn task_status(&self, name: &str) -> Option<TaskStatus> {
        self.shared.lock().unwrap().status.get(name).cloned()
    }

    // ── 循环实现 ────────────────────────────────────────────────

    async fn task_loop(
        name: String,
        interval: Duration,
        run: Arc<AsyncRun>,
        shared: Arc<Mutex<Shared>>,
    ) {
        let mut backoff = RESTART_BACKOFF_BASE;
        loop {
            let started_at = Utc::now();
            mark(&shared, &name, |s| {
                s.running = true;
                s.last_started_at = Some(started_at);
                s.attempts += 1;
            });

            // 每轮包 tokio::task::spawn：panic 被 JoinError 捕获，循环不中断。
            let run = Arc::clone(&run);
            let result = tokio::task::spawn(async move { run().await }).await;

            let completed_at = Utc::now();
            match result {
                Ok(Ok(())) => {
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = None;
                    });
                    backoff = RESTART_BACKOFF_BASE;
                    tokio::time::sleep(interval).await;
                }
                Ok(Err(e)) => {
                    tracing::warn!(task = %name, error = %e, "background_task_failed");
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = Some(e.to_string());
                    });
                    sleep_backoff(&shared, &name, &mut backoff).await;
                }
                Err(join) => {
                    tracing::warn!(task = %name, error = %join, "background_task_panicked");
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = Some(format!("panic: {join}"));
                        s.panics += 1;
                    });
                    sleep_backoff(&shared, &name, &mut backoff).await;
                }
            }
        }
    }

    async fn watcher_loop(name: String, watcher: WatcherSpec, shared: Arc<Mutex<Shared>>) {
        let mut backoff = RESTART_BACKOFF_BASE;
        loop {
            let started_at = Utc::now();
            mark(&shared, &name, |s| {
                s.running = true;
                s.last_started_at = Some(started_at);
                s.attempts += 1;
            });

            let watcher_ops = Arc::clone(&watcher.watcher);
            let result = tokio::task::spawn(async move { watcher_ops.poll_change().await }).await;

            let completed_at = Utc::now();
            match result {
                Ok(Ok(true)) => {
                    (watcher.on_change)();
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = None;
                    });
                    backoff = RESTART_BACKOFF_BASE;
                    tokio::time::sleep(watcher.interval).await;
                }
                Ok(Ok(false)) => {
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = None;
                    });
                    backoff = RESTART_BACKOFF_BASE;
                    tokio::time::sleep(watcher.interval).await;
                }
                Ok(Err(e)) => {
                    tracing::warn!(task = %name, error = %e, "settings_watcher_failed");
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = Some(e.to_string());
                    });
                    sleep_backoff(&shared, &name, &mut backoff).await;
                }
                Err(join) => {
                    tracing::warn!(task = %name, error = %join, "settings_watcher_panicked");
                    mark(&shared, &name, |s| {
                        s.running = false;
                        s.last_completed_at = Some(completed_at);
                        s.last_error = Some(format!("panic: {join}"));
                        s.panics += 1;
                    });
                    sleep_backoff(&shared, &name, &mut backoff).await;
                }
            }
        }
    }
}

/// 原子更新某任务状态。
fn mark(shared: &Arc<Mutex<Shared>>, name: &str, f: impl FnOnce(&mut TaskStatus)) {
    let mut shared = shared.lock().unwrap();
    let entry = shared
        .status
        .entry(name.to_string())
        .or_insert_with(|| TaskStatus {
            name: name.to_string(),
            running: false,
            last_started_at: None,
            last_completed_at: None,
            last_error: None,
            attempts: 0,
            panics: 0,
        });
    f(entry);
}

/// 指数退避续跑（对齐 Go `runSupervisedTask` 的 backoff*2 封顶 30s）。
async fn sleep_backoff(_shared: &Arc<Mutex<Shared>>, _name: &str, backoff: &mut Duration) {
    let wait = *backoff;
    *backoff = (*backoff * 2).min(RESTART_BACKOFF_MAX);
    tokio::time::sleep(wait).await;
}
