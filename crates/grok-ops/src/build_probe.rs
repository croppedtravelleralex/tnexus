//! Build 探针监控状态机（对齐 Go `account/build_probe_monitor.go`）。
//!
//! 纯内存状态：`start` / `finish` 记录当前探针、滚动统计与最近结果；`snapshot`
//! 输出 `BuildProbeStatus`。无 IO；池规模汇总由调用方（four_pool）注入。
//!
//! 并发：`std::sync::Mutex` 内省锁（同 Go `sync.RWMutex`）。

use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use grok_domain::Account;
use grok_pool::build_pool::{BuildPool, BuildProbePoolSummary};

/// 最近结果保留上限（Go `maxRecentBuildProbeResults`）。
pub const MAX_RECENT_BUILD_PROBE_RESULTS: usize = 20;

/// 探针车道（Go `BuildProbeMode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProbeMode {
    Verification,
    Normal,
    Delete,
    Dispatch,
}

impl BuildProbeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildProbeMode::Verification => "verification",
            BuildProbeMode::Normal => "normal",
            BuildProbeMode::Delete => "delete",
            BuildProbeMode::Dispatch => "dispatch",
        }
    }
}

/// 探针结果分类（Go `BuildProbeOutcome`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProbeOutcome {
    Verified,
    NormalOk,
    DispatchOk,
    Cooldown,
    Failed,
    Deletable,
    Deleted,
}

impl BuildProbeOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildProbeOutcome::Verified => "verified",
            BuildProbeOutcome::NormalOk => "normalOk",
            BuildProbeOutcome::DispatchOk => "dispatchOk",
            BuildProbeOutcome::Cooldown => "cooldown",
            BuildProbeOutcome::Failed => "failed",
            BuildProbeOutcome::Deletable => "deletable",
            BuildProbeOutcome::Deleted => "deleted",
        }
    }
}

/// 探针失败分类（对齐 Go 哨兵错误 `errPurgeDeletable` / `errPurgeDeleted`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    /// 账号被判可删（`deletable:` 标记）。
    PurgeDeletable,
    /// 账号已删除。
    PurgeDeleted,
    /// 其它失败（普通 / 可重试）。
    Other(String),
}

impl ProbeFailure {
    /// 从错误文本分类（Go `isTerminalBuildError` 之外的哨兵判断由调用方完成）。
    pub fn from_err_text(text: &str) -> ProbeFailure {
        let t = text.trim();
        if t.starts_with(PURGE_DELETABLE_MARK) {
            return ProbeFailure::PurgeDeletable;
        }
        if t.starts_with(PURGE_DELETED_MARK) {
            return ProbeFailure::PurgeDeleted;
        }
        ProbeFailure::Other(t.to_string())
    }
}

/// `deletable:` 哨兵前缀（`errPurgeDeletable` 的文本形态）。
pub const PURGE_DELETABLE_MARK: &str = "purge: deletable";
/// `deleted` 哨兵前缀（`errPurgeDeleted` 的文本形态）。
pub const PURGE_DELETED_MARK: &str = "purge: deleted";

/// 当前探针（Go `BuildProbeCurrent`）。
#[derive(Debug, Clone)]
pub struct BuildProbeCurrent {
    pub account_id: i64,
    pub account_name: String,
    pub mode: BuildProbeMode,
    pub started_at: DateTime<Utc>,
}

/// 单次探针结果（Go `BuildProbeResult`）。
#[derive(Debug, Clone)]
pub struct BuildProbeResult {
    pub account_id: i64,
    pub account_name: String,
    pub mode: BuildProbeMode,
    pub outcome: BuildProbeOutcome,
    pub pool: Option<BuildPool>,
    pub error: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub duration: Duration,
}

/// 滚动统计（Go `BuildProbeStatistics`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildProbeStatistics {
    pub attempts: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub verified: i64,
    pub normal_ok: i64,
    pub dispatch_ok: i64,
    pub cooled_down: i64,
    pub deletable: i64,
    pub deleted: i64,
    pub consecutive_failures: i64,
    pub lane_attempts: BuildProbeLaneAttempts,
}

