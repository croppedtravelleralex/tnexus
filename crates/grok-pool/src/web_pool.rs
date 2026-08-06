//! Web 图池选择纯函数（对齐 Go `account/web_pool.go`）。
//!
//! 只含可脱离 Service/仓库测试的判定与排序逻辑；Service 接线（`ReconcileWebPools`、
//! `WebPools`、`SyncImageDispatchPins`）属 G3-P5 仓库集成，此处不实现。

use chrono::{DateTime, Utc};

use grok_domain::{
    imagine_quota::{
        imagine_dispatch_quota_admissible, imagine_known_quota_fresh, imagine_quota_exhausted,
        imagine_quota_unknown_fresh, imagine_window_upstream_fresh,
    },
    ModelState, ModelStatus, QuotaWindow,
};

pub const WEB_IMAGE_POOL_CAP: usize = 50;
pub const WEB_CHAT_POOL_CAP: usize = 50;
pub const IMAGINE_UPSTREAM: &str = "grok-imagine-image";

/// Web 图池候选（对齐 Go `webPoolCandidate`）。
#[derive(Debug, Clone, Default)]
pub struct WebPoolCandidate {
    pub id: i64,
    pub priority: i32,
    pub fast_rem: i64,
    pub auto_rem: i64,
    pub imagine_window: Option<QuotaWindow>,
    pub model_state: Option<ModelState>,
    pub enabled: bool,
    pub active: bool,
    /// 账号级 cooldown（`cooldown_until` 未过）。
    pub cooling: bool,
    /// grok-imagine-image 模型 block（只挡图池）。
    pub imagine_blocked: bool,
}

/// Web 调度选池入参（对齐 Go `WebPoolContext`，subset：仅用到的字段）。
#[derive(Debug, Clone, Default)]
pub struct WebPoolContext {
    pub id: i64,
    pub priority: i32,
    pub fast_rem: i64,
    pub auto_rem: i64,
    pub enabled: bool,
    pub active: bool,
    /// 账号级 cooldown（`cooldown_until` 未过）。
    pub cooling: bool,
    pub imagine_window: Option<QuotaWindow>,
    pub model_state: Option<ModelState>,
    pub imagine_blocked: bool,
}

impl WebPoolContext {
    fn as_candidate(&self) -> WebPoolCandidate {
        WebPoolCandidate {
            id: self.id,
            priority: self.priority,
            fast_rem: self.fast_rem,
            auto_rem: self.auto_rem,
            imagine_window: self.imagine_window.clone(),
            model_state: self.model_state.clone(),
            enabled: self.enabled,
            active: self.active,
            cooling: self.cooling,
            imagine_blocked: self.imagine_blocked,
        }
    }
}

/// 从 `fast`/`auto` 等额度窗口列表中取某 mode 的剩余额度；无则 0。
pub fn quota_remaining(windows: &[QuotaWindow], mode: &str) -> i64 {
    windows
        .iter()
        .find(|w| w.mode == mode)
        .map(|w| w.remaining)
        .unwrap_or(0)
}

/// 取指定 mode 的额度窗口引用。
pub fn find_quota_window<'a>(windows: &'a [QuotaWindow], mode: &str) -> Option<&'a QuotaWindow> {
    windows.iter().find(|w| w.mode == mode)
}

/// 取指定 upstream model 的模型状态引用。
pub fn find_model_state<'a>(
    states: &'a [ModelState],
    upstream_model: &str,
) -> Option<&'a ModelState> {
    states.iter().find(|s| s.upstream_model == upstream_model)
}

/// 是否应由 dispatch 放行图号（对齐 Go `imageDispatchAdmissible`，并要求健康 modelState）。
pub fn image_dispatch_admissible(candidate: &WebPoolCandidate, now: DateTime<Utc>) -> bool {
    if !candidate.enabled || !candidate.active || candidate.cooling {
        return false;
    }
    if candidate.imagine_blocked {
        let positive = candidate
            .imagine_window
            .as_ref()
            .map(|w| grok_domain::imagine_quota::imagine_quota_known_positive(w.total, w.remaining))
            .unwrap_or(false);
        if !positive {
            return false;
        }
    }
    if !imagine_dispatch_quota_admissible(
        candidate.imagine_window.as_ref(),
        candidate.model_state.as_ref(),
        now,
    ) {
        return false;
    }
    let Some(state) = &candidate.model_state else {
        return false;
    };
    matches!(
        state.status,
        ModelStatus::Available | ModelStatus::QuotaAvailable | ModelStatus::Unknown
    )
}

