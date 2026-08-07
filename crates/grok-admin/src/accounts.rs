//! 账号管理端点域（G4-P2，对齐 Go `transport/http/account/handler.go` +
//! `application/account/service.go` 的账号/额度/模型状态管理子集）。
//!
//! 纯编排层：校验 + 错误映射（NotFound / InvalidFilter / InvalidRequest），
//! 持久化全部经 [`AdminStore`] trait（后续接 grok-storage `PgAccountRepository`，
//! 测试注入内存 fake）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use grok_domain::{Account, AuthStatus, ModelState, Provider, QuotaSource, QuotaWindow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{AdminError, AdminResult};

/// 账号列表视图（对齐 Go `accountResponse` 子集，字段名 snake_case）。
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: i64,
    pub provider: String,
    pub name: String,
    pub enabled: bool,
    pub auth_status: String,
    pub priority: i32,
    pub observed_model: Option<String>,
    pub max_concurrent: i32,
    pub failure_count: i32,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<&Account> for AccountView {
    fn from(account: &Account) -> Self {
        Self {
            id: account.id,
            provider: account.provider.as_str().to_string(),
            name: account.name.clone(),
            enabled: account.enabled,
            auth_status: auth_status_str(account.auth_status).to_string(),
            priority: account.priority,
            observed_model: account.observed_model.clone(),
            max_concurrent: account.max_concurrent,
            failure_count: account.failure_count,
            cooldown_until: account.cooldown_until,
            last_error: account.last_error.clone(),
            created_at: account.created_at,
            updated_at: account.updated_at,
        }
    }
}

/// 账号详情（账号 + 额度窗口 + 模型状态，对齐 Go `get` 的完整视图）。
#[derive(Debug, Clone, Serialize)]
pub struct AccountDetail {
    #[serde(flatten)]
    pub account: AccountView,
    pub quota_windows: Vec<QuotaWindow>,
    pub model_states: Vec<ModelState>,
}

/// 账号更新输入（对齐 Go `updateRequest` 子集：enabled/auth_status/priority/cooldown）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateAccountInput {
    pub enabled: Option<bool>,
    pub auth_status: Option<String>,
    pub priority: Option<i32>,
    pub cooldown_until: Option<DateTime<Utc>>,
}

/// 列表过滤（对齐 Go `ListFilter` 的 provider/enabled/auth_status 维度）。
#[derive(Debug, Clone, Default)]
pub struct AccountListFilter {
    pub provider: Option<Provider>,
    pub enabled: Option<bool>,
    pub auth_status: Option<AuthStatus>,
}

/// 分页结果（对齐 Go `list` 响应 items/page/pageSize/total）。
#[derive(Debug, Clone)]
pub struct AccountPage {
    pub items: Vec<AccountView>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
}

/// 批量导入单条输入（对齐 Go `accounts/import` 的逐行 JSON）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportAccountInput {
    /// 必填：唯一身份键。
    pub identity_key: String,
    /// 必填：grok_build / grok_web / grok_console。
    pub provider: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub max_concurrent: Option<i32>,
    /// 可选：加密凭据（写入 grok_credentials.encrypted_primary）。
    #[serde(default)]
    pub credential: Option<String>,
}

/// 单条导入失败（对齐 Go 逐条错误 `{index, reason}`）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportError {
    pub index: usize,
    pub reason: String,
}

/// 导入结果（对齐 Go `accounts/import` 响应）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportResult {
    pub imported: i64,
    pub failed: i64,
    pub errors: Vec<ImportError>,
}

/// 每日聚合点（可视化面板数据源；对齐 Go `analytics/timeseries`）。
#[derive(Debug, Clone, Serialize)]
pub struct TimeseriesPoint {
    pub date: String,
    pub requests: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub latency_p50_ms: i64,
}