/// 各车道尝试数（Go `BuildProbeLaneAttempts`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildProbeLaneAttempts {
    pub verification: i64,
    pub normal: i64,
    pub delete: i64,
    pub dispatch: i64,
}

/// 探针监控快照（Go `BuildProbeStatus`）。
#[derive(Debug, Clone)]
pub struct BuildProbeStatus {
    pub enabled: bool,
    pub running: bool,
    pub purge_apply: bool,
    pub interval: Duration,
    pub idle_interval: Duration,
    pub initial_delay: Duration,
    pub started_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub last_error: String,
    pub current: Option<BuildProbeCurrent>,
    pub statistics: BuildProbeStatistics,
    pub pools: BuildProbePoolSummary,
    pub recent: Vec<BuildProbeResult>,
}

#[derive(Debug, Default)]
struct MonitorState {
    enabled: bool,
    purge_apply: bool,
    interval: Duration,
    idle_interval: Duration,
    initial_delay: Duration,
    started_at: Option<DateTime<Utc>>,
    next_run_at: Option<DateTime<Utc>>,
    last_completed_at: Option<DateTime<Utc>>,
    last_error: String,
    current: Option<BuildProbeCurrent>,
    statistics: BuildProbeStatistics,
    recent: Vec<BuildProbeResult>,
}

/// Build 探针监控（Go `buildProbeMonitor`）。
#[derive(Debug, Default)]
pub struct BuildProbeMonitor {
    state: Mutex<MonitorState>,
}

