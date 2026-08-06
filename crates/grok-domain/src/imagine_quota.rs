//! Imagine 额度闸门判定（对齐 Go `domain/account/imagine_quota.go`）。
//!
//! Web 生图轨（grok-imagine-image）的额度判断与 text 轨不同：上游 free-usage-gates
//! 可能返回 0/0（闸门不适用或上限未知，≠耗尽）。本模块集中这些判定，供 G3 selector
//! 与池准入复用。

use chrono::{DateTime, Utc};

use crate::account::{ModelState, ModelStatus, QuotaSource, QuotaWindow};

pub const MODE_IMAGINE: &str = "imagine";

/// Imagine 闸门同步与 Lite 成功证据的有效窗口。
pub const IMAGINE_QUOTA_FRESH_TTL_SECS: i64 = 30 * 60;

fn ttl() -> chrono::Duration {
    chrono::Duration::seconds(IMAGINE_QUOTA_FRESH_TTL_SECS)
}

/// free-usage-gates 返回 0/0：闸门不适用或上限未知，不等于耗尽。
pub fn imagine_quota_limit_unknown(total: i64, remaining: i64) -> bool {
    total == 0 && remaining == 0
}

/// 上游明确返回正总量且剩余为 0。
pub fn imagine_quota_exhausted(total: i64, remaining: i64) -> bool {
    total > 0 && remaining <= 0
}

/// 闸门返回可解析的正剩余次数（含 micro-credit）。
pub fn imagine_quota_known_positive(total: i64, remaining: i64) -> bool {
    total > 0 && remaining > 0
}

/// imagine 窗口是否为近期上游同步。
pub fn imagine_window_upstream_fresh(window: &QuotaWindow, now: DateTime<Utc>) -> bool {
    if window.mode != MODE_IMAGINE {
        return false;
    }
    if window.source != QuotaSource::Upstream {
        return false;
    }
    match window.synced_at {
        Some(synced) => now.signed_duration_since(synced) <= ttl(),
        None => false,
    }
}

/// 闸门返回可信正额度且同步未过期。
pub fn imagine_known_quota_fresh(window: &QuotaWindow, now: DateTime<Utc>) -> bool {
    imagine_window_upstream_fresh(window, now)
        && imagine_quota_known_positive(window.total, window.remaining)
}

/// 近期同步的 0/0 未知闸门。
pub fn imagine_quota_unknown_fresh(window: &QuotaWindow, now: DateTime<Utc>) -> bool {
    imagine_window_upstream_fresh(window, now)
        && imagine_quota_limit_unknown(window.total, window.remaining)
}

/// 模型状态记录近期（TTL 内）真实 Lite 成功。
pub fn imagine_recent_lite_success(state: Option<&ModelState>, now: DateTime<Utc>) -> bool {
    match state {
        Some(s) if s.status == ModelStatus::Available && s.last_success_at.is_some() => {
            now.signed_duration_since(s.last_success_at.unwrap()) <= ttl()
        }
        _ => false,
    }
}

/// 将 imagine 剩余换算为可读生成次数；未知时 `known=false`。
pub fn imagine_remaining_generations(window: &QuotaWindow) -> (i64, bool) {
    if window.mode != MODE_IMAGINE {
        return (0, false);
    }
    if imagine_quota_limit_unknown(window.total, window.remaining) {
        return (0, false);
    }
    if imagine_quota_exhausted(window.total, window.remaining) {
        return (0, true);
    }
    imagine_generations(window.remaining, window.total)
}

/// 仅凭额度窗口与模型状态判定是否可进入 Lite 调度。
///
/// Lite 本身不返回剩余次数；0/0 时依赖近期 Lite 成功或待探测状态。
pub fn imagine_dispatch_quota_admissible(
    window: Option<&QuotaWindow>,
    state: Option<&ModelState>,
    now: DateTime<Utc>,
) -> bool {
    let Some(window) = window else { return false };
    if !imagine_window_upstream_fresh(window, now) {
        return false;
    }
    if imagine_quota_exhausted(window.total, window.remaining) {
        return false;
    }
    if imagine_known_quota_fresh(window, now) {
        return true;
    }
    if !imagine_quota_unknown_fresh(window, now) {
        return false;
    }
    match state {
        None => false,
        Some(s) => match s.status {
            // available 表示历史上 Lite 真实成功；闸门 0/0 不可靠，勿因无近期成功而踢出。
            ModelStatus::Available => true,
            ModelStatus::QuotaAvailable | ModelStatus::Unknown => true,
            _ => false,
        },
    }
}

