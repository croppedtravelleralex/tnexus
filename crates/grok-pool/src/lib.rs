//! grok-pool — Grok 号池（G1 简化单池）。
//!
//! 阶段边界（39e G1-P4）：G1 仅提供 **简化单池** dispatch，用于 OCR/chat
//! 最小闭环，可 pin 测试账号。完整双轨号池（Web Image 四池 + Chat 三池）、
//! `poolindex`（heap / DRR / timing_wheel）、`lane_quota`、`four_pool_probe`
//! 属 G3（39e G3-P1..P3，Go `account/web_pool*.go`）。本 crate 刻意不实现它们。
//!
//! 并发模型：单进程内存池，用 `tokio::sync::RwLock` 保护；多实例部署需
//! Redis runtime（G3-7）。

use std::collections::HashMap;
use std::sync::Arc;

use grok_domain::{Account, Provider};
use grok_storage::repo::AccountRepository;
use grok_storage::StorageError;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::RwLock;
use tokio::time::Duration;

/// 简化单池。
///
/// 载入 `grok_web` enabled 账号；`select` 优先返回 pin 账号，否则在非冷却
/// 账号中随机选一个。`dispatch_failure` 记录冷却窗口，期间不参与选号。
pub struct SimplifiedPool {
    inner: RwLock<Inner>,
    // RNG 用 seed 初始化：生产用 StdRng::from_entropy()，测试可注入固定 seed。
    rng: RwLock<StdRng>,
    /// 定长冷却时长（dispatch_failure 后该账号冷却这么久）。
    cooldown: Duration,
}

struct Inner {
    /// 池内账号（id → entry，按 `list_pool` 顺序）。
    accounts: Vec<Account>,
    /// 当前 pin 的测试账号 id。
    pin: Option<i64>,
    /// cooldown_until：冷却结束时间戳（tokio Instant 单调时钟）。
    cooldown_until: HashMap<i64, tokio::time::Instant>,
    /// dispatch 记账。
    success_count: HashMap<i64, u64>,
    failure_count: HashMap<i64, u64>,
}

/// 池加载结果；`load` 时 `account` 为 None 代表加载失败。
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
}

impl SimplifiedPool {
    /// 用默认冷却时长（2s）新建空池；需调用 `load` 填充账号。
    pub fn new() -> Self {
        Self::with_seed_and_cooldown(0, Duration::from_secs(2))
    }

    /// 载入账号（`grok_web` + enabled）到池内。
    pub async fn load<R: AccountRepository + ?Sized>(&self, repo: &R) -> Result<(), LoadError> {
        let accounts = repo
            .list_pool(Provider::GrokWeb, true)
            .await
            .map_err(LoadError::Storage)?;
        let mut inner = self.inner.write().await;
        inner.accounts = accounts;
        inner.success_count.clear();
        inner.failure_count.clear();
        inner.cooldown_until.clear();
        Ok(())
    }

    /// 直接注入内存账号（测试 / 单测直达，不依赖 PG）。
    pub async fn load_in_memory(&self, accounts: Vec<Account>) {
        let mut inner = self.inner.write().await;
        inner.accounts = accounts;
        inner.pin = None;
        inner.success_count.clear();
        inner.failure_count.clear();
        inner.cooldown_until.clear();
    }

    /// 池内账号数量（已启用的 grok_web 账号总数）。
    pub async fn len(&self) -> usize {
        self.inner.read().await.accounts.len()
    }

    /// 是否为空池。
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// 固定测试账号。pin 之后 `select` 优先返回该账号（若在池内且未冷却）。
    pub async fn pin(&self, account_id: i64) {
        self.inner.write().await.pin = Some(account_id);
    }

    /// 取消 pin。
    pub async fn unpin(&self) {
        self.inner.write().await.pin = None;
    }

    /// 当前 pin 账号（测试断言用）。
    pub async fn pinned(&self) -> Option<i64> {
        self.inner.read().await.pin
    }

    /// 选一个用于推理的账号。
    ///
    /// - 若 pin 账号在池内且未冷却，优先返回它。
    /// - 否则在所有「未冷却」账号中随机取一个。
    /// - 无可选账号返回 `None`；`exclude` 账号即便在池内也被排除。
    pub async fn select(&self, exclude: Option<i64>) -> Option<i64> {
        let now = tokio::time::Instant::now();
        let inner = self.inner.read().await;

        // 1) pin 优先（未冷却 + 未被 exclude + 未超出池）。
        if let Some(pin_id) = inner.pin {
            if Some(pin_id) != exclude && !inner.in_cooldown(pin_id, now) && inner.has(pin_id) {
                return Some(pin_id);
            }
        }

        // 2) 随机选一个非冷却、非 exclude 账号。
        let candidates: Vec<i64> = inner
            .accounts
            .iter()
            .map(|a| a.id)
            .filter(|id| Some(*id) != exclude && !inner.in_cooldown(*id, now))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let idx = self.rng.write().await.gen_range(0..candidates.len());
        Some(candidates[idx])
    }

    /// dispatch 记账：Success +1。
    pub async fn dispatch_success(&self, account_id: i64) {
        let mut inner = self.inner.write().await;
        *inner.success_count.entry(account_id).or_insert(0) += 1;
    }

    /// dispatch 记账：Failure +1 + 进入冷却（之后 `selected` 不再放行该账号）。
    pub async fn dispatch_failure(&self, account_id: i64) {
        let mut inner = self.inner.write().await;
        *inner.failure_count.entry(account_id).or_insert(0) += 1;
        inner
            .cooldown_until
            .insert(account_id, tokio::time::Instant::now() + self.cooldown);
    }

