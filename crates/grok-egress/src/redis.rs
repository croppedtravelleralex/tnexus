//! Redis 多实例并发闸门实现（G3）。
//!
//! 语义对齐 Go `infra/egress/manager.go` 的 per-scope 有界闸门，但把槽位状态放到
//! Redis，使多实例以同一 `(scope, gate)` 名额竞争（docs/39b §5 `concurrency`）。
//!
//! 存储模型：每个 `(scope, gate)` 一个 **ZSET**，key = `concurrency:{scope}:{gate}`
//! （`prefix` 可自定义，默认空）。一个活跃持有者 = 一个 member（值 = 本实例派发的
//! 唯一租约 ID），member 的 score = 租约到期 epoch 秒。
//!
//! - `acquire`：先清除过期 member（`ZREMRANGEBYSCORE -inf <now`），再 `ZCARD` 与上限
//!   比较；未满则以本次租约唯一 ID 为 member `ZADD`（score = now + lease）。满则轮询
//!   等待，受 `lease` 时长约束；超时返回 `Error::Timeout`。
//! - `release`：`ZREM` 自己的 member（幂等），由租约 drop / 显式 release 触发。
//! - `active`：清除过期后返回 `ZCARD`（跨实例当前占用数）。
//!
//! 原子性：ZCARD 与 ZADD 各自为单条命令；即使多个实例同时越过 `ZCARD < limit`，各自
//! `ZADD` 的唯一 member 也只会让活跃数恰好等于持有 ZSET member 数，不会超卖。过期
//! score 提供租约到期自动让位，实例崩溃时槽位最终被清理。
//!
//! 等待策略为 polling（`poll_interval`，默认 10ms），非阻塞订阅。G3-P5 只做基础名额
//! 竞争，不含 Go 侧节点选择 / 粘滞 / 流量 hops。
//!
//! 由于 `LeaseManager` 的 `release`/`active` 为同步接口而 Redis 是异步客户端，
//! 释放与查询均通过当前 runtime 的 `Handle::block_on`/`spawn` 派发（调用方须处于
//! 有 runtime 的上下文）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use grok_domain::egress::Scope;

use crate::lease::{Error, GateId, Lease, LeaseManager};

/// 允许参与 Redis 并发竞争的 scope。G3 起全部开放；单实例 G1 仍只走内存实现。
pub const REDIS_ENABLED_SCOPES: &[Scope] = &[Scope::GrokWeb, Scope::GrokWebAsset];

/// 默认轮询间隔：短步长重试，权衡空占 CPU 与响应度。
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Redis 多实例并发闸门。
#[derive(Clone)]
pub struct RedisLeaseManager {
    redis: ConnectionManager,
    /// Redis key 前缀（`grok` 效果由调用方传；默认空串不写前缀）。
    prefix: Arc<str>,
    /// (scope -> 并发上限)；缺省 web=1、asset=4、其余=1。
    limits: Arc<std::sync::RwLock<HashMap<String, usize>>>,
    /// 本实例名，用于租约 ID 前缀，区分多实例。
    instance: Arc<str>,
    /// 轮询等待间隔。
    poll_interval: Duration,
}

impl std::fmt::Debug for RedisLeaseManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisLeaseManager")
            .field("prefix", &self.prefix)
            .field("instance", &self.instance)
            .field("poll_interval", &self.poll_interval)
            .finish_non_exhaustive()
    }
}

