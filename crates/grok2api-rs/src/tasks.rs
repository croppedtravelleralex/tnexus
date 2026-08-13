//! :8000 后台任务接线（对齐 Go `internal/app/application.go` 的 22 后台任务启动语义）。
//!
//! 现状（2026-08-06）：
//! 现状（2026-08-06）：
//!
//! **已接线**：`build_four_pool_probe`（Build 四池维护探针，`grok-ops::four_pool`
//! 配合 `pg_ops::PgBuildProbeOps`，真实 PG 数据；上游探测走
//! [`BuildProbeTransport`]，默认 [`NotWiredTransport`] 返回「未接线」，探测本身
//! 不影响调度索引/冷却记账）。
//!
//! **已接线**：`build_four_pool_probe`；`web_quota_refresh`（直连 `/rest/rate-limits` → PG）；
//! `grok_web_nurture`（`GROK_NURTURE_ENABLED=1`）。
//!
//! **TODO（需 Go sidecar）**：`web_dispatch_probe`（Web 池探针）、`pin_sync`。
//!
//! 开关：`GROK_TASKS_ENABLED=1`（缺省关）；interval 可配：
//! `GROK_PROBE_INTERVAL_MS`（缺省 30s）、`GROK_QUOTA_INTERVAL_MS`（60s）、
//! `GROK_PIN_INTERVAL_MS`（120s）。
//!
//! 无 `GROK_DATABASE_URL` 时（`pg_ops` 无池可建）不启动并日志提示。
//! 所有任务经 [`TaskScheduler`]（G4-P4）包装：单轮 panic 被捕获、指数退避续跑
//! （G4-A4 crash restart 语义），状态可查 `status_snapshot`。

use std::sync::Arc;
use std::time::Duration;

use grok_ops::four_pool::BuildFourPool;
use grok_ops::pg_ops::{NotWiredTransport, PgBuildProbeOps};
use grok_ops::scheduler::{AsyncRun, TaskScheduler};
use grok_pool::SimplifiedPool;
use grok_storage::repo::account::PgAccountRepository;
use tokio::task::AbortHandle;

use grok_domain::WebLane;
use grok_ops::probe::WebDispatchProbe;

use crate::grok_nurture_ops::GrokNurtureService;
use crate::web_nurture;
use crate::web_quota::WebQuotaService;

/// 后台任务配置（env 驱动）。
#[derive(Debug, Clone)]
pub struct TaskConfig {
    /// `GROK_TASKS_ENABLED=1` 时启动。
    pub enabled: bool,
    /// Build 四池维护探针 interval（默认 30s）。
    pub probe_interval: Duration,
    /// quota 刷新 interval（默认 60s；未接线，仅保留语义）。
    pub quota_interval: Duration,
    /// pin 同步 interval（默认 120s；未接线，仅保留语义）。
    pub pin_interval: Duration,
    /// 号池与 PG 增量对账 interval（默认 60s）。
    pub pool_reconcile_interval: Duration,
}

impl TaskConfig {
    /// 从 env 读取。
    pub fn from_env() -> Self {
        let enabled = std::env::var("GROK_TASKS_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let ms = |key: &str, default: u64| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default)
        };
        Self {
            enabled,
            probe_interval: Duration::from_millis(ms("GROK_PROBE_INTERVAL_MS", 30_000)),
            quota_interval: Duration::from_millis(ms("GROK_QUOTA_INTERVAL_MS", 60_000)),
            pin_interval: Duration::from_millis(ms("GROK_PIN_INTERVAL_MS", 120_000)),
            pool_reconcile_interval: Duration::from_millis(ms(
                "GROK_POOL_RECONCILE_INTERVAL_MS",
                60_000,
            )),
        }
    }

    /// 直接构造（测试/程序化接线用）。
    #[cfg(test)]
    pub fn new(enabled: bool, probe_interval: Duration) -> Self {
        Self {
            enabled,
            probe_interval,
            quota_interval: Duration::from_secs(60),
            pin_interval: Duration::from_secs(120),
            pool_reconcile_interval: Duration::from_secs(60),
        }
    }
}

/// 后台任务集合（scheduler + abort handles）。
pub struct BackgroundTasks {
    pub scheduler: TaskScheduler,
    handles: Vec<AbortHandle>,
}

impl BackgroundTasks {
    pub fn empty() -> Self {
        Self {
            scheduler: TaskScheduler::new(),
            handles: Vec::new(),
        }
    }

    /// 运行中任务状态快照（透传 scheduler）。
    pub fn status_snapshot(&self) -> Vec<grok_ops::scheduler::TaskStatus> {
        self.scheduler.status_snapshot()
    }
}

