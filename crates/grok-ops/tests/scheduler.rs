//! G4-P4 任务编排器集成测试（对齐 Go `application.go` 的 supervised/periodic 语义）。
//!
//! 覆盖：
//! - 正常任务按 interval 续跑（attempts 递增）
//! - 故意 panic 的任务：panic 被捕获后按退避续跑（G4-A4 crash restart）
//! - 返回 Err 的任务：记 last_error 并按退避续跑
//! - 状态快照字段（running/last_started_at/last_error/panics）
//! - SettingsWatcher 变更轮询 → on_change 回调触发

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use grok_ops::scheduler::{SettingsWatcher, TaskScheduler};

/// 构造一个计数器任务：每次运行把 count+1，可注入 panic/错误行为。
fn counter_task(
    count: Arc<AtomicU64>,
    panic_first_n: u64,
    fail_first_n: u64,
) -> Arc<grok_ops::scheduler::AsyncRun> {
    Arc::new(move || {
        let count = Arc::clone(&count);
        Box::pin(async move {
            let n = count.fetch_add(1, Ordering::SeqCst);
            if n < panic_first_n {
                panic!("boom {n}");
            }
            if n < panic_first_n + fail_first_n {
                return Err(grok_ops::error::OpsError::Quota(format!("fail {n}")));
            }
            Ok(())
        })
    })
}

#[tokio::test]
async fn panicking_task_restarts_and_keeps_running() {
    let count = Arc::new(AtomicU64::new(0));
    let mut scheduler = TaskScheduler::new();
    // panic 一次后成功；interval 很小保证多轮
    scheduler.add_task(
        "panic-task",
        Duration::from_millis(20),
        counter_task(Arc::clone(&count), 1, 0),
    );
    let handles = scheduler.spawn_all();

    // 等待：第一轮 panic（退避 1s）后第二轮成功。退避 1s 较长，等 attempts>=2 且 panics>=1。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = scheduler.task_status("panic-task") {
            if status.attempts >= 2 && status.panics >= 1 && status.last_error.is_none() {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "task did not restart: {:?}",
            scheduler.task_status("panic-task")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let status = scheduler.task_status("panic-task").unwrap();
    assert_eq!(status.panics, 1, "one panic captured");
    assert!(status.attempts >= 2, "restarted after panic");
    assert_eq!(count.load(Ordering::SeqCst), status.attempts);

    for h in handles {
        h.abort();
    }
    // 给 abort 一点时间落地，避免测试 runtime 退出时仍有活动 task。
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test]
async fn failing_task_records_error_and_restarts() {
    let count = Arc::new(AtomicU64::new(0));
    let mut scheduler = TaskScheduler::new();
    scheduler.add_task(
        "failing-task",
        Duration::from_millis(20),
        counter_task(Arc::clone(&count), 0, 2), // 前两轮返回 Err
    );
    let handles = scheduler.spawn_all();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = scheduler.task_status("failing-task") {
            if status.attempts >= 3 && status.panics == 0 && status.last_error.is_none() {
                break;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "failing task did not recover: {:?}",
            scheduler.task_status("failing-task")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let status = scheduler.task_status("failing-task").unwrap();
    assert!(status.attempts >= 3, "restarted after failures");
    assert_eq!(status.panics, 0);

    for h in handles {
        h.abort();
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
}

#[tokio::test]
async fn status_snapshot_lists_all_tasks_sorted() {
    let count = Arc::new(AtomicU64::new(0));
    let mut scheduler = TaskScheduler::new();
    scheduler.add_task(
        "beta",
        Duration::from_millis(5),
        counter_task(Arc::clone(&count), 0, 0),
    );
    scheduler.add_task(
        "alpha",
        Duration::from_millis(5),
        counter_task(Arc::clone(&count), 0, 0),
    );
    let handles = scheduler.spawn_all();

    tokio::time::sleep(Duration::from_millis(60)).await;

    let snapshot = scheduler.status_snapshot();
    let names: Vec<&str> = snapshot.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"], "sorted by name");
    for status in &snapshot {
        assert!(status.attempts >= 1, "each task ran at least once");
        assert!(status.last_started_at.is_some());
        assert!(status.last_completed_at.is_some());
        assert!(status.last_error.is_none());
    }

    for h in handles {
        h.abort();
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
}

// ── SettingsWatcher ───────────────────────────────────────────────

struct FakeWatcher {
    /// 队列：每次 poll 弹一个；Some(true) 表示该次轮询发现变更。
    pending: Arc<Mutex<std::collections::VecDeque<bool>>>,
    poll_calls: Arc<AtomicU64>,
}

impl FakeWatcher {
    fn new(pending: Vec<bool>) -> Self {
        Self {
            pending: Arc::new(Mutex::new(pending.into_iter().collect())),
            poll_calls: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait]
impl SettingsWatcher for FakeWatcher {
    async fn poll_change(&self) -> Result<bool, grok_ops::error::OpsError> {
        self.poll_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.pending.lock().unwrap().pop_front().unwrap_or(false))
    }
}

#[tokio::test]
async fn settings_watcher_triggers_change_callback() {
    let changes = Arc::new(AtomicU64::new(0));
    let watcher = Arc::new(FakeWatcher::new(vec![false, true, true, false, true]));
    let poll_calls = Arc::clone(&watcher.poll_calls);

    let mut scheduler = TaskScheduler::new();
    let on_change = {
        let changes = Arc::clone(&changes);
        Arc::new(move || {
            changes.fetch_add(1, Ordering::SeqCst);
        }) as Arc<dyn Fn() + Send + Sync>
    };
    scheduler.add_settings_watcher(
        "settings-listener",
        Duration::from_millis(10),
        watcher as Arc<dyn SettingsWatcher>,
        on_change,
    );
    let handles = scheduler.spawn_all();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if changes.load(Ordering::SeqCst) >= 3 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "change callback not triggered: {} change(s), {} poll(s)",
            changes.load(Ordering::SeqCst),
            poll_calls.load(Ordering::SeqCst)
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let status = scheduler.task_status("settings-listener").unwrap();
    assert!(
        status.attempts >= 5,
        "watcher polled at least 5 times: {status:?}"
    );
    assert_eq!(
        changes.load(Ordering::SeqCst),
        3,
        "three true polls triggered callback"
    );

    for h in handles {
        h.abort();
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
}