    /// 查询账号累计成功次数（测试 / Admin 指标）。
    pub async fn success_count(&self, account_id: i64) -> u64 {
        self.inner
            .read()
            .await
            .success_count
            .get(&account_id)
            .copied()
            .unwrap_or(0)
    }

    /// 查询账号累计失败次数。
    pub async fn failure_count(&self, account_id: i64) -> u64 {
        self.inner
            .read()
            .await
            .failure_count
            .get(&account_id)
            .copied()
            .unwrap_or(0)
    }

    /// 该账号当前是否处于冷却窗口。
    pub async fn in_cooldown(&self, account_id: i64) -> bool {
        let inner = self.inner.read().await;
        inner.in_cooldown(account_id, tokio::time::Instant::now())
    }

    /// 池内账号 id 列表（保持 `list_pool` 顺序，测试断言用）。
    pub async fn account_ids(&self) -> Vec<i64> {
        self.inner
            .read()
            .await
            .accounts
            .iter()
            .map(|a| a.id)
            .collect()
    }

    // ---- 测试辅助：注入确定的种子与冷却，保证单测确定性 ----

    #[doc(hidden)]
    pub fn with_seed_and_cooldown(seed: u64, cooldown: Duration) -> Self {
        Self {
            inner: RwLock::new(Inner {
                accounts: Vec::new(),
                pin: None,
                cooldown_until: HashMap::new(),
                success_count: HashMap::new(),
                failure_count: HashMap::new(),
            }),
            rng: RwLock::new(StdRng::seed_from_u64(seed)),
            cooldown,
        }
    }

    #[doc(hidden)]
    pub fn with_cooldown(cooldown: Duration) -> Self {
        Self::with_seed_and_cooldown(42, cooldown)
    }
}

impl Inner {
    fn has(&self, id: i64) -> bool {
        self.accounts.iter().any(|a| a.id == id)
    }

    fn in_cooldown(&self, id: i64, now: tokio::time::Instant) -> bool {
        self.cooldown_until
            .get(&id)
            .map(|until| *until > now)
            .unwrap_or(false)
    }
}

impl Default for SimplifiedPool {
    fn default() -> Self {
        Self::new()
    }
}

// 用 `std::sync::Arc` 封装对外暴露，方便多 handler 持有同一实例。
/// 一个可共享的 SimplifiedPool 别名（供多 handler 持有同一实例）。
pub type SharedPool = Arc<SimplifiedPool>;

#[cfg(test)]
mod tests {
    use super::*;
    use grok_domain::{Account, AuthStatus, Provider};

    fn acc(id: i64) -> Account {
        Account {
            id,
            identity_key: format!("web-{id}"),
            provider: Provider::GrokWeb,
            enabled: true,
            auth_status: AuthStatus::Active,
            priority: 0,
            observed_model: None,
        }
    }

    async fn pool_with(ids: &[i64], cooldown: Duration) -> SimplifiedPool {
        let p = SimplifiedPool::with_cooldown(cooldown);
        p.load_in_memory(ids.iter().map(|&id| acc(id)).collect())
            .await;
        p
    }

    #[tokio::test]
    async fn pin_is_preferred() {
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        p.pin(2).await;
        assert_eq!(p.select(None).await, Some(2));
        // 连续多次都应返回 pin。
        assert_eq!(p.select(None).await, Some(2));
        assert_eq!(p.select(None).await, Some(2));
    }

    #[tokio::test]
    async fn pin_rejected_if_excluded() {
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        p.pin(2).await;
        // exclude 与 pin 相同 → 跳过 pin，回退随机（且不再选 2）。
        let sel = p.select(Some(2)).await;
        assert_ne!(sel, Some(2));
        assert!(sel.is_some());
    }

    #[tokio::test]
    async fn select_rotates_among_available() {
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        // 同一 seed → 结果确定；但仅断言返回池内 id 之一。
        for _ in 0..20 {
            let sel = p.select(None).await.unwrap();
            assert!((1..=3).contains(&sel));
        }
    }

    #[tokio::test]
    async fn empty_pool_returns_none() {
        let p = pool_with(&[], Duration::from_secs(60)).await;
        assert!(p.select(None).await.is_none());
    }

    #[tokio::test]
    async fn cooldown_blocks_dispatch() {
        let p = pool_with(&[1, 2, 3], Duration::from_millis(40)).await;
        // 全部失败进入冷却。
        p.dispatch_failure(1).await;
        p.dispatch_failure(2).await;
        p.dispatch_failure(3).await;
        assert!(p.select(None).await.is_none(), "all in cooldown → none");
        assert!(p.in_cooldown(1).await);

        // 冷却窗口过后（>40ms）重新放行。
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            p.select(None).await.is_some(),
            "cooldown expired → available"
        );
        assert!(!p.in_cooldown(1).await);
    }

    #[tokio::test]
    async fn dispatch_accounts_success_failure() {
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        p.dispatch_success(1).await;
        p.dispatch_success(1).await;
        p.dispatch_failure(1).await;
        assert_eq!(p.success_count(1).await, 2);
        assert_eq!(p.failure_count(1).await, 1);
        // 未操作账号计数为 0。
        assert_eq!(p.success_count(2).await, 0);
    }

    #[tokio::test]
    async fn cooldown_only_blocks_that_account() {
        let p = pool_with(&[1, 2, 3], Duration::from_millis(200)).await;
        p.dispatch_failure(1).await;
        // 2 / 3 仍可选。
        let sel = p.select(None).await;
        assert!(matches!(sel, Some(2) | Some(3)));
    }

    #[tokio::test]
    async fn pool_len_after_load_in_memory() {
        let p = SimplifiedPool::with_cooldown(Duration::from_secs(60));
        p.load_in_memory(vec![acc(1), acc(2)]).await;
        assert_eq!(p.len().await, 2);
    }
}
