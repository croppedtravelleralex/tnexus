//! 账号 / Provider / 额度领域类型（骨架）。
//! 对照 Go `domain/account`；字段后续按 docs/39b 补齐。

use serde::{Deserialize, Serialize};

/// Provider 类型，对应 `grok_accounts.provider` CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    GrokBuild,
    GrokWeb,
    GrokConsole,
}

/// Web 调度/维护双轨（对齐 Go `WebLane`，见 web_pool_probe.go）。
///
/// G3 selector（号池 selector）按 lane 选择账号池：
/// - `Image` → grok-imagine 生图轨
/// - `Chat` → fast/auto 文本轨
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebLane {
    Image,
    Chat,
}

impl WebLane {
    pub fn as_str(self) -> &'static str {
        match self {
            WebLane::Image => "image",
            WebLane::Chat => "chat",
        }
    }
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::GrokBuild => "grok_build",
            Provider::GrokWeb => "grok_web",
            Provider::GrokConsole => "grok_console",
        }
    }
}

/// 账号认证状态，对应 `grok_accounts.auth_status`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatus {
    Unknown,
    Active,
    Restricted,
    Banned,
    /// 需重新授权（Go `ReauthRequired`）：Build 四池中视为可删（对齐 `AccountPoolAt`）。
    ReauthRequired,
}

/// 账号主表领域模型。
///
/// G0 最小字段见 docs/39b §3 表 3；G3 追加 selector 需要的健康/冷却/额度闸门字段。
/// 新字段（追加于 G0 之后）均带默认值，保证旧构造点 `..Default::default()` 可编译、
/// 反序列化缺省字段时优雅回退。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub identity_key: String,
    pub provider: Provider,
    pub enabled: bool,
    pub auth_status: AuthStatus,
    pub priority: i32,
    pub observed_model: Option<String>,

    // ── G3 selector 字段（对照 Go Credential，见 domain/account/account.go）──
    /// 该账号允许的最大并发数（Go `MaxConcurrent`，默认 8）。
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: i32,
    /// 额度余量保留阈值：低于此值视为不可调度（Go `MinimumRemaining`，默认 0）。
    #[serde(default)]
    pub minimum_remaining: i64,
    /// 连续失败次数（Go `FailureCount`）。>0 且 cooldown_until 未过 → 冷却中。
    #[serde(default)]
    pub failure_count: i32,
    /// 冷却截止时刻，未过则 account 不可调度（Go `CooldownUntil`）。
    #[serde(default)]
    pub cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
    /// 最近一次错误信息（Go `LastError`；`web_dead:` 前缀 → 判死池）。
    #[serde(default)]
    pub last_error: Option<String>,
    /// 记录本账号上最近一次真实模型请求的结果状态（G3 selector 优先据此判可用）。
    #[serde(default)]
    pub model_state: Option<ModelState>,
    /// 账号当前归属的调度轨。None 表示未知，需按模型解析（对齐 `ResolveWebAcquireLane`）。
    #[serde(default)]
    pub lane: Option<WebLane>,

    // ── G3-P3 Build 四池索引字段（对齐 Go `Credential.CreatedAt/UpdatedAt/LastUsedAt`）──
    /// 创建时刻（verification 池 due 基准）。
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 更新时刻（delete 池 due / dispatch 探针 due 基准）。
    #[serde(default)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 最近一次被调度选中时刻（dispatch 公平序）。
    #[serde(default)]
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_max_concurrent() -> i32 {
    // 对齐 Go `DefaultMaxConcurrent = 8`。
    8
}

impl Default for Account {
    fn default() -> Self {
        Self {
            id: 0,
            identity_key: String::new(),
            provider: Provider::GrokWeb,
            enabled: true,
            auth_status: AuthStatus::Active,
            priority: 0,
            observed_model: None,
            max_concurrent: default_max_concurrent(),
            minimum_remaining: 0,
            failure_count: 0,
            cooldown_until: None,
            last_error: None,
            model_state: None,
            lane: None,
            created_at: None,
            updated_at: None,
            last_used_at: None,
        }
    }
}

/// 额度来源（对齐 Go `QuotaSource`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    #[default]
    Default,
    Estimated,
    Upstream,
}

impl QuotaSource {
    pub fn as_str(self) -> &'static str {
        match self {
            QuotaSource::Default => "default",
            QuotaSource::Estimated => "estimated",
            QuotaSource::Upstream => "upstream",
        }
    }
}

/// 账号额度窗口（fast/auto/imagine），对应 `grok_quota_windows`（对齐 Go `QuotaWindow`）。
///
/// `total` 为窗口总额，`remaining` 为剩余，`reset_at` 为重置时刻。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub account_id: i64,
    /// 额度模式：fast / auto / imagine（Go `Mode`）。
    pub mode: String,
    pub remaining: i64,
    pub total: i64,
    #[serde(default)]
    pub reset_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 上游同步时刻；imagine 新鲜度（TTL 30min）据此判定（Go `SyncedAt`）。
    #[serde(default)]
    pub synced_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 额度来源；imagine fresh 仅认 `Upstream`（Go `Source`）。
    #[serde(default)]
    pub source: QuotaSource,
    /// 更新时间（Go `UpdatedAt`）。
    #[serde(default)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Default for QuotaWindow {
    fn default() -> Self {
        Self {
            account_id: 0,
            mode: String::new(),
            remaining: 0,
            total: 0,
            reset_at: None,
            synced_at: None,
            source: QuotaSource::default(),
            updated_at: chrono::DateTime::UNIX_EPOCH,
        }
    }
}

