//! Build 四池选择与热路径索引（对齐 Go `account/four_pool_probe.go`）。
//!
//! 纯函数 + 索引结构，无 IO：
//! - [`build_account_pool_at`] ← Go `AccountPoolAt`：dispatch / normal / verification / delete
//! - [`BuildPoolIndex`] ← Go `dispatchIndex` + `verifyHeap` + `normalHeap` + `deleteHeap` +
//!   `dispatchProbeHeap` + `maintenanceDRR`（`RebuildBuildPoolIndex` / `indexAccountLocked`）
//! - [`is_terminal_build_error`] ← Go `isTerminalBuildError`
//! - [`summarize_build_probe_pools`] ← Go `summarizeBuildProbePools`
//!
//! IO（账号列表 / recovery / billing 拉取）由调用方（grok-ops）负责。

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use grok_domain::{Account, Billing, QuotaRecovery, QuotaRecoveryStatus};

use crate::poolindex::{DRRScheduler, DispatchEntry, DispatchIndex, DueHeap, Lane};

/// 四池池名（对齐 Go `PoolDispatch` 等）。
pub const POOL_DISPATCH: &str = "dispatch";
pub const POOL_NORMAL: &str = "normal";
pub const POOL_VERIFICATION: &str = "verification";
pub const POOL_DELETE: &str = "delete";

/// 可删标记前缀（`deletable:`）；`retired:` 前缀同样进 delete 池。
pub const DELETABLE_PREFIX: &str = "deletable:";

/// Build 四池之一。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildPool {
    Dispatch,
    Normal,
    Verification,
    Delete,
}

impl BuildPool {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildPool::Dispatch => POOL_DISPATCH,
            BuildPool::Normal => POOL_NORMAL,
            BuildPool::Verification => POOL_VERIFICATION,
            BuildPool::Delete => POOL_DELETE,
        }
    }
}

/// Build 账号归属四池（对齐 Go `AccountPoolAt`）。
///
/// 手动禁用（无 `deletable:`/`retired:` 前缀）返回 `None`，不进四池索引。
pub fn build_account_pool_at(
    account: &Account,
    now: DateTime<Utc>,
    recovery: Option<&QuotaRecovery>,
) -> Option<BuildPool> {
    let err_text = account
        .last_error
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if err_text.starts_with(DELETABLE_PREFIX) || err_text.starts_with("retired:") {
        return Some(BuildPool::Delete);
    }
    if !account.enabled {
        return None;
    }
    if account.auth_status == grok_domain::AuthStatus::ReauthRequired {
        return Some(BuildPool::Delete);
    }
    if account.provider == grok_domain::Provider::GrokBuild
        && account
            .observed_model
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Some(BuildPool::Verification);
    }
    if let Some(recovery) = recovery {
        if recovery.status == QuotaRecoveryStatus::Exhausted
            || recovery.status == QuotaRecoveryStatus::Probing
        {
            return Some(BuildPool::Normal);
        }
    }
    if account.cooldown_until.is_some_and(|until| until > now) {
        return Some(BuildPool::Normal);
    }
    Some(BuildPool::Dispatch)
}

/// 终端错误（对齐 Go `isTerminalBuildError`）：命中即判账号可删。
pub fn is_terminal_build_error(reason: &str) -> bool {
    let text = reason.to_ascii_lowercase();
    text.contains("invalid_grant")
        || text.contains("access denied")
        || text.contains("permission-denied")
        || text.contains("permission_denied")
        || text.contains("requires a fresh rt")
        || text.contains("credential rejected")
}

/// Build 四池热路径索引（对齐 Go `RebuildBuildPoolIndex` / `indexAccountLocked` 的字段组）。
#[derive(Default)]
pub struct BuildPoolIndex {
    verify_heap: DueHeap,
    normal_heap: DueHeap,
    delete_heap: DueHeap,
    dispatch_probe_heap: DueHeap,
    dispatch_index: DispatchIndex,
    maintenance_drr: DRRScheduler,
}