impl BuildProbeMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 配置（Go `configure`）：`interval > 0` 即启用，并设首次运行时刻 = now + initial_delay。
    pub fn configure(
        &self,
        now: DateTime<Utc>,
        interval: Duration,
        idle_interval: Duration,
        initial_delay: Duration,
    ) {
        let mut s = self.state.lock().unwrap();
        s.enabled = interval > Duration::zero();
        s.interval = interval;
        s.idle_interval = idle_interval;
        s.initial_delay = initial_delay;
        if !s.enabled {
            s.next_run_at = None;
            return;
        }
        if s.started_at.is_none() {
            s.started_at = Some(now);
        }
        s.next_run_at = Some(now + initial_delay);
    }

    pub fn set_purge_apply(&self, enabled: bool) {
        self.state.lock().unwrap().purge_apply = enabled;
    }

    pub fn purge_apply_enabled(&self) -> bool {
        self.state.lock().unwrap().purge_apply
    }

    /// 调度下次运行（Go `schedule`）。
    pub fn schedule(&self, next: DateTime<Utc>) {
        let mut s = self.state.lock().unwrap();
        if s.enabled {
            s.next_run_at = Some(next);
        }
    }

    /// 探针开始（Go `start`）。
    pub fn start(&self, candidate: &Account, mode: BuildProbeMode, started_at: DateTime<Utc>) {
        let mut s = self.state.lock().unwrap();
        s.next_run_at = None;
        s.current = Some(BuildProbeCurrent {
            account_id: candidate.id,
            account_name: candidate.identity_key.clone(),
            mode,
            started_at,
        });
    }

    /// 探针结束并记账（Go `finish`）。
    pub fn finish(
        &self,
        candidate: &Account,
        mode: BuildProbeMode,
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
        probe_err: Option<&ProbeFailure>,
    ) {
        let mut s = self.state.lock().unwrap();
        let pool = if matches!(probe_err, Some(ProbeFailure::PurgeDeleted)) {
            Some(BuildPool::Delete)
        } else {
            build_account_pool_at(candidate, completed_at)
        };
        let outcome = build_probe_outcome(mode, pool, probe_err);
        let error_message = probe_err
            .map(|e| match e {
                ProbeFailure::PurgeDeletable => PURGE_DELETABLE_MARK.to_string(),
                ProbeFailure::PurgeDeleted => PURGE_DELETED_MARK.to_string(),
                ProbeFailure::Other(text) => text.trim().to_string(),
            })
            .map(|t| {
                if t.len() > 512 {
                    t[..512].to_string()
                } else {
                    t
                }
            })
            .unwrap_or_default();

        s.statistics.attempts += 1;
        match mode {
            BuildProbeMode::Verification => s.statistics.lane_attempts.verification += 1,
            BuildProbeMode::Normal => s.statistics.lane_attempts.normal += 1,
            BuildProbeMode::Delete => s.statistics.lane_attempts.delete += 1,
            BuildProbeMode::Dispatch => s.statistics.lane_attempts.dispatch += 1,
        }
        // 对齐 Go `finish` 的 switch：PurgeDeleted → deleted；PurgeDeletable 或
        // outcome==Deletable → deletable；成功 → succeeded；其余 → failed（冷却另计）。
        let is_purge_deleted = matches!(probe_err, Some(ProbeFailure::PurgeDeleted));
        let is_purge_deletable = matches!(probe_err, Some(ProbeFailure::PurgeDeletable));
        if is_purge_deleted {
            s.statistics.failed += 1;
            s.statistics.deleted += 1;
            s.statistics.consecutive_failures += 1;
        } else if is_purge_deletable || outcome == BuildProbeOutcome::Deletable {
            s.statistics.failed += 1;
            s.statistics.deletable += 1;
            s.statistics.consecutive_failures += 1;
        } else if probe_err.is_none() {
            s.statistics.succeeded += 1;
            s.statistics.consecutive_failures = 0;
            match mode {
                BuildProbeMode::Verification => s.statistics.verified += 1,
                BuildProbeMode::Normal => s.statistics.normal_ok += 1,
                BuildProbeMode::Dispatch => s.statistics.dispatch_ok += 1,
                BuildProbeMode::Delete => {}
            }
        } else {
            s.statistics.failed += 1;
            s.statistics.consecutive_failures += 1;
            if outcome == BuildProbeOutcome::Cooldown {
                s.statistics.cooled_down += 1;
            }
        }

        let result = BuildProbeResult {
            account_id: candidate.id,
            account_name: candidate.identity_key.clone(),
            mode,
            outcome,
            pool,
            error: error_message.clone(),
            started_at,
            completed_at,
            duration: completed_at - started_at,
        };
        s.recent.insert(0, result);
        s.recent.truncate(MAX_RECENT_BUILD_PROBE_RESULTS);
        s.current = None;
        s.last_completed_at = Some(completed_at);
        s.last_error = error_message;
    }

    /// 快照（Go `snapshot`），`pools` 由调用方注入。
    pub fn snapshot(&self, pools: BuildProbePoolSummary) -> BuildProbeStatus {
        let s = self.state.lock().unwrap();
        BuildProbeStatus {
            enabled: s.enabled,
            running: s.current.is_some(),
            purge_apply: s.purge_apply,
            interval: s.interval,
            idle_interval: s.idle_interval,
            initial_delay: s.initial_delay,
            started_at: s.started_at,
            next_run_at: s.next_run_at,
            last_completed_at: s.last_completed_at,
            last_error: s.last_error.clone(),
            current: s.current.clone(),
            statistics: s.statistics,
            pools,
            recent: s.recent.clone(),
        }
    }
}

fn build_account_pool_at(candidate: &Account, now: DateTime<Utc>) -> Option<BuildPool> {
    grok_pool::build_pool::build_account_pool_at(candidate, now, None)
}