impl Drop for BackgroundTasks {
    fn drop(&mut self) {
        // 进程退出 / 重建时中止任务循环，避免孤儿任务。
        for handle in self.handles.drain(..) {
            handle.abort();
        }
    }
}

/// 构造 Build 四池探针（真实 PG 号池 + 未接线上游探测）。
pub fn build_four_pool(cfg: &TaskConfig, repo: PgAccountRepository) -> Option<Arc<BuildFourPool>> {
    if !cfg.enabled {
        return None;
    }
    // 上游探测未接线（NotWiredTransport）：索引/冷却记账不受影响，探测结果按失败走冷却。
    let ops = PgBuildProbeOps::with_transport(Arc::new(repo), Arc::new(NotWiredTransport));
    let four_pool = Arc::new(BuildFourPool::new(Arc::new(ops)));
    let (interval, idle) = intervals(&cfg.probe_interval);
    four_pool.configure(interval, idle, interval);
    Some(four_pool)
}

/// 号池增量对账任务的依赖。
///
/// 内存号池只在启动时 `load` 一次，PG 里的启停与冷却传不到选号器；
/// 这个任务负责把两边拉齐（见 `SimplifiedPool::reconcile`）。
pub struct PoolReconcile {
    pub pool: Arc<SimplifiedPool>,
    pub repo: Arc<PgAccountRepository>,
}

