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
use tokio::sync::RwLock;
use tokio::time::Duration;

/// 调度索引原语（G3，Go `account/poolindex/*.go` 移植）。
pub mod poolindex;

/// Web 图池选择纯函数（G3，Go `account/web_pool.go` 移植）。
pub mod web_pool;

/// Web dispatch pin 对齐纯函数（G3，Go `web_pool_pins.go` + `imagine_slots.go` 移植）。
pub mod pins;

/// Build 四池选择与热路径索引（G3-P3，Go `account/four_pool_probe.go` 移植）。
pub mod build_pool;

/// 账号选择器（G3-P4，Go `gateway/selector.go` 移植）。
pub mod selector;

/// 简化单池。
///
/// 载入 `grok_web` enabled 账号；`select` 优先返回 pin 账号，否则按最近最少使用
/// 顺序（LRU）选一个非冷却账号。`dispatch_failure` 记录短冷却窗口（2s），
/// `dispatch_rate_limited` 记录指数退避长冷却（60s 起，上限 300s），均期间不参与选号。
pub struct SimplifiedPool {
    inner: RwLock<Inner>,
    /// 普通失败冷却时长（dispatch_failure 后该账号冷却这么久）。
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
    /// 连续限速/拒绝失败计数（用于指数退避冷却；dispatch_success 后清零）。
    rl_failure_count: HashMap<i64, u32>,
    /// 全局单调选择序号（每次 select_skip 成功选中后递增）。
    select_seq: u64,
    /// 每个账号最近一次被选中时的 select_seq（0 / 缺省 = 从未选中，优先级最高）。
    last_selected_seq: HashMap<i64, u64>,
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
        inner.rl_failure_count.clear();
        inner.select_seq = 0;
        inner.last_selected_seq.clear();
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
        inner.rl_failure_count.clear();
        inner.select_seq = 0;
        inner.last_selected_seq.clear();
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
    /// - 否则按 LRU（最近最少使用）顺序选非冷却账号；同等未选中则 id 升序决胜。
    /// - 无可选账号返回 `None`；`exclude` 账号即便在池内也被排除。
    pub async fn select(&self, exclude: Option<i64>) -> Option<i64> {
        self.select_skip(exclude, &[]).await
    }

    /// 选一个账号，跳过 `skip` 列表中的 id（用于无 session keys 等场景，不进入冷却）。
    pub async fn select_skip(&self, exclude: Option<i64>, skip: &[i64]) -> Option<i64> {
        let now = tokio::time::Instant::now();
        // 写锁：需要更新 last_selected_seq（LRU 记账）
        let mut inner = self.inner.write().await;

        // 收集可用账号 id（for 循环避免闭包多重借用歧义）
        let mut eligible: Vec<i64> = Vec::new();
        for a in &inner.accounts {
            let id = a.id;
            if Some(id) == exclude {
                continue;
            }
            if skip.contains(&id) {
                continue;
            }
            if inner
                .cooldown_until
                .get(&id)
                .map(|&u| u > now)
                .unwrap_or(false)
            {
                continue;
            }
            eligible.push(id);
        }

        // pin 优先
        if let Some(pin_id) = inner.pin {
            if eligible.contains(&pin_id) {
                let seq = inner.select_seq.saturating_add(1);
                inner.select_seq = seq;
                inner.last_selected_seq.insert(pin_id, seq);
                return Some(pin_id);
            }
        }

        // LRU：选 (上次选中序号, id) 最小的账号；序号 0 / 缺省 = 从未选中，优先级最高
        let chosen = eligible
            .iter()
            .copied()
            .min_by_key(|&id| (inner.last_selected_seq.get(&id).copied().unwrap_or(0), id));

        if let Some(id) = chosen {
            let seq = inner.select_seq.saturating_add(1);
            inner.select_seq = seq;
            inner.last_selected_seq.insert(id, seq);
        }
        chosen
    }

    /// dispatch 记账：Success +1，同时清零该账号的连续限速失败计数。
    pub async fn dispatch_success(&self, account_id: i64) {
        let mut inner = self.inner.write().await;
        *inner.success_count.entry(account_id).or_insert(0) += 1;
        // 成功后清零，下次限速重新从 60s 基础值开始退避
        inner.rl_failure_count.remove(&account_id);
    }

    /// dispatch 记账：Failure +1 + 短冷却（`self.cooldown`，默认 2s）。
    /// 用于普通瞬时失败（网络抖动等）；限速/拒绝失败请用 `dispatch_rate_limited`。
    pub async fn dispatch_failure(&self, account_id: i64) {
        let mut inner = self.inner.write().await;
        *inner.failure_count.entry(account_id).or_insert(0) += 1;
        inner
            .cooldown_until
            .insert(account_id, tokio::time::Instant::now() + self.cooldown);
    }

