//! 到期最小堆（Go `heap.go` 移植）。
//!
//! 用 `BTreeMap<(DateTime, u64)>` 模拟：键 (到期时刻, id) 升序 → 迭代首项即「最早到期」。
//! 语义与 Go `dueHeap`（按 DueAt 升序、同刻按 id 升序）一致，且天然支持 upsert-fix。

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

/// 到期堆条目引用（外部持有 id 即可；具体到期时刻在堆内）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DueItem {
    pub id: u64,
    pub due_at: DateTime<Utc>,
}

/// 按到期时间升序的最小堆，带 id 旁表（Go `DueHeap`）。
#[derive(Debug, Default)]
pub struct DueHeap {
    /// 键 = (due_at, id)；值 = ()（哨兵）。BTree 迭代序即到期序。
    tree: BTreeMap<(DateTime<Utc>, u64), ()>,
    by_id: std::collections::HashMap<u64, DateTime<Utc>>,
}

impl DueHeap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    pub fn upsert(&mut self, id: u64, due_at: DateTime<Utc>) {
        if let Some(old) = self.by_id.remove(&id) {
            self.tree.remove(&(old, id));
        }
        self.tree.insert((due_at, id), ());
        self.by_id.insert(id, due_at);
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(due) = self.by_id.remove(&id) {
            self.tree.remove(&(due, id));
        }
    }

    pub fn contains(&self, id: u64) -> bool {
        self.by_id.contains_key(&id)
    }

    /// 堆顶（最早到期）是否已到期：`due_at <= now`。
    pub fn peek_due(&self, now: DateTime<Utc>) -> Option<u64> {
        self.tree
            .first_key_value()
            .filter(|((due, _), _)| *due <= now)
            .map(|((_, id), _)| *id)
    }

    /// 弹出已到期的堆顶；未到期不弹。
    pub fn pop_due(&mut self, now: DateTime<Utc>) -> Option<u64> {
        let head = self.tree.first_key_value().map(|((d, id), _)| (*d, *id))?;
        if head.0 > now {
            return None;
        }
        self.tree.remove(&head);
        self.by_id.remove(&head.1);
        Some(head.1)
    }

    /// 返回 `due_at <= now` 的账号 id（最多 `limit` 条），不从堆中移除。
    pub fn due_ids(&self, now: DateTime<Utc>, limit: usize) -> Vec<u64> {
        if limit == 0 {
            return Vec::new();
        }
        self.tree
            .iter()
            .take(limit)
            .filter(|((due, _), _)| *due <= now)
            .map(|((_, id), _)| *id)
            .collect()
    }

    /// 无论是否到期都弹出堆顶（删除池 FIFO 可用 epoch 0）。
    pub fn pop_any(&mut self) -> Option<u64> {
        let head = self.tree.first_key_value().map(|((d, id), _)| (*d, *id))?;
        self.tree.remove(&head);
        self.by_id.remove(&head.1);
        Some(head.1)
    }
}