/// 注册后台任务到 scheduler。
///
/// - `four_pool` 为 None（无 DB / 未启用）时不注册任何任务。
/// - quota / pin / web probe：无 PG 后端实现（需 Go sidecar），仅日志提示（TODO）。
pub fn register_tasks(
    scheduler: &mut TaskScheduler,
    cfg: &TaskConfig,
    four_pool: Option<Arc<BuildFourPool>>,
    quota: Option<Arc<WebQuotaService>>,
    nurture: Option<Arc<GrokNurtureService>>,
    web_probe: Option<Arc<WebDispatchProbe>>,
    pool_reconcile: Option<PoolReconcile>,
) {
    if !cfg.enabled {
        tracing::info!("GROK_TASKS_ENABLED 未设置：后台任务未注册");
        return;
    }
    let Some(four_pool) = four_pool else {
        tracing::warn!(
            "GROK_DATABASE_URL 不可用：后台任务未启动（build_four_pool_probe 需 PG 号池）"
        );
        return;
    };

    scheduler.add_task(
        "build_four_pool_probe",
        cfg.probe_interval,
        probe_run(four_pool),
    );

    if let Some(PoolReconcile { pool, repo }) = pool_reconcile {
        scheduler.add_task("grok_pool_reconcile", cfg.pool_reconcile_interval, {
            Arc::new(move || {
                let pool = pool.clone();
                let repo = repo.clone();
                Box::pin(async move {
                    match pool.reconcile(repo.as_ref()).await {
                        Ok(report) => {
                            if report.changed() {
                                tracing::info!(
                                    "grok_pool_reconcile total={} added={} removed={} pg_cooldowns={}",
                                    report.total,
                                    report.added,
                                    report.removed,
                                    report.cooldowns_applied
                                );
                            }
                            Ok(())
                        }
                        Err(e) => Err(grok_ops::OpsError::Probe(e.to_string())),
                    }
                })
            })
        });
        tracing::info!(
            "grok_pool_reconcile enabled interval={:?}",
            cfg.pool_reconcile_interval
        );
    } else {
        tracing::warn!("grok_pool_reconcile 未接线：号池启停与 PG 冷却不会进入选号器");
    }

    if web_nurture::nurture_enabled() {
        let base = std::env::var("GROK_NURTURE_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8000".into());
        let auth = std::env::var("GROK_GATEWAY_AUTH_KEY")
            .ok()
            .or_else(|| std::env::var("GATEWAY_AUTH_KEY").ok());
        let client = reqwest::Client::new();
        let interval = web_nurture::nurture_interval();
        scheduler.add_task("grok_web_nurture", interval, {
            Arc::new(move || {
                let client = client.clone();
                let base = base.clone();
                let auth = auth.clone();
                Box::pin(async move {
                    web_nurture::run_once(&client, &base, auth.as_deref())
                        .await
                        .map_err(|e| grok_ops::OpsError::Probe(e.to_string()))
                })
            })
        });
        tracing::info!("grok_web_nurture enabled interval={interval:?}");
    }

    if let Some(quota_svc) = quota {
        let batch_limit = std::env::var("GROK_QUOTA_BATCH_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(32);
        let svc = quota_svc.clone();
        scheduler.add_task("web_quota_refresh", cfg.quota_interval, {
            Arc::new(move || {
                let svc = svc.clone();
                Box::pin(async move {
                    let (ok, fail) = svc.refresh_enabled_batch(batch_limit).await;
                    if ok > 0 || fail > 0 {
                        tracing::info!("web_quota_refresh round ok={ok} fail={fail}");
                    }
                    Ok(())
                })
            })
        });
        tracing::info!(
            "web_quota_refresh enabled interval={:?} batch={batch_limit}",
            cfg.quota_interval
        );
    } else {
        tracing::warn!(
            "web_quota_refresh({:?}) 未接线：需 GROK2API_DIRECT + GROK_CREDENTIAL_KEY",
            cfg.quota_interval
        );
    }

    if let Some(probe) = web_probe {
        let probe = probe.clone();
        scheduler.add_task("web_dispatch_probe", cfg.probe_interval, {
            Arc::new(move || {
                let probe = probe.clone();
                Box::pin(async move {
                    let n = probe
                        .run_once(WebLane::Image)
                        .await
                        .map_err(|e| grok_ops::OpsError::Probe(e.to_string()))?;
                    if n > 0 {
                        tracing::debug!("web_dispatch_probe probed {n} accounts");
                    }
                    Ok(())
                })
            })
        });
        tracing::info!(
            "web_dispatch_probe enabled interval={:?}",
            cfg.probe_interval
        );
    } else {
        tracing::warn!(
            "web_dispatch_probe({:?}) 未接线：需 GROK2API_DIRECT + GROK_CREDENTIAL_KEY + keys",
            cfg.probe_interval
        );
    }

    if let Some(svc) = nurture {
        let nurture_interval = Duration::from_secs(
            std::env::var("GROK_NURTURE_QUEUE_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
        );
        let svc = svc.clone();
        scheduler.add_task("grok_nurture_queue", nurture_interval, {
            Arc::new(move || {
                let svc = svc.clone();
                Box::pin(async move {
                    if !svc.ops.is_running() {
                        return Ok(());
                    }
                    if let Some(job) = svc.ops.pop_job() {
                        match svc.process_job(&job).await {
                            Ok(v) => tracing::info!(
                                account_id = job.account_id,
                                result = %v,
                                "nurture queue job ok"
                            ),
                            Err(e) => tracing::warn!(
                                account_id = job.account_id,
                                error = %e,
                                "nurture queue job failed"
                            ),
                        }
                    }
                    Ok(())
                })
            })
        });
        tracing::info!("grok_nurture_queue enabled interval={nurture_interval:?}");
    }

    tracing::warn!(
        "pin_sync({:?}) 未接线：待 Go sidecar（G6）",
        cfg.pin_interval
    );
}

/// 单轮 Build 四池维护探针的调度任务闭包（经 scheduler 包装，panic 自动续跑）。
fn probe_run(four_pool: Arc<BuildFourPool>) -> Arc<AsyncRun> {
    Arc::new(move || {
        let four_pool = Arc::clone(&four_pool);
        Box::pin(async move {
            // maintenance_tick 返回 OpsResult<TickResult>：无候选时是 Ok(none)，不报错。
            four_pool.maintenance_tick().await.map(|_| ())
        })
    })
}

/// 启动后台任务（`GROK_TASKS_ENABLED=1` 且 DB 就绪时）。
pub fn spawn_background_tasks(
    cfg: &TaskConfig,
    repo: PgAccountRepository,
    quota: Option<Arc<WebQuotaService>>,
    nurture: Option<Arc<GrokNurtureService>>,
    web_probe: Option<Arc<WebDispatchProbe>>,
    pool_reconcile: Option<PoolReconcile>,
) -> BackgroundTasks {
    let mut scheduler = TaskScheduler::new();
    let four_pool = build_four_pool(cfg, repo);
    register_tasks(
        &mut scheduler,
        cfg,
        four_pool,
        quota,
        nurture,
        web_probe,
        pool_reconcile,
    );
    let handles = scheduler.spawn_all();
    tracing::info!(
        "后台任务已启动: {}",
        scheduler
            .status_snapshot()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    BackgroundTasks { scheduler, handles }
}

/// 把 std interval 换算为 four_pool monitor 的 (interval, idle_interval)。
/// idle（无候选时）用 3×interval；interval 下限 1s 防高频轮询空转。
fn intervals(probe_interval: &Duration) -> (chrono::Duration, chrono::Duration) {
    let base = (*probe_interval).max(Duration::from_secs(1));
    let interval =
        chrono::Duration::from_std(base).unwrap_or_else(|_| chrono::Duration::seconds(30));
    let idle = chrono::Duration::from_std(base.saturating_mul(3))
        .unwrap_or_else(|_| chrono::Duration::seconds(90));
    (interval, idle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicI64, Ordering};

    use async_trait::async_trait;
    use grok_domain::{Account, Billing, QuotaRecovery};
    use grok_ops::four_pool::BuildProbeOps;
    use grok_ops::OpsResult;

    // ── 计数 fake（仅记 list_build_accounts 调用次数，其余默认）──
    struct CountingOps {
        list_calls: AtomicI64,
    }

    #[async_trait]
    impl BuildProbeOps for CountingOps {
        async fn get_account(&self, _id: i64) -> OpsResult<Option<Account>> {
            Ok(None)
        }
        async fn list_build_accounts(
            &self,
            _now: chrono::DateTime<chrono::Utc>,
        ) -> OpsResult<Vec<Account>> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
        async fn recoveries_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, QuotaRecovery>> {
            Ok(HashMap::new())
        }
        async fn billings_for(&self, _ids: &[i64]) -> OpsResult<HashMap<i64, Billing>> {
            Ok(HashMap::new())
        }
        async fn get_recovery(&self, _id: i64) -> OpsResult<Option<QuotaRecovery>> {
            Ok(None)
        }
        async fn get_billing(&self, _id: i64) -> OpsResult<Option<Billing>> {
            Ok(None)
        }
        async fn prepare_credential(
            &self,
            account: &Account,
            _refresh_tokens: bool,
        ) -> OpsResult<Account> {
            Ok(account.clone())
        }
        async fn refresh_billing(&self, _id: i64) -> OpsResult<()> {
            Ok(())
        }
        async fn probe_chat_credential(&self, _account: &Account) -> Result<String, String> {
            Err("not wired".into())
        }
        async fn probe_chat_capability(&self, _account: &Account) -> Result<String, String> {
            Err("not wired".into())
        }
        async fn observe_model(&self, _id: i64, _model: &str) -> OpsResult<()> {
            Ok(())
        }
        async fn update_health(
            &self,
            _id: i64,
            _failure_count: i32,
            _cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
            _reason: &str,
            _reset_last_success: bool,
        ) -> OpsResult<()> {
            Ok(())
        }
        async fn mark_deletable(&self, _id: i64, _reason: &str) -> OpsResult<()> {
            Ok(())
        }
        async fn clear_recovery(&self, _id: i64) -> OpsResult<()> {
            Ok(())
        }
        async fn delete_account(&self, _id: i64) -> OpsResult<()> {
            Ok(())
        }
    }

    fn fake_four_pool() -> Arc<BuildFourPool> {
        let ops = Arc::new(CountingOps {
            list_calls: AtomicI64::new(0),
        });
        Arc::new(BuildFourPool::new(ops))
    }

    // ── TaskConfig ──────────────────────────────────────────────

    #[test]
    fn task_config_from_env_defaults() {
        // 干净 env：disabled + 默认 interval。
        std::env::remove_var("GROK_TASKS_ENABLED");
        std::env::remove_var("GROK_PROBE_INTERVAL_MS");
        let cfg = TaskConfig::from_env();
        assert!(!cfg.enabled);
        assert_eq!(cfg.probe_interval, Duration::from_secs(30));
    }

    #[test]
    fn task_config_from_env_overrides() {
        std::env::set_var("GROK_TASKS_ENABLED", "1");
        std::env::set_var("GROK_PROBE_INTERVAL_MS", "1234");
        let cfg = TaskConfig::from_env();
        assert!(cfg.enabled);
        assert_eq!(cfg.probe_interval, Duration::from_millis(1234));
        std::env::remove_var("GROK_TASKS_ENABLED");
        std::env::remove_var("GROK_PROBE_INTERVAL_MS");
    }

    // ── register_tasks ──────────────────────────────────────────

    #[test]
    fn register_disabled_registers_nothing() {
        let mut scheduler = TaskScheduler::new();
        let cfg = TaskConfig::new(false, Duration::from_secs(10));
        register_tasks(
            &mut scheduler,
            &cfg,
            Some(fake_four_pool()),
            None,
            None,
            None,
            None,
        );
        assert!(scheduler.status_snapshot().is_empty());
    }

    #[test]
    fn register_without_db_registers_nothing() {
        let mut scheduler = TaskScheduler::new();
        let cfg = TaskConfig::new(true, Duration::from_secs(10));
        register_tasks(&mut scheduler, &cfg, None, None, None, None, None);
        assert!(scheduler.status_snapshot().is_empty());
    }

    #[tokio::test]
    async fn register_enabled_with_pool_registers_probe_task() {
        let mut scheduler = TaskScheduler::new();
        let cfg = TaskConfig::new(true, Duration::from_secs(10));
        register_tasks(
            &mut scheduler,
            &cfg,
            Some(fake_four_pool()),
            None,
            None,
            None,
            None,
        );
        let handles = scheduler.spawn_all();
        // status 由 task_loop 首轮 mark 填充，等一小会再快照。
        tokio::time::sleep(Duration::from_millis(100)).await;
        for h in &handles {
            h.abort();
        }
        let snapshot = scheduler.status_snapshot();
        let names: Vec<&str> = snapshot.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["build_four_pool_probe"]);
    }

    /// 接线回归：给了对账依赖就必须注册 `grok_pool_reconcile`。
    /// 少了它，管理台的账号启停在容器重启前不会生效。
    #[tokio::test]
    async fn register_wires_pool_reconcile_task() {
        let mut scheduler = TaskScheduler::new();
        let cfg = TaskConfig::new(true, Duration::from_secs(10));
        // lazy 连接：注册阶段不实际连库。
        let pg = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://u:p@127.0.0.1:1/db")
            .expect("lazy pool");
        register_tasks(
            &mut scheduler,
            &cfg,
            Some(fake_four_pool()),
            None,
            None,
            None,
            Some(PoolReconcile {
                pool: Arc::new(SimplifiedPool::new()),
                repo: Arc::new(PgAccountRepository::new(pg)),
            }),
        );
        let handles = scheduler.spawn_all();
        tokio::time::sleep(Duration::from_millis(100)).await;
        for h in &handles {
            h.abort();
        }
        let names: Vec<String> = scheduler
            .status_snapshot()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "grok_pool_reconcile"),
            "对账任务未注册: {names:?}"
        );
    }

    // ── spawn 后 run_once 被调 ──────────────────────────────────

    #[tokio::test]
    async fn spawn_runs_probe_task() {
        let ops = Arc::new(CountingOps {
            list_calls: AtomicI64::new(0),
        });
        let four_pool = Arc::new(BuildFourPool::new(ops.clone()));
        let mut scheduler = TaskScheduler::new();
        let cfg = TaskConfig::new(true, Duration::from_millis(20));
        register_tasks(&mut scheduler, &cfg, Some(four_pool), None, None, None, None);
        let handles = scheduler.spawn_all();

        // 等至少一轮：maintenance_tick 调 list_build_accounts（fake 计数）。
        tokio::time::sleep(Duration::from_millis(120)).await;
        for h in &handles {
            h.abort();
        }
        let calls = ops.list_calls.load(Ordering::SeqCst);
        assert!(calls >= 1, "probe task should have run, calls = {calls}");
        let status = scheduler.task_status("build_four_pool_probe");
        assert!(status.is_some());
        let status = status.unwrap();
        assert!(status.attempts >= 1, "attempts = {}", status.attempts);
        assert_eq!(status.panics, 0, "empty pool tick should not panic");
    }

    // ── panic 任务被恢复 ────────────────────────────────────────

    #[tokio::test]
    async fn panic_task_recovers() {
        let mut scheduler = TaskScheduler::new();
        let attempts = Arc::new(AtomicI64::new(0));
        let attempts2 = Arc::clone(&attempts);
        // 第 1 轮 panic；之后正常。验证 scheduler 捕获 panic 并续跑（G4-A4）。
        let run: Arc<AsyncRun> = Arc::new(move || {
            let attempts = Arc::clone(&attempts2);
            Box::pin(async move {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    panic!("boom on first round");
                }
                Ok(())
            })
        });
        scheduler.add_task("panic_probe", Duration::from_millis(10), run);
        let handles = scheduler.spawn_all();

        // panic 后 backoff=1s，等 1.2s 保证第二次 run 发生。
        tokio::time::sleep(Duration::from_millis(1200)).await;
        for h in &handles {
            h.abort();
        }
        let status = scheduler.task_status("panic_probe").unwrap();
        assert!(status.panics >= 1, "panics = {}", status.panics);
        assert!(
            status.attempts >= 2,
            "recovered and kept running: {}",
            status.attempts
        );
        let n = attempts.load(Ordering::SeqCst);
        assert!(n >= 2, "run invoked after panic: {n}");
    }
}
