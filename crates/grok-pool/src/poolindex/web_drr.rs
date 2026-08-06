//! Web DRR 加权轮询（Go `web_drr.go` 移植）。

/// 维护探针子车道（Go `WebMaintenanceLane`），recovery 内 priority。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WebMaintenanceLane {
    #[default]
    RecoveryVerify = 0,
    RecoveryCooldown = 1,
    Dead = 2,
}

impl WebMaintenanceLane {
    pub fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(WebMaintenanceLane::RecoveryVerify),
            1 => Some(WebMaintenanceLane::RecoveryCooldown),
            2 => Some(WebMaintenanceLane::Dead),
            _ => None,
        }
    }
}

/// 加权轮询：verify 非空时 verify:cooldown:dead = 5:3:2；verify 空时 recovery:dead = 7:3
/// （Go `WebDRRScheduler`）。
#[derive(Debug, Default)]
pub struct WebDRRScheduler {
    pos: usize,
    pattern: [WebMaintenanceLane; 10],
    fallback: [WebMaintenanceLane; 10],
}

impl WebDRRScheduler {
    pub fn new() -> Self {
        Self {
            pos: 0,
            pattern: [
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryCooldown,
                WebMaintenanceLane::RecoveryCooldown,
                WebMaintenanceLane::RecoveryCooldown,
                WebMaintenanceLane::Dead,
                WebMaintenanceLane::Dead,
            ],
            fallback: [
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::RecoveryVerify,
                WebMaintenanceLane::Dead,
                WebMaintenanceLane::Dead,
                WebMaintenanceLane::Dead,
            ],
        }
    }

    /// 选出下一车道。`has_work[i]` 表示该车道当前有到期任务。
    pub fn next(&mut self, has_work: [bool; 3]) -> Option<WebMaintenanceLane> {
        let pattern = if has_work[WebMaintenanceLane::RecoveryVerify as usize] {
            &self.pattern
        } else {
            &self.fallback
        };
        for _ in 0..pattern.len() {
            let lane = pattern[self.pos % pattern.len()];
            self.pos += 1;
            if has_work[lane as usize] {
                return Some(lane);
            }
        }
        for (i, &w) in has_work.iter().enumerate() {
            if w {
                return WebMaintenanceLane::from_index(i);
            }
        }
        None
    }
}