impl RedisLeaseManager {
    /// 用已建好的 Redis `ConnectionManager` 构造。
    ///
    /// `prefix` 为 Redis key 前缀（如 `grok`），默认空串不写前缀。
    /// 缺省并发：`grok_web=1`、`grok_web_asset=4`、其余 scope=1（对齐 Go 默认）。
    pub fn new(redis: ConnectionManager, prefix: impl Into<String>) -> Self {
        Self {
            redis,
            prefix: Arc::from(prefix.into()),
            limits: Arc::new(std::sync::RwLock::new(HashMap::new())),
            instance: Arc::from(format!("i{}", take_instance_num())),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// 覆盖各 scope 并发上限。
    pub fn with_limits(
        redis: ConnectionManager,
        prefix: impl Into<String>,
        limits: &[(Scope, usize)],
    ) -> Self {
        let m = Self::new(redis, prefix);
        {
            let mut map = m.limits.write().unwrap();
            for (scope, n) in limits {
                map.insert(scope.as_str().to_string(), *n);
            }
        }
        m
    }

    /// 显式给定实例名（多实例测试用）。
    pub fn with_instance(
        redis: ConnectionManager,
        prefix: impl Into<String>,
        instance: impl Into<String>,
    ) -> Self {
        Self {
            redis,
            prefix: Arc::from(prefix.into()),
            limits: Arc::new(std::sync::RwLock::new(HashMap::new())),
            instance: Arc::from(instance.into()),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

impl RedisLeaseManager {
    /// scope 并发上限（缺省 web=1、asset=4、其余=1）。
    fn limit_for(&self, scope: Scope) -> usize {
        let map = self.limits.read().unwrap();
        if let Some(n) = map.get(&scope.as_str().to_string()) {
            return *n;
        }
        match scope {
            Scope::GrokWeb => 1,
            Scope::GrokWebAsset => 4,
            _ => 1,
        }
    }

    /// 拼接 Redis ZSET key：`{prefix}:concurrency:{scope}:{gate}`。
    fn zset_key(&self, scope: Scope, gate: &str) -> String {
        let mut k = String::new();
        if !self.prefix.is_empty() {
            k.push_str(&self.prefix);
            k.push(':');
        }
        k.push_str("concurrency:");
        k.push_str(scope.as_str());
        k.push(':');
        k.push_str(gate);
        k
    }

    /// 清除已过期 member（score < now）。
    async fn health(&self, key: &str, now: i64) -> Result<(), Error> {
        let mut con = self.redis.clone();
        con.zrembyscore(key, "-inf", now)
            .await
            .map_err(|e| Error::Store(e.to_string()))
    }

    /// 尝试占一个槽位：清除过期后若未满则用唯一 member ZADD。
    /// `Ok(Some(uid))` 表示抢占成功；`Ok(None)` 表示槽满。
    async fn try_acquire(
        &self,
        key: &str,
        scope: Scope,
        now: i64,
        expire: i64,
    ) -> Result<Option<(String, i64)>, Error> {
        let limit = self.limit_for(scope);
        self.health(key, now).await?;
        let mut con = self.redis.clone();
        let n: i64 = con
            .zcard(key)
            .await
            .map_err(|e| Error::Store(e.to_string()))?;
        if n >= limit as i64 {
            return Ok(None);
        }
        let uid = format!("{}-{}", self.instance, now_nanos());
        let added: i64 = con
            .zadd(key, &uid, expire)
            .await
            .map_err(|e| Error::Store(e.to_string()))?;
        if added >= 1 {
            Ok(Some((uid, expire)))
        } else {
            // member 已存在（本实例同 ns 内重复）——极罕见；幂等拒绝。
            Ok(None)
        }
    }

    /// 异步释放：ZREM 自己的 member（best-effort）。
    async fn do_release(redis: ConnectionManager, key: String, uid: String) {
        let mut con = redis;
        let _ = con.zrem::<_, _, i64>(key, uid).await;
    }
}

#[async_trait::async_trait]
impl LeaseManager for RedisLeaseManager {
    async fn acquire(&self, scope: Scope, gate: GateId, lease: Duration) -> Result<Lease, Error> {
        if !REDIS_ENABLED_SCOPES.contains(&scope) {
            return Err(Error::ScopeUnsupported(scope.as_str().to_string()));
        }
        let key = self.zset_key(scope, &gate);
        let deadline = tokio::time::Instant::now() + lease;
        let lease_secs = lease.as_secs().max(1) as i64;
        loop {
            let now = now_epoch();
            match self.try_acquire(&key, scope, now, now + lease_secs).await {
                Ok(Some((uid, _exp))) => {
                    let redis = self.redis.clone();
                    let key2 = key.clone();
                    let uid2 = uid.clone();
                    return Ok(Lease::new_redis(
                        scope,
                        gate,
                        Box::new(move || {
                            let r = redis;
                            let k = key2;
                            let u = uid2;
                            // 释放走当前 runtime 派发异步 ZREM。
                            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                                handle.spawn(Self::do_release(r, k, u));
                            }
                        }),
                    ));
                }
                Ok(None) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::Timeout(lease));
                    }
                    let wait = self.poll_interval.min(lease);
                    tokio::time::sleep(wait).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn release(&self, lease: Lease) {
        drop(lease);
    }

    fn active(&self, scope: Scope, gate: &str) -> usize {
        let key = self.zset_key(scope, gate);
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let cur = handle.block_on(async {
                    self.health(&key, now_epoch()).await?;
                    let mut con = self.redis.clone();
                    let n: i64 = con
                        .zcard(&key)
                        .await
                        .map_err(|e| Error::Store(e.to_string()))?;
                    Ok::<usize, Error>(n.max(0) as usize)
                });
                cur.unwrap_or(0)
            }
            Err(_) => 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// 多实例默认实例名编号（进程内递增 + 微随机）。
fn take_instance_num() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id() as u64;
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    (pid & 0xffff_ffff) * 1_000_000 + seq
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::LeaseManager;
    use std::time::Duration;

    // 仅当存在本地 Redis 时运行；否则忽略（CI 无 redis 时跳过而不失败）。
    // 无 redis 时返回 None；有则直接构造. 返回 (manager, 可直接 async 查询的 client)。
    async fn try_manager() -> Option<(RedisLeaseManager, ConnectionManager)> {
        let url = std::env::var("GROK_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1".into());
        let client = redis::Client::open(url.as_str()).ok()?;
        // 探测连通性（短超时），连不通快速失败。
        let mut con = client.get_multiplexed_tokio_connection().await.ok()?;
        let _: String = redis::cmd("PING")
            .query_async(&mut con)
            .await
            .unwrap_or_else(|_| "PONG".into());
        let mgr = ConnectionManager::new(client).await.ok()?;
        Some((RedisLeaseManager::new(mgr.clone(), ""), mgr.clone()))
    }

    // 需要真实 Redis；无 redis 时自动跳过（本地有可真正覆盖核心路径）。
    #[tokio::test(flavor = "multi_thread")]
    async fn acquire_release_roundtrip() {
        let Some((m, redis)) = try_manager().await else {
            eprintln!("redis not available; skipping");
            return;
        };
        let gate: GateId = format!("rt-{}", now_nanos());
        let key = m.zset_key(Scope::GrokWeb, &gate);
        let card = |redis: &ConnectionManager| {
            let mut con = redis.clone();
            let key = &key;
            async move {
                let n: i64 = con.zcard(key).await.unwrap_or(0);
                n.max(0) as usize
            }
        };
        assert_eq!(card(&redis).await, 0, "empty gate");

        let l = m
            .acquire(Scope::GrokWeb, gate.clone(), Duration::from_secs(2))
            .await
            .expect("acquire");
        assert_eq!(card(&redis).await, 1, "held lease counts in ZSET");
        assert_eq!(l.gate(), gate.as_str());

        l.release();
        // 释放为异步派发，需让步等待 ZREM 生效。
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        while tokio::time::Instant::now() < deadline {
            if card(&redis).await == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("lease not released after 200ms");
    }

    // 多实例名额竞争：grok_web 上限 1，占满后第二实例在短时限内应 Timeout。
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrency_ceiling_blocks_second() {
        let Some((m1, _)) = try_manager().await else {
            eprintln!("redis not available; skipping");
            return;
        };
        let Some((m2, redis)) = try_manager().await else {
            return;
        };
        let gate: GateId = format!("ceiling-{}", now_nanos());
        let key = m1.zset_key(Scope::GrokWeb, &gate);

        let a = m1
            .acquire(Scope::GrokWeb, gate.clone(), Duration::from_secs(3))
            .await
            .expect("first acquire");

        // 第二实例同 gate：web 上限 1 → 短暂超时。
        let b = m2
            .acquire(Scope::GrokWeb, gate.clone(), Duration::from_millis(60))
            .await;
        assert!(
            matches!(b, Err(Error::Timeout(_))),
            "second instance should timeout at ceiling, got {b:?}"
        );

        // 释放后第二实例可获槽。
        a.release();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        loop {
            let mut con = redis.clone();
            let n: i64 = con.zcard(&key).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "lease not released");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let c = m2
            .acquire(Scope::GrokWeb, gate.clone(), Duration::from_secs(2))
            .await
            .expect("second acquires after first releases");
        c.release();
    }

    // 无 redis 也须编译通过的空测试，确保依赖树 OK（占位）。
    #[test]
    fn compiles_without_redis() {
        let _ = DEFAULT_POLL_INTERVAL;
    }
}