/// 配额扣减结果（G1 fast 额度验收用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaDeduction {
    pub account_id: i64,
    pub remaining_after: i64,
}

/// 账号在单个上游模型上的独立可用性（对照 Go `ModelState` + `ModelStatus`）。
///
/// G3 selector 优先按 `model_state.status` 判断账号在某模型上是否可用，不依赖全局 auth。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    #[default]
    Unknown,
    QuotaAvailable,
    Available,
    SoftStop,
    QuotaExhausted,
    AuthFailed,
    SignatureFailed,
}

impl ModelStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelStatus::Unknown => "unknown",
            ModelStatus::QuotaAvailable => "quota_available",
            ModelStatus::Available => "available",
            ModelStatus::SoftStop => "soft_stop",
            ModelStatus::QuotaExhausted => "quota_exhausted",
            ModelStatus::AuthFailed => "auth_failed",
            ModelStatus::SignatureFailed => "signature_failed",
        }
    }
}

/// 账号在单个上游模型上的结果状态（对照 Go `ModelState`）；额度次数由 `QuotaWindow` 独立保存。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelState {
    pub account_id: i64,
    pub upstream_model: String,
    #[serde(default)]
    pub status: ModelStatus,
    #[serde(default)]
    pub reason: Option<String>,
    /// 连续失败次数（Go `ConsecutiveFailures`）。
    #[serde(default)]
    pub consecutive_failures: i32,
    #[serde(default)]
    pub last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub last_success_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub cooldown_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// 额度恢复判别类别：Free 需真实流量探测，Paid（账户期）需 Billing 探测（Go `QuotaRecoveryKind`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaRecoveryKind {
    #[default]
    Free,
    Paid,
}

/// Free 额度耗尽后的持久化恢复状态（Go `QuotaRecoveryStatus`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaRecoveryStatus {
    #[default]
    Active,
    Exhausted,
    Probing,
}

/// 额度耗尽后的单次恢复探测状态（对照 Go `QuotaRecovery`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaRecovery {
    pub account_id: i64,
    #[serde(default)]
    pub kind: QuotaRecoveryKind,
    #[serde(default)]
    pub status: QuotaRecoveryStatus,
    #[serde(default)]
    pub confirmed_used: i64,
    #[serde(default)]
    pub confirmed_limit: i64,
    #[serde(default)]
    pub exhausted_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub next_probe_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub last_confirmed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for QuotaRecovery {
    fn default() -> Self {
        Self {
            account_id: 0,
            kind: QuotaRecoveryKind::Free,
            status: QuotaRecoveryStatus::Active,
            confirmed_used: 0,
            confirmed_limit: 0,
            exhausted_at: None,
            next_probe_at: None,
            last_confirmed_at: None,
        }
    }
}

/// 账号最近一次额度快照（对照 Go `Billing`）。G3 selector 在 Paid 恢复/额度未知时参考。
///
/// 字段裁剪为 selector 决策所需的 min：月额度上限、已用、按需封顶、统一账单标记、账期字符串。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Billing {
    pub account_id: i64,
    /// 上游计划代码（plan_code）。
    pub plan_code: String,
    /// 月度额度上限（monthly_limit）。
    pub monthly_limit: f64,
    /// 本月已用（used）。
    pub used: f64,
    /// 按需额度（封顶）cap。
    pub on_demand_cap: f64,
    /// 按需已用（on_demand_used）。
    pub on_demand_used: f64,
    /// 预付余额。>0 表示可脱离月限额继续服务。
    pub prepaid_balance: f64,
    /// 统一账单用户标记，影响 IsExhausted 判定。
    pub is_unified_billing_user: bool,
    /// 账期类型（usage_period_type）。
    pub usage_period_type: String,
    /// 本期起止（RFC3339 字符串，解析出账期结束用）。
    pub usage_period_start: String,
    pub usage_period_end: String,
}

impl Default for Billing {
    fn default() -> Self {
        Self {
            account_id: 0,
            plan_code: String::new(),
            monthly_limit: 0.0,
            used: 0.0,
            on_demand_cap: 0.0,
            on_demand_used: 0.0,
            prepaid_balance: 0.0,
            is_unified_billing_user: false,
            usage_period_type: String::new(),
            usage_period_start: String::new(),
            usage_period_end: String::new(),
        }
    }
}

impl Billing {
    /// 当前剩余额度（负向钳制为 0，对照 Go `Billing.Remaining()`）。
    pub fn remaining(&self) -> f64 {
        let remaining = self.monthly_limit - self.used;
        if remaining < 0.0 {
            return 0.0;
        }
        remaining
    }