/// Top 账号视图（对齐 Go `analytics/top-accounts`）。
#[derive(Debug, Clone, Serialize)]
pub struct TopAccountView {
    pub account_id: i64,
    pub name: String,
    pub requests: i64,
    pub failed: i64,
    /// 失败率 0.0–1.0。
    pub failure_rate: f64,
}

/// 额度窗口写入输入（对齐 Go `quotaWindowResponse` 的可写子集）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct QuotaWindowInput {
    pub mode: String,
    pub remaining: i64,
    pub total: i64,
    #[serde(default)]
    pub reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub synced_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub source: Option<String>,
}

/// 账号管理存储抽象（后续由 grok-storage `PgAccountRepository` 实现）。
#[async_trait]
pub trait AdminStore: Send + Sync {
    /// 分页列表（对齐 Go `AccountRepository.List` 的过滤 + total 语义）。
    async fn list_accounts(
        &self,
        filter: &AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<AccountPage>;
    /// 单账号（None = 未找到）。
    async fn get_account(&self, id: i64) -> AdminResult<Option<Account>>;
    /// 更新并返回更新后的账号（None = 未找到）。
    async fn update_account(
        &self,
        id: i64,
        input: &UpdateAccountInput,
    ) -> AdminResult<Option<Account>>;
    /// 删除账号；`Ok(true)` = 已删除，`Ok(false)` = 不存在。
    async fn delete_account(&self, id: i64) -> AdminResult<bool>;
    /// 账号的全部额度窗口（Go `GetQuotaWindows`）。
    async fn list_quota_windows(&self, account_id: i64) -> AdminResult<Vec<QuotaWindow>>;
    /// 写回（按 (account_id, mode) upsert，对齐 Go `SaveQuotaWindows`）。
    async fn upsert_quota_window(&self, window: QuotaWindow) -> AdminResult<QuotaWindow>;
    /// 账号的全部模型状态（Go `GetModelStates`）。
    async fn list_model_states(&self, account_id: i64) -> AdminResult<Vec<ModelState>>;

    // ── G6 运维端点（39g §1.2 缺失项）──────────────────────────
    /// 池规模汇总（对齐 Go `accounts/summary`）。
    async fn pool_summary(&self) -> AdminResult<AccountSummary>;
    /// 账号分析（对齐 Go `accounts/analytics`）。
    async fn analytics(&self) -> AdminResult<AccountAnalytics>;
    /// 单账号 billing 探测；`Ok(false)` = 账号不存在（Go `refresh-billing`）。
    async fn refresh_billing(&self, account_id: i64) -> AdminResult<bool>;
    /// 单账号 quota 刷新；`Ok(false)` = 账号不存在（Go `refresh-quota`）。
    async fn refresh_quota(&self, account_id: i64) -> AdminResult<bool>;
    /// 单账号 token 刷新；`Ok(false)` = 账号不存在（Go `refresh-token`）。
    async fn refresh_token(&self, account_id: i64) -> AdminResult<bool>;
    /// 触发重登；`Ok(false)` = 账号不存在（Go `reauth`）。
    async fn reauth(&self, account_id: i64) -> AdminResult<bool>;

    // ── G6 批量导入 + 可视化聚合（39g §1.2 缺失项）──────────────
    /// 批量导入；返回成功/失败明细（Go `accounts/import`）。
    async fn import_accounts(&self, inputs: &[ImportAccountInput]) -> AdminResult<ImportResult>;
    /// 按天聚合请求统计（近 `days` 天；无数据返回空数组）。
    async fn timeseries(&self, days: i64) -> AdminResult<Vec<TimeseriesPoint>>;
    /// 按请求量/失败率取 Top 账号。
    async fn top_accounts(&self, limit: i64) -> AdminResult<Vec<TopAccountView>>;
}

/// 池规模汇总（对齐 Go `accounts/summary`；按 provider × 池态计数）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountSummary {
    /// 每 provider 的账号总数。
    pub total: i64,
    /// 已启用且 Active。
    pub available: i64,
    /// 冷却中（cooldown_until > now）。
    pub cooldown: i64,
    /// 需重新授权（reauthRequired）。
    pub reauth_required: i64,
    /// 手动禁用。
    pub disabled: i64,
    /// 探针中（failure_count > 0 且冷却未过）。
    pub probing: i64,
    /// 额度已耗尽（remaining <= 0 且 total > 0 的窗口数）。
    pub quota_exhausted: i64,
    /// 各 provider 明细。
    pub by_provider: HashMap<String, ProviderSummary>,
}