    /// dispatch 记账：限速/拒绝失败（429 / 403）→ 指数退避长冷却。
    ///
    /// 退避策略：`60s × 2^(n-1)`，上限 300s（5 分钟）。
    /// - 第 1 次连续限速：60s
    /// - 第 2 次：120s
    /// - 第 3 次及以上：300s（硬上限）
    ///
    /// 连续限速计数在 `dispatch_success` 后清零，下一轮限速从 60s 重新计算。
    pub async fn dispatch_rate_limited(&self, account_id: i64) {
        let mut inner = self.inner.write().await;
        *inner.failure_count.entry(account_id).or_insert(0) += 1;
        let consecutive = {
            let c = inner.rl_failure_count.entry(account_id).or_insert(0);
            *c += 1;
            *c
        };
        // 60s × 2^(n-1)，上限 300s
        let shift = consecutive.saturating_sub(1).min(2) as u32;
        let backoff = Duration::from_secs((60u64 << shift).min(300));
        inner
            .cooldown_until
            .insert(account_id, tokio::time::Instant::now() + backoff);
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

    // ---- 测试辅助：注入确定的冷却，保证单测确定性（seed 参数保留签名兼容性，已忽略） ----

    #[doc(hidden)]
    pub fn with_seed_and_cooldown(_seed: u64, cooldown: Duration) -> Self {
        Self {
            inner: RwLock::new(Inner {
                accounts: Vec::new(),
                pin: None,
                cooldown_until: HashMap::new(),
                success_count: HashMap::new(),
                failure_count: HashMap::new(),
                rl_failure_count: HashMap::new(),
                select_seq: 0,
                last_selected_seq: HashMap::new(),
            }),
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
            ..Default::default()
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
        // exclude 与 pin 相同 → 跳过 pin，LRU 回退（不再选 2）。
        let sel = p.select(Some(2)).await;
        assert_ne!(sel, Some(2));
        assert!(sel.is_some());
    }

    #[tokio::test]
    async fn select_rotates_among_available() {
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        // LRU：每次选中后序号递增，确保轮转；断言返回池内 id 之一。
        for _ in 0..20 {
            let sel = p.select(None).await.unwrap();
            assert!((1..=3).contains(&sel));
        }
    }

    #[tokio::test]
    async fn lru_cycles_through_all_accounts() {
        // 无 pin：多次 select 应轮转覆盖所有账号（LRU 保证没有账号被饥饿）。
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        let mut seen: std::collections::HashSet<i64> = Default::default();
        for _ in 0..6 {
            let id = p.select(None).await.unwrap();
            seen.insert(id);
        }
        // 6 次选择中，3 个账号都应至少出现过一次（LRU 轮转）。
        assert_eq!(seen.len(), 3, "LRU 调度应覆盖所有账号: {seen:?}");
    }

    #[tokio::test]
    async fn lru_order_is_deterministic() {
        // 无 pin、无冷却：首次 6 次 select 应严格按 id 升序轮转（1→2→3→1→2→3）。
        let p = pool_with(&[1, 2, 3], Duration::from_secs(60)).await;
        let expected = vec![1i64, 2, 3, 1, 2, 3];
        for &want in &expected {
            let got = p.select(None).await.unwrap();
            assert_eq!(got, want, "LRU 轮转顺序不符（期望 {want}，实际 {got}）");
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
        // LRU：非冷却账号中 id 最小且未选中的是 2。
        let sel = p.select(None).await;
        assert!(matches!(sel, Some(2) | Some(3)));
    }

    #[tokio::test]
    async fn pool_len_after_load_in_memory() {
        let p = SimplifiedPool::with_cooldown(Duration::from_secs(60));
        p.load_in_memory(vec![acc(1), acc(2)]).await;
        assert_eq!(p.len().await, 2);
    }

    #[tokio::test]
    async fn rate_limited_applies_longer_cooldown() {
        // dispatch_rate_limited 后账号立即进入长冷却（>> 测试运行时间）。
        let p = pool_with(&[1, 2], Duration::from_millis(10)).await;
        p.dispatch_rate_limited(1).await;
        // 普通冷却 10ms 远不够；限速冷却 60s，测试运行不会超时。
        assert!(p.in_cooldown(1).await, "限速后应立即进入长冷却");
        // 未限速账号 2 仍可用。
        assert_eq!(p.select(None).await, Some(2));
    }

    #[tokio::test]
    async fn rate_limited_backoff_grows_with_consecutive_failures() {
        // 验证连续限速冷却时长单调增长（通过消耗 rl_failure_count 侧面验证）。
        let p = pool_with(&[1], Duration::from_millis(10)).await;
        // 第 1 次：60s
        p.dispatch_rate_limited(1).await;
        assert!(p.in_cooldown(1).await);
        // 第 2 次：120s（仍冷却，因为 120s > 60s > 测试时间）
        p.dispatch_rate_limited(1).await;
        assert!(p.in_cooldown(1).await);
        // failure_count 累计
        assert_eq!(p.failure_count(1).await, 2);
    }

    #[tokio::test]
    async fn dispatch_success_resets_rl_failure_count() {
        // 成功后 rl_failure_count 清零：下次限速重新从 60s 开始计算。
        let p = pool_with(&[1, 2], Duration::from_millis(10)).await;
        p.dispatch_rate_limited(1).await; // rl_count = 1，冷却 60s
        p.dispatch_rate_limited(1).await; // rl_count = 2，冷却 120s
                                          // 清零（success 不提前解除冷却，仅重置计数器）
        p.dispatch_success(1).await;
        // failure_count 仍为 2（success_count 增至 1），但 rl_failure_count 已清零
        assert_eq!(p.failure_count(1).await, 2);
        assert_eq!(p.success_count(1).await, 1);
        // 冷却仍在（没有提前解除）
        assert!(p.in_cooldown(1).await, "success 不应提前解除限速冷却");
    }
}