    /// 是否已达保留阈值（对照 Go `Billing.IsExhausted(minimum)`）。
    pub fn is_exhausted(&self, minimum: f64) -> bool {
        if self.monthly_limit > 0.0 && self.remaining() <= minimum {
            return true;
        }
        self.credit_usage_percent() >= 100.0
            && (self.on_demand_cap > 0.0 || !self.usage_period_type.is_empty())
    }

    /// 信用使用百分比（约 100 时进入 exhausted 判定）；域内维持简单近似。
    pub fn credit_usage_percent(&self) -> f64 {
        if self.monthly_limit <= 0.0 {
            return 0.0;
        }
        ((self.used / self.monthly_limit) * 100.0).min(100.0)
    }
}

/// 聚合账号选择热路径所需的持久化快照（对照 Go `RoutingCandidate`）。
///
/// G3 selector 输入：绑定 Billing / QuotaRecovery / ModelState 以判定候选可用性与优先级。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingCandidate {
    pub account: Account,
    pub billing: Option<Billing>,
    pub quota: Option<QuotaWindow>,
    pub recovery: Option<QuotaRecovery>,
    pub model_state: Option<ModelState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_defaults_are_sane_for_selector() {
        let a = Account::default();
        assert_eq!(a.max_concurrent, 8);
        assert_eq!(a.minimum_remaining, 0);
        assert_eq!(a.failure_count, 0);
        assert!(a.cooldown_until.is_none());
        assert!(a.last_error.is_none());
        assert!(a.model_state.is_none());
        assert!(a.lane.is_none());
    }

    #[test]
    fn account_defaults_back_compat_with_legacy_json() {
        // 旧构造点只写 G0 字段，序列化后反序列化应回填 G3 默认值。
        let legacy = Account {
            id: 1,
            identity_key: "web-1".into(),
            provider: Provider::GrokWeb,
            enabled: true,
            auth_status: AuthStatus::Active,
            priority: 0,
            observed_model: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&legacy).unwrap();
        let round: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(round.id, 1);
        assert_eq!(round.max_concurrent, 8);
        assert!(round.model_state.is_none());
    }

    #[test]
    fn account_full_serde_round_trip() {
        let a = Account {
            id: 7,
            identity_key: "web-7".into(),
            provider: Provider::GrokWeb,
            enabled: true,
            auth_status: AuthStatus::Active,
            priority: 3,
            observed_model: Some("grok-3".into()),
            max_concurrent: 64,
            minimum_remaining: 2,
            failure_count: 1,
            cooldown_until: Some(chrono::Utc::now()),
            last_error: Some("quota".into()),
            model_state: Some(ModelState {
                account_id: 7,
                upstream_model: "grok-imagine".into(),
                status: ModelStatus::Available,
                reason: None,
                consecutive_failures: 0,
                last_attempt_at: Some(chrono::Utc::now()),
                last_success_at: None,
                cooldown_until: None,
            }),
            lane: Some(WebLane::Image),
            created_at: None,
            updated_at: None,
            last_used_at: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_concurrent, 64);
        assert_eq!(back.minimum_remaining, 2);
        assert_eq!(back.model_state.as_ref().unwrap().status, ModelStatus::Available);
        assert_eq!(back.lane, Some(WebLane::Image));
    }

    #[test]
    fn model_state_default_and_as_str() {
        let ms = ModelState::default();
        assert_eq!(ms.status, ModelStatus::Unknown);
        assert_eq!(ms.consecutive_failures, 0);
        assert_eq!(ModelStatus::SoftStop.as_str(), "soft_stop");
    }

    #[test]
    fn billing_remaining_clamps_and_exhaustion() {
        let mut b = Billing {
            monthly_limit: 100.0,
            used: 40.0,
            ..Default::default()
        };
        assert_eq!(b.remaining(), 60.0);
        // 负向钳制
        b.used = 200.0;
        assert_eq!(b.remaining(), 0.0);
        assert!(b.is_exhausted(0.0));
    }

    #[test]
    fn recovery_default_and_serde_round_trip() {
        let r = QuotaRecovery::default();
        assert_eq!(r.status, QuotaRecoveryStatus::Active);
        let json = serde_json::to_string(&r).unwrap();
        let back: QuotaRecovery = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, QuotaRecoveryKind::Free);
    }

    #[test]
    fn routing_candidate_serde_round_trip() {
        let c = RoutingCandidate {
            account: Account::default(),
            billing: Some(Billing::default()),
            recovery: Some(QuotaRecovery::default()),
            model_state: Some(ModelState::default()),
            quota: None,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: RoutingCandidate = serde_json::from_str(&json).unwrap();
        assert!(back.billing.is_some());
        assert!(back.recovery.is_some());
        assert!(back.quota.is_none());
    }

    #[test]
    fn lane_as_str() {
        assert_eq!(WebLane::Image.as_str(), "image");
        assert_eq!(WebLane::Chat.as_str(), "chat");
    }
}