/// 将上游 imagine 计数换算为可读生成次数。
///
/// Grok 可能返回小整数（如 12/7）或 micro-credit 大数（如 3850000000）。
pub fn imagine_generations(remaining: i64, total: i64) -> (i64, bool) {
    if total <= 0 || remaining < 0 {
        return (0, false);
    }
    if total <= 1000 {
        return (remaining, true);
    }
    let mut unit = total / 10;
    if unit < 1 {
        unit = 1;
    }
    ((remaining + unit - 1) / unit, true)
}

/// imagine 总的生成额度。
pub fn imagine_generations_total(total: i64) -> (i64, bool) {
    if total <= 0 {
        return (0, false);
    }
    if total <= 1000 {
        return (total, true);
    }
    let mut unit = total / 10;
    if unit < 1 {
        unit = 1;
    }
    ((total + unit - 1) / unit, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::QuotaWindow;

    fn window(total: i64, remaining: i64, synced: DateTime<Utc>) -> QuotaWindow {
        QuotaWindow {
            account_id: 1,
            mode: MODE_IMAGINE.to_string(),
            remaining,
            total,
            synced_at: Some(synced),
            source: QuotaSource::Upstream,
            updated_at: synced,
            ..Default::default()
        }
    }

    fn state(status: ModelStatus, last_success: Option<DateTime<Utc>>) -> ModelState {
        ModelState {
            account_id: 1,
            upstream_model: "grok-imagine-image".into(),
            status,
            last_success_at: last_success,
            ..Default::default()
        }
    }

    #[test]
    fn fresh_window_classifies() {
        let now = Utc::now();
        let fresh = window(10, 3, now - chrono::Duration::minutes(5));
        assert!(imagine_window_upstream_fresh(&fresh, now));
        assert!(imagine_known_quota_fresh(&fresh, now));
        assert!(!imagine_quota_unknown_fresh(&fresh, now));

        let zero = window(0, 0, now - chrono::Duration::minutes(5));
        assert!(imagine_window_upstream_fresh(&zero, now));
        assert!(!imagine_known_quota_fresh(&zero, now));
        assert!(imagine_quota_unknown_fresh(&zero, now));
    }

    #[test]
    fn stale_or_non_upstream_is_not_fresh() {
        let now = Utc::now();
        let stale = window(10, 3, now - chrono::Duration::hours(2));
        assert!(!imagine_window_upstream_fresh(&stale, now));

        let mut other = window(10, 3, now - chrono::Duration::minutes(5));
        other.source = QuotaSource::Default;
        assert!(!imagine_window_upstream_fresh(&other, now));
    }

    #[test]
    fn dispatch_admissible_gates() {
        let now = Utc::now();
        let fresh = window(10, 3, now - chrono::Duration::minutes(5));
        // exhausted (known positive total, zero remaining) rejected even if fresh.
        let exhausted = window(10, 0, now - chrono::Duration::minutes(5));
        assert!(!imagine_dispatch_quota_admissible(
            Some(&exhausted),
            Some(&state(ModelStatus::QuotaAvailable, None)),
            now
        ));
        // known positive accepted.
        assert!(imagine_dispatch_quota_admissible(
            Some(&fresh),
            Some(&state(ModelStatus::Available, None)),
            now
        ));
        // zero/zero unknown gate accepted with recent lite success (available).
        let unknown = window(0, 0, now - chrono::Duration::minutes(5));
        assert!(imagine_dispatch_quota_admissible(
            Some(&unknown),
            Some(&state(
                ModelStatus::Available,
                Some(now - chrono::Duration::minutes(10))
            )),
            now
        ));
        // stale window always rejected.
        let stale = window(10, 3, now - chrono::Duration::hours(2));
        assert!(!imagine_dispatch_quota_admissible(Some(&stale), None, now));
    }

    #[test]
    fn generations_conversion() {
        // small counts pass through directly.
        assert_eq!(imagine_generations(7, 12), (7, true));
        assert_eq!(imagine_generations(0, 12), (0, true));
        // micro-credit: macro units of total/10, ceiling division per remaining.
        // 5,000,000 = exactly 5 × 1,000,000-unit -> 5.
        assert_eq!(imagine_generations(5_000_000, 10_000_000), (5, true));
        // 5,000,001 = just into the 6th macro unit -> 6.
        assert_eq!(imagine_generations(5_000_001, 10_000_000), (6, true));
        // negative remaining -> unknown.
        assert_eq!(imagine_generations(-1, 100), (0, false));
        // zero/zero unknown gate has no readable generations.
        assert_eq!(
            imagine_remaining_generations(&window(0, 0, Utc::now())),
            (0, false)
        );
        // exhausted (known positive total, zero remaining) known=true, 0 gens.
        assert_eq!(
            imagine_remaining_generations(&window(10, 0, Utc::now())),
            (0, true)
        );
    }
}