/// 单 provider 明细（对齐 Go summary 的 provider 分组）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProviderSummary {
    pub total: i64,
    pub available: i64,
    pub cooldown: i64,
    pub reauth_required: i64,
    pub disabled: i64,
}

/// 账号分析（对齐 Go `accounts/analytics`；额度状态分布）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountAnalytics {
    /// 额度已知且未耗尽。
    pub quota_known: i64,
    /// 额度已耗尽。
    pub quota_exhausted: i64,
    /// 额度未知（无窗口或 0/0）。
    pub quota_unknown: i64,
    /// 有 billing 快照的账号数。
    pub billing_count: i64,
    /// 各模型观察到的账号数。
    pub by_model: HashMap<String, i64>,
}

/// 账号管理服务：校验 + 编排（无 IO 细节）。
pub struct AccountAdminService {
    store: Arc<dyn AdminStore>,
}

impl AccountAdminService {
    pub fn new(store: Arc<dyn AdminStore>) -> Self {
        Self { store }
    }

    /// 列表：分页参数规范化 + 过滤 + total（对齐 Go `List`）。
    pub async fn list(
        &self,
        filter: AccountListFilter,
        page: i64,
        page_size: i64,
    ) -> AdminResult<AccountPage> {
        let page = page.max(1);
        let page_size = if (1..=100).contains(&page_size) {
            page_size
        } else {
            20
        };
        self.store.list_accounts(&filter, page, page_size).await
    }

    /// 详情：账号 + 额度窗口 + 模型状态（对齐 Go `Get`）。
    pub async fn get(&self, id: i64) -> AdminResult<AccountDetail> {
        let account = self
            .store
            .get_account(id)
            .await?
            .ok_or_else(|| AdminError::NotFound(format!("account {id}")))?;
        let quota_windows = self.store.list_quota_windows(id).await?;
        let model_states = self.store.list_model_states(id).await?;
        Ok(AccountDetail {
            account: AccountView::from(&account),
            quota_windows,
            model_states,
        })
    }

    /// 更新（对齐 Go `Update`；auth_status 需为合法值）。
    pub async fn update(&self, id: i64, input: &UpdateAccountInput) -> AdminResult<AccountView> {
        if let Some(raw) = &input.auth_status {
            parse_auth_status(raw)?;
        }
        let account = self
            .store
            .update_account(id, input)
            .await?
            .ok_or_else(|| AdminError::NotFound(format!("account {id}")))?;
        Ok(AccountView::from(&account))
    }

    /// 删除（对齐 Go `Delete`；不存在 → NotFound）。
    pub async fn delete(&self, id: i64) -> AdminResult<()> {
        let deleted = self.store.delete_account(id).await?;
        if !deleted {
            return Err(AdminError::NotFound(format!("account {id}")));
        }
        Ok(())
    }

    /// 账号额度窗口（对齐 Go `Get` 的 QuotaWindows 部分）。
    pub async fn quota_windows(&self, id: i64) -> AdminResult<Vec<QuotaWindow>> {
        if self.store.get_account(id).await?.is_none() {
            return Err(AdminError::NotFound(format!("account {id}")));
        }
        self.store.list_quota_windows(id).await
    }

