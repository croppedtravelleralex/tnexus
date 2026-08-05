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
            return Err(AdminError::InvalidRequest("remaining/total 不能为负".into()));
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

/// QuotaSource 解析（对齐 DB CHECK：default/estimated/upstream）。
pub fn parse_quota_source(raw: &str) -> AdminResult<QuotaSource> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "default" => Ok(QuotaSource::Default),
        "estimated" => Ok(QuotaSource::Estimated),
        "upstream" => Ok(QuotaSource::Upstream),
        other => Err(AdminError::InvalidRequest(format!(
            "无效额度来源: {other}"
        ))),
    }
}