impl BuildPoolIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.verify_heap.len()
            + self.normal_heap.len()
            + self.delete_heap.len()
            + self.dispatch_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 从账号列表全量重建索引（对齐 Go `RebuildBuildPoolIndex` 内层循环）。
    pub fn rebuild(
        &mut self,
        accounts: &[Account],
        recoveries: &HashMap<i64, QuotaRecovery>,
        billings: &HashMap<i64, Billing>,
        now: DateTime<Utc>,
    ) {
        *self = Self::new();
        for account in accounts {
            self.sync_account(
                account,
                recoveries.get(&account.id),
                billings.get(&account.id),
                now,
            );
        }
    }

    /// 移除账号（对齐 `indexAccountLocked` 的前置 Remove 组）。
    pub fn remove(&mut self, id: i64) {
        self.verify_heap.remove(id as u64);
        self.normal_heap.remove(id as u64);
        self.delete_heap.remove(id as u64);
        self.dispatch_probe_heap.remove(id as u64);
        self.dispatch_index.remove(id as u64);
    }

    /// 单个账号入池（对齐 Go `indexAccountLocked`）。
    pub fn sync_account(
        &mut self,
        account: &Account,
        recovery: Option<&QuotaRecovery>,
        billing: Option<&Billing>,
        now: DateTime<Utc>,
    ) {
        self.remove(account.id);
        let Some(pool) = build_account_pool_at(account, now, recovery) else {
            return;
        };
        let mut due = now;
        if let Some(cooldown) = account.cooldown_until {
            if cooldown > now {
                due = cooldown;
            }
        }
        match pool {
            BuildPool::Verification => {
                self.verify_heap
                    .upsert(account.id as u64, account.created_at.unwrap_or(now));
            }
            BuildPool::Normal => {
                if let Some(recovery) = recovery {
                    if let Some(next_probe) = recovery.next_probe_at {
                        if next_probe > due {
                            due = next_probe;
                        }
                    }
                }
                self.normal_heap.upsert(account.id as u64, due);
            }
            BuildPool::Delete => {
                self.delete_heap
                    .upsert(account.id as u64, account.updated_at.unwrap_or(now));
            }
            BuildPool::Dispatch => {
                let last_selected = account.last_used_at.unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
                let (quota_known, quota_remaining) =
                    crate::poolindex::dispatch_quota(billing, recovery);
                self.dispatch_index.upsert(DispatchEntry {
                    id: account.id as u64,
                    priority: account.priority,
                    quota_known,
                    quota_remaining,
                    last_selected_at: last_selected,
                });
                let probe_at = account
                    .updated_at
                    .unwrap_or_else(|| now - chrono::Duration::hours(1));
                self.dispatch_probe_heap.upsert(account.id as u64, probe_at);
            }
        }
    }

    /// 调度池热路径有序 ID（对齐 `OrderedDispatchIDs`）。
    pub fn ordered_dispatch_ids(&self, limit: usize) -> Vec<i64> {
        self.dispatch_index
            .ascend(limit)
            .into_iter()
            .map(|entry| entry.id as i64)
            .collect()
    }

    /// 成功租约后更新调度公平序（对齐 `NoteDispatchSelected`）。
    pub fn note_dispatch_selected(&mut self, id: i64, at: DateTime<Utc>) {
        self.dispatch_index.touch_selected(id as u64, at);
    }

    /// 普通池到期探针候选（对齐 `DueNormalProbeIDs`）。
    pub fn due_normal_probe_ids(&self, now: DateTime<Utc>, limit: usize) -> Vec<i64> {
        self.normal_heap
            .due_ids(now, limit)
            .into_iter()
            .map(|id| id as i64)
            .collect()
    }

    /// 弹出下一个调度探针账号：到期优先，否则任意（对齐 `DispatchProbeTick` 头部）。
    pub fn pop_dispatch_probe(&mut self, now: DateTime<Utc>) -> Option<i64> {
        self.dispatch_probe_heap
            .pop_due(now)
            .or_else(|| self.dispatch_probe_heap.pop_any())
            .map(|id| id as i64)
    }

    /// 探针成功后重排下次探针（对齐 `runDispatchProbe` 尾部 `Upsert(ready.ID, now)`）。
    pub fn resched_dispatch_probe(&mut self, id: i64, at: DateTime<Utc>) {
        self.dispatch_probe_heap.upsert(id as u64, at);
    }

    /// 维护探针选道并取号（对齐 `MaintenanceProbeTick` 的 DRR + Pop 组）。
    pub fn maintenance_next(&mut self, now: DateTime<Utc>) -> Option<(Lane, i64)> {
        let verify_ready = self.verify_heap.peek_due(now).is_some();
        let normal_ready = self.normal_heap.peek_due(now).is_some();
        let delete_ready = !self.delete_heap.is_empty();
        let lane = self
            .maintenance_drr
            .next([verify_ready, normal_ready, delete_ready])?;
        let id = match lane {
            Lane::Verification => self.verify_heap.pop_due(now),
            Lane::Normal => self.normal_heap.pop_due(now),
            Lane::Delete => self.delete_heap.pop_any(),
        }
        .map(|id| id as i64)?;
        Some((lane, id))
    }
}

/// 四池规模汇总（对齐 Go `BuildProbePoolSummary` + `summarizeBuildProbePools`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildProbePoolSummary {
    pub dispatch: i64,
    pub normal: i64,
    pub verification: i64,
    pub delete: i64,
}

pub fn summarize_build_probe_pools(
    accounts: &[Account],
    recoveries: &HashMap<i64, QuotaRecovery>,
    now: DateTime<Utc>,
) -> BuildProbePoolSummary {
    let mut result = BuildProbePoolSummary::default();
    for account in accounts {
        match build_account_pool_at(account, now, recoveries.get(&account.id)) {
            Some(BuildPool::Dispatch) => result.dispatch += 1,
            Some(BuildPool::Normal) => result.normal += 1,
            Some(BuildPool::Verification) => result.verification += 1,
            Some(BuildPool::Delete) => result.delete += 1,
            None => {}
        }
    }
    result
}