    /// 写回额度窗口（对齐 Go `SaveQuotaWindows`；mode 必填、负值拒绝）。
    pub async fn upsert_quota(
        &self,
        id: i64,
        input: &QuotaWindowInput,
    ) -> AdminResult<QuotaWindow> {
        let mode = input.mode.trim().to_string();
        if mode.is_empty() {
            return Err(AdminError::InvalidRequest("mode 不能为空".into()));
        }
        if input.remaining < 0 || input.total < 0 {
            return Err(AdminError::InvalidRequest(
                "remaining/total 不能为负".into(),
            ));
        }
        if self.store.get_account(id).await?.is_none() {
            return Err(AdminError::NotFound(format!("account {id}")));
        }
        let source = match &input.source {
            None => QuotaSource::default(),
            Some(raw) => parse_quota_source(raw)?,
        };
        let window = QuotaWindow {
            account_id: id,
            mode,
            remaining: input.remaining,
            total: input.total,
            reset_at: input.reset_at,
            synced_at: input.synced_at,
            source,
            updated_at: Utc::now(),
        };
        self.store.upsert_quota_window(window).await
    }

    /// 账号模型状态（对齐 Go `GetModelStates`）。
    pub async fn model_states(&self, id: i64) -> AdminResult<Vec<ModelState>> {
        if self.store.get_account(id).await?.is_none() {
            return Err(AdminError::NotFound(format!("account {id}")));
        }
        self.store.list_model_states(id).await
    }

    /// 池规模汇总（对齐 Go `accounts/summary`）。
    pub async fn summary(&self) -> AdminResult<AccountSummary> {
        self.store.pool_summary().await
    }

    /// 账号分析（对齐 Go `accounts/analytics`）。
    pub async fn analytics(&self) -> AdminResult<AccountAnalytics> {
        self.store.analytics().await
    }

    /// 批量导入：逐条字段级校验（identity_key ≤64 / provider 枚举 / name ≤160 /
    /// priority 0..=1000 / max_concurrent 1..=256），超限记 error 条目不 panic；
    /// 校验通过项委托 store 落库（冲突等 store 级错误按原输入下标映射回）。
    pub async fn import(&self, inputs: &[ImportAccountInput]) -> AdminResult<ImportResult> {
        // 空数组 → 直接返回空结果（不调用 store）。
        if inputs.is_empty() {
            return Ok(ImportResult::default());
        }
        let mut result = ImportResult::default();
        let mut valid: Vec<ImportAccountInput> = Vec::with_capacity(inputs.len());
        let mut orig_index: Vec<usize> = Vec::with_capacity(inputs.len());
        for (index, input) in inputs.iter().enumerate() {
            match validate_import_input(input) {
                Some(reason) => {
                    result.failed += 1;
                    result.errors.push(ImportError { index, reason });
                }
                None => {
                    valid.push(input.clone());
                    orig_index.push(index);
                }
            }
        }
        if valid.is_empty() {
            return Ok(result);
        }
        let store_result = self.store.import_accounts(&valid).await?;
        result.imported += store_result.imported;
        result.failed += store_result.failed;
        for e in store_result.errors {
            result.errors.push(ImportError {
                index: orig_index.get(e.index).copied().unwrap_or(e.index),
                reason: e.reason,
            });
        }
        Ok(result)
    }

    /// 近 `days` 天每日聚合（可视化面板数据源）。
    pub async fn timeseries(&self, days: i64) -> AdminResult<Vec<TimeseriesPoint>> {
        let days = days.clamp(1, 90);
        self.store.timeseries(days).await
    }

    /// Top 账号（按请求量降序；请求量为 0 时按失败率降序）。
    pub async fn top_accounts(&self, limit: i64) -> AdminResult<Vec<TopAccountView>> {
        self.store.top_accounts(limit.clamp(1, 50)).await
    }

    /// 运维动作：单账号 billing 探测 / quota 刷新 / token 刷新 / 重登。
    /// 账号不存在 → NotFound；动作委托给 store（SQL / grok-ops backend 留 TODO）。
    pub async fn refresh_billing(&self, id: i64) -> AdminResult<()> {
        self.require_account(id).await?;
        self.store.refresh_billing(id).await?;
        Ok(())
    }