/// 结果分类（Go `buildProbeOutcome`）。
pub fn build_probe_outcome(
    mode: BuildProbeMode,
    pool: Option<BuildPool>,
    probe_err: Option<&ProbeFailure>,
) -> BuildProbeOutcome {
    if matches!(probe_err, Some(ProbeFailure::PurgeDeleted)) {
        return BuildProbeOutcome::Deleted;
    }
    if (matches!(probe_err, Some(ProbeFailure::PurgeDeletable)) || pool == Some(BuildPool::Delete))
        && probe_err.is_some()
    {
        return BuildProbeOutcome::Deletable;
    }
    if probe_err.is_none() {
        return match mode {
            BuildProbeMode::Normal => BuildProbeOutcome::NormalOk,
            BuildProbeMode::Dispatch => BuildProbeOutcome::DispatchOk,
            _ => BuildProbeOutcome::Verified,
        };
    }
    if pool == Some(BuildPool::Normal) {
        return BuildProbeOutcome::Cooldown;
    }
    BuildProbeOutcome::Failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_domain::AuthStatus;

    fn account(id: i64) -> Account {
        Account {
            id,
            identity_key: format!("acc-{id}"),
            provider: grok_domain::Provider::GrokBuild,
            enabled: true,
            auth_status: AuthStatus::Active,
            ..Default::default()
        }
    }

    #[test]
    fn outcome_mapping_matches_go() {
        let now = Utc::now();
        // success
        assert_eq!(
            build_probe_outcome(
                BuildProbeMode::Verification,
                Some(BuildPool::Dispatch),
                None
            ),
            BuildProbeOutcome::Verified
        );
        assert_eq!(
            build_probe_outcome(BuildProbeMode::Normal, Some(BuildPool::Dispatch), None),
            BuildProbeOutcome::NormalOk
        );
        assert_eq!(
            build_probe_outcome(BuildProbeMode::Dispatch, Some(BuildPool::Dispatch), None),
            BuildProbeOutcome::DispatchOk
        );
        // deletable: purge mark or pool=delete with err
        assert_eq!(
            build_probe_outcome(
                BuildProbeMode::Verification,
                Some(BuildPool::Delete),
                Some(&ProbeFailure::PurgeDeletable)
            ),
            BuildProbeOutcome::Deletable
        );
        // delete mode purge err → Deleted (errPurgeDeleted checked first)
        assert_eq!(
            build_probe_outcome(
                BuildProbeMode::Delete,
                Some(BuildPool::Delete),
                Some(&ProbeFailure::PurgeDeleted)
            ),
            BuildProbeOutcome::Deleted
        );
        // generic failure in normal pool → cooldown
        assert_eq!(
            build_probe_outcome(
                BuildProbeMode::Verification,
                Some(BuildPool::Normal),
                Some(&ProbeFailure::Other("boom".into()))
            ),
            BuildProbeOutcome::Cooldown
        );
        assert_eq!(
            build_probe_outcome(
                BuildProbeMode::Verification,
                Some(BuildPool::Verification),
                Some(&ProbeFailure::Other("boom".into()))
            ),
            BuildProbeOutcome::Failed
        );

        // finish(): success path updates stats
        let m = BuildProbeMonitor::new();
        m.configure(
            now,
            Duration::seconds(30),
            Duration::minutes(5),
            Duration::minutes(2),
        );
        let a = account(7);
        m.start(&a, BuildProbeMode::Verification, now);
        let s = m.snapshot(BuildProbePoolSummary::default());
        assert!(s.enabled && s.running);
        assert_eq!(s.current.as_ref().unwrap().account_id, 7);
        assert_eq!(
            s.current.as_ref().unwrap().mode,
            BuildProbeMode::Verification
        );
        assert_eq!(s.next_run_at, None, "running clears next_run_at");

        m.finish(
            &a,
            BuildProbeMode::Verification,
            now,
            now + Duration::seconds(1),
            None,
        );
        let s = m.snapshot(BuildProbePoolSummary::default());
        assert!(!s.running);
        assert_eq!(s.statistics.attempts, 1);
        assert_eq!(s.statistics.succeeded, 1);
        assert_eq!(s.statistics.verified, 1);
        assert_eq!(s.statistics.lane_attempts.verification, 1);
        assert_eq!(s.recent.len(), 1);
        assert_eq!(s.recent[0].outcome, BuildProbeOutcome::Verified);
        assert_eq!(s.last_completed_at, Some(now + Duration::seconds(1)));
    }
}