/// 图池验收资格（对齐 Go `imagePoolEligible`）。比 `imageDispatchAdmissible` 宽松。
pub fn image_pool_eligible(candidate: &WebPoolCandidate, now: DateTime<Utc>) -> bool {
    if !candidate.enabled || !candidate.active || candidate.cooling {
        return false;
    }
    let window: Option<&QuotaWindow> = candidate.imagine_window.as_ref();
    let positive = window
        .map(|w| grok_domain::imagine_quota::imagine_quota_known_positive(w.total, w.remaining))
        .unwrap_or(false);
    if candidate.imagine_blocked && !positive {
        return false;
    }
    if let Some(w) = window {
        if imagine_quota_exhausted(w.total, w.remaining) {
            return false;
        }
        if positive
            && candidate
                .model_state
                .as_ref()
                .is_some_and(|s| s.status == ModelStatus::QuotaExhausted)
        {
            return imagine_known_quota_fresh(w, now);
        }
    }
    let fresh_known = window.is_some_and(|w| imagine_known_quota_fresh(w, now));
    let fresh_unknown = window.is_some_and(|w| imagine_quota_unknown_fresh(w, now));
    match candidate.model_state.as_ref() {
        None => fresh_known,
        Some(state) => match state.status {
            ModelStatus::AuthFailed
            | ModelStatus::SignatureFailed
            | ModelStatus::QuotaExhausted => false,
            ModelStatus::SoftStop => {
                if state.cooldown_until.is_some_and(|c| c > now) {
                    return false;
                }
                fresh_known
            }
            ModelStatus::Available => match window {
                Some(w) if imagine_known_quota_fresh(w, now) => true,
                _ => fresh_unknown,
            },
            ModelStatus::QuotaAvailable | ModelStatus::Unknown => fresh_known || fresh_unknown,
        },
    }
}

/// 图池排序秩（对齐 Go `imagePoolRank`）。
fn image_pool_rank(candidate: &WebPoolCandidate, now: DateTime<Utc>) -> i32 {
    let fresh = |w: &QuotaWindow| imagine_known_quota_fresh(w, now);
    let available = || {
        candidate
            .model_state
            .as_ref()
            .is_some_and(|s| s.status == ModelStatus::Available)
    };
    let quota_available = || {
        candidate
            .model_state
            .as_ref()
            .is_some_and(|s| s.status == ModelStatus::QuotaAvailable)
    };
    if candidate.imagine_window.as_ref().is_some_and(fresh) {
        return if available() { 3 } else { 2 };
    }
    if available() {
        return 2;
    }
    if let Some(w) = &candidate.imagine_window {
        if w.total > 0 && w.remaining > 0 {
            return 1;
        }
    }
    if quota_available() {
        return 1;
    }
    0
}

/// 图池优先序比较（对齐 Go `imagePoolLess`）。
pub fn image_pool_less(a: &WebPoolCandidate, b: &WebPoolCandidate, now: DateTime<Utc>) -> bool {
    let fresh_a = a
        .imagine_window
        .as_ref()
        .map(|w| imagine_window_upstream_fresh(w, now))
        .unwrap_or(false);
    let fresh_b = b
        .imagine_window
        .as_ref()
        .map(|w| imagine_window_upstream_fresh(w, now))
        .unwrap_or(false);
    if fresh_a != fresh_b {
        return fresh_a;
    }
    if a.priority != b.priority {
        return a.priority > b.priority;
    }
    let rank_a = image_pool_rank(a, now);
    let rank_b = image_pool_rank(b, now);
    if rank_a != rank_b {
        return rank_a > rank_b;
    }
    let rem_a = a.imagine_window.as_ref().map(|w| w.remaining).unwrap_or(0);
    let rem_b = b.imagine_window.as_ref().map(|w| w.remaining).unwrap_or(0);
    if rem_a != rem_b {
        return rem_a > rem_b;
    }
    a.id < b.id
}

/// 过滤 + 稳定排序 + cap 截断，返回选中的 id 列表（对齐 Go `selectWebPoolIDs`）。
///
/// `less(a, b)` 返回 true 表示 a 排在 b 前（升序语义下的"更优先"）。
pub fn select_web_pool_ids<F, G>(
    candidates: &[WebPoolCandidate],
    cap: usize,
    eligible: F,
    less: G,
) -> Vec<i64>
where
    F: Fn(&WebPoolCandidate) -> bool,
    G: Fn(&WebPoolCandidate, &WebPoolCandidate) -> bool,
{
    let mut filtered: Vec<&WebPoolCandidate> = candidates.iter().filter(|c| eligible(c)).collect();
    filtered.sort_by(|a, b| {
        if less(a, b) {
            std::cmp::Ordering::Less
        } else if less(b, a) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    if filtered.len() > cap {
        filtered.truncate(cap);
    }
    filtered.into_iter().map(|c| c.id).collect()
}

/// 图池入池保留判定（对齐 Go `imageAccountRetained`）。
///
/// 本实现不依赖 `webImagePoolAt`（属 G3-P3 四池归类）：仅要求图池 eligible，
/// 即「不在净删除池」的充分可留在池判定。Service 接线时再对齐四池细分。
pub fn image_account_retained(input: &WebPoolContext, now: DateTime<Utc>) -> bool {
    image_pool_eligible(&input.as_candidate(), now)
}