    pub async fn refresh_quota(&self, id: i64) -> AdminResult<()> {
        self.require_account(id).await?;
        self.store.refresh_quota(id).await?;
        Ok(())
    }

    pub async fn refresh_token(&self, id: i64) -> AdminResult<()> {
        self.require_account(id).await?;
        self.store.refresh_token(id).await?;
        Ok(())
    }

    pub async fn reauth(&self, id: i64) -> AdminResult<()> {
        self.require_account(id).await?;
        self.store.reauth(id).await?;
        Ok(())
    }

    async fn require_account(&self, id: i64) -> AdminResult<()> {
        if self.store.get_account(id).await?.is_none() {
            return Err(AdminError::NotFound(format!("account {id}")));
        }
        Ok(())
    }
}

/// AuthStatus 输出字符串（对齐 Go `Credential.AuthStatus` 序列化）。
pub fn auth_status_str(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::Unknown => "unknown",
        AuthStatus::Active => "active",
        AuthStatus::Restricted => "restricted",
        AuthStatus::Banned => "banned",
        AuthStatus::ReauthRequired => "reauthRequired",
    }
}

/// AuthStatus 解析（接受 DB 的 `reauthRequired` 与 Rust 序列化的 `reauth_required`）。
pub fn parse_auth_status(raw: &str) -> AdminResult<AuthStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "unknown" => Ok(AuthStatus::Unknown),
        "active" => Ok(AuthStatus::Active),
        "restricted" => Ok(AuthStatus::Restricted),
        "banned" => Ok(AuthStatus::Banned),
        "reauthrequired" | "reauth_required" => Ok(AuthStatus::ReauthRequired),
        other => Err(AdminError::InvalidRequest(format!(
            "无效 auth_status: {other}"
        ))),
    }
}

/// Provider 解析（导入输入用；未知 → None 而非 Err，便于逐条计数）。
pub fn parse_provider(raw: &str) -> Option<Provider> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "grok_build" | "build" => Some(Provider::GrokBuild),
        "grok_web" | "web" => Some(Provider::GrokWeb),
        "grok_console" | "console" => Some(Provider::GrokConsole),
        _ => None,
    }
}

/// QuotaSource 解析（对齐 DB CHECK：default/estimated/upstream）。
pub fn parse_quota_source(raw: &str) -> AdminResult<QuotaSource> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "default" => Ok(QuotaSource::Default),
        "estimated" => Ok(QuotaSource::Estimated),
        "upstream" => Ok(QuotaSource::Upstream),
        other => Err(AdminError::InvalidRequest(format!("无效额度来源: {other}"))),
    }
}
/// 逐条字段级校验（对齐 Go 账号字段约束与 grok_accounts schema CHECK）。
/// 返回错误描述；`None` = 通过。
pub(crate) fn validate_import_input(input: &ImportAccountInput) -> Option<String> {
    let identity_key = input.identity_key.trim();
    if identity_key.is_empty() {
        return Some("identity_key 不能为空".into());
    }
    if identity_key.len() > 64 {
        return Some(format!("identity_key 超长(>64): {identity_key}"));
    }
    if parse_provider(&input.provider).is_none() {
        return Some(format!("unknown provider: {}", input.provider));
    }
    if let Some(name) = &input.name {
        if name.trim().len() > 160 {
            return Some("name 超长(>160)".into());
        }
    }
    if let Some(priority) = input.priority {
        if !(0..=1000).contains(&priority) {
            return Some(format!("priority 超出范围(0..=1000): {priority}"));
        }
    }
    if let Some(mc) = input.max_concurrent {
        if !(1..=256).contains(&mc) {
            return Some(format!("max_concurrent 超出范围(1..=256): {mc}"));
        }
    }
    None
}
