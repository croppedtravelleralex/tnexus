//! 调度有序集（Go `dispatch.go` 移植）。

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use grok_domain::{Billing, QuotaRecovery, QuotaRecoveryStatus};

/// 调度池有序集中的一条账号记录。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DispatchEntry {
    pub id: u64,
    pub priority: i32,
    pub quota_remaining: f64,
    pub quota_known: bool,
    pub last_selected_at: DateTime<Utc>,
}

impl DispatchEntry {
    /// 默认构造（低位优先级 + 未知额度 + epoch）。
    pub fn new(id: u64) -> Self {
        Self {
            id,
            priority: 0,
            quota_remaining: 0.0,
            quota_known: false,
            last_selected_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        }
    }
}

/// 排序键（对齐 `dispatchKey.less`，实现为降序优先等语义）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct DispatchKey {
    priority: i32,
    quota_remaining: f64,
    quota_known: bool,
    last_selected_at: DateTime<Utc>,
    id: u64,
}

impl DispatchKey {
    fn of(e: &DispatchEntry) -> Self {
        Self {
            priority: e.priority,
            quota_remaining: e.quota_remaining,
            quota_known: e.quota_known,
            last_selected_at: e.last_selected_at,
            id: e.id,
        }
    }

    /// Rust `Ord` 是升序；我们把 Go 的 `less`（被排前面 → "更小"）映射成标准 `cmp`：
    /// 排前面的返回 `Less`。
    fn go_less(&self, o: &DispatchKey) -> bool {
        if self.priority != o.priority {
            return self.priority > o.priority;
        }
        if self.quota_known != o.quota_known {
            return self.quota_known && !o.quota_known;
        }
        if self.quota_known && self.quota_remaining != o.quota_remaining {
            return self.quota_remaining > o.quota_remaining;
        }
        if self.last_selected_at != o.last_selected_at {
            return self.last_selected_at < o.last_selected_at;
        }
        self.id < o.id
    }
}

impl PartialOrd for DispatchKey {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for DispatchKey {
    /// Rust `Ord` 是升序；我们把 Go 的 `less`（被排前面 → "更小"）映射成标准 `cmp`：
    /// 排前面的返回 `Less`。
    fn cmp(&self, o: &Self) -> Ordering {
        if self.go_less(o) {
            Ordering::Less
        } else if o.go_less(self) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl Eq for DispatchKey {}

/// 从 billing 快照或活跃 recovery 推导调度额度序字段（Go `DispatchQuota`）。
///
/// 复用 `grok_domain::Billing` / `grok_domain::QuotaRecovery`。
pub fn dispatch_quota(billing: Option<&Billing>, recovery: Option<&QuotaRecovery>) -> (bool, f64) {
    if let Some(b) = billing {
        if b.monthly_limit > 0.0 {
            return (true, b.remaining());
        }
        if b.on_demand_cap > 0.0 {
            let mut used = b.on_demand_used;
            if used == 0.0 && b.credit_usage_percent() > 0.0 {
                used = b.on_demand_cap * b.credit_usage_percent() / 100.0;
            }
            let rem = (b.on_demand_cap - used).max(0.0);
            return (true, rem);
        }
        if b.prepaid_balance > 0.0 {
            return (true, b.prepaid_balance);
        }
    }
    if let Some(r) = recovery {
        if r.status == QuotaRecoveryStatus::Active && r.confirmed_limit > 0 {
            let rem = (r.confirmed_limit - r.confirmed_used).max(0) as f64;
            return (true, rem);
        }
    }
    (false, 0.0)
}

/// 可选镜像（如 Redis ZSET）；失败不影响内存索引（Go `DispatchMirror`）。
pub trait DispatchMirror: Send + Sync {
    fn upsert(&self, entry: &DispatchEntry);
    fn remove(&self, id: u64);
    fn touch_selected(&self, id: u64, at: DateTime<Utc>);
}

/// 内存 BTree 有序集 + byID 旁表；可选 Mirror（Go `DispatchIndex`）。
pub struct DispatchIndex {
    tree: BTreeMap<DispatchKey, DispatchEntry>,
    by_id: HashMap<u64, DispatchKey>,
    mirror: Option<Box<dyn DispatchMirror>>,
}

impl Default for DispatchIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl DispatchIndex {
    pub fn new() -> Self {
        Self {
            tree: BTreeMap::new(),
            by_id: HashMap::new(),
            mirror: None,
        }
    }

    pub fn set_mirror(&mut self, mirror: Option<Box<dyn DispatchMirror>>) {
        self.mirror = mirror;
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn upsert(&mut self, entry: DispatchEntry) {
        if let Some(old) = self.by_id.remove(&entry.id) {
            self.tree.remove(&old);
        }
        let key = DispatchKey::of(&entry);
        self.tree.insert(key, entry);
        self.by_id.insert(entry.id, key);
        if let Some(m) = &self.mirror {
            m.upsert(&entry);
        }
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(old) = self.by_id.remove(&id) {
            self.tree.remove(&old);
        }
        if let Some(m) = &self.mirror {
            m.remove(id);
        }
    }

    pub fn contains(&self, id: u64) -> bool {
        self.by_id.contains_key(&id)
    }

    /// 按调度优先序返回至多 `limit` 条快照；`limit <= 0` 返回全部。
    pub fn ascend(&self, limit: usize) -> Vec<DispatchEntry> {
        let items: Vec<&DispatchEntry> = if limit > 0 {
            self.tree.values().take(limit).collect()
        } else {
            self.tree.values().collect()
        };
        items.into_iter().cloned().collect()
    }

    /// 成员 id 集合（对账用）。
    pub fn ids(&self) -> std::collections::HashSet<u64> {
        self.by_id.keys().copied().collect()
    }

    pub fn touch_selected(&mut self, id: u64, at: DateTime<Utc>) {
        match self.by_id.remove(&id) {
            Some(old) => {
                if let Some(entry) = self.tree.remove(&old) {
                    let mut updated = entry;
                    updated.last_selected_at = at;
                    let nk = DispatchKey::of(&updated);
                    self.tree.insert(nk, updated);
                    self.by_id.insert(id, nk);
                }
            }
            None => return,
        }
        if let Some(m) = &self.mirror {
            m.touch_selected(id, at);
        }
    }
}
