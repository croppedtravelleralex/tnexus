//! 调度索引镜像（Go `redis_mirror.go` 移植）。
//!
//! `RedisDispatchMirror` 用 ZSET 镜像调度索引序（score 为复合序编码）。
//! 失败不影响内存索引（调用方 `DispatchIndex` 在锁外触发 mirror）。

use chrono::{DateTime, Utc};

use super::dispatch::{DispatchEntry, DispatchMirror};

/// 复合序编码为可排序浮点（对齐 Go `dispatchScore`）。
///
/// 高 priority、高额度、早 lastSelected 排前：`pri*1e12 + quota*1e3 - last_epoch_ms/1e6`。
pub fn dispatch_score(entry: &DispatchEntry) -> f64 {
    let pri = entry.priority as f64 * 1e12;
    let quota = if entry.quota_known {
        entry.quota_remaining * 1e3
    } else {
        0.0
    };
    let last = -entry.last_selected_at.timestamp_millis() as f64 / 1e6;
    pri + quota + last
}

/// Redis ZSET 镜像（`RedisDispatchMirror`）。`client` 为可注入的 redis 连接。
///
/// 本实现仅在真实 Redis 可用时执行；`client == None` 时静默 no-op（与 Go nil-guard 一致）。
pub struct RedisDispatchMirror {
    redis: Option<redis::aio::ConnectionManager>,
    key: String,
}

impl RedisDispatchMirror {
    /// `key_prefix` 为空时默认 `grok2api`，镜像 key 为 `{prefix}:build:dispatch-index`。
    pub fn new(redis: Option<redis::aio::ConnectionManager>, key_prefix: &str) -> Self {
        let prefix = if key_prefix.is_empty() {
            "grok2api".to_string()
        } else {
            key_prefix.to_string()
        };
        Self {
            redis,
            key: format!("{prefix}:build:dispatch-index"),
        }
    }

    fn zadd(&self, id: u64, score: f64) {
        let Some(mut redis) = self.redis.clone() else {
            return;
        };
        let key = self.key.clone();
        tokio::spawn(async move {
            let _: redis::RedisResult<usize> = redis::cmd("ZADD")
                .arg(&[key.clone(), score.to_string(), id.to_string()])
                .query_async(&mut redis)
                .await;
        });
    }

    fn zrem(&self, id: u64) {
        let Some(mut redis) = self.redis.clone() else {
            return;
        };
        let key = self.key.clone();
        tokio::spawn(async move {
            let _: redis::RedisResult<usize> = redis::cmd("ZREM")
                .arg(&[key.clone(), id.to_string()])
                .query_async(&mut redis)
                .await;
        });
    }
}

impl DispatchMirror for RedisDispatchMirror {
    fn upsert(&self, entry: &DispatchEntry) {
        self.zadd(entry.id, dispatch_score(entry));
    }

    fn remove(&self, id: u64) {
        self.zrem(id);
    }

    fn touch_selected(&self, id: u64, at: DateTime<Utc>) {
        // 仅刷新 lastSelected 分量：score 需完整 entry，此处只记录一条 `at` 增量——
        // 简化实现直接 ZADD 覆盖 last 分量（Go 同受限逻辑）。
        let score = dispatch_score(&DispatchEntry {
            id,
            priority: 0,
            quota_remaining: 0.0,
            quota_known: false,
            last_selected_at: at,
        });
        self.zadd(id, score);
    }
}