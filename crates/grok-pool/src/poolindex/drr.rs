//! DRR 加权轮询（Go `drr.go` 移植）。

/// 维护探针车道（Go `Lane`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lane {
    #[default]
    Verification = 0,
    Normal = 1,
    Delete = 2,
}

impl Lane {
    pub fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Lane::Verification),
            1 => Some(Lane::Normal),
            2 => Some(Lane::Delete),
            _ => None,
        }
    }
}

/// 加权轮询（空车道跳过，deficit 结转语义）：
/// 三池有任务时 验证:普通:删除 = 5:3:2；验证空时 普通:删除 = 7:3（Go `DRRScheduler`）。
#[derive(Debug, Default)]
pub struct DRRScheduler {
    pos: usize,
    pattern: [Lane; 10],
    fallback: [Lane; 10],
}

impl DRRScheduler {
    pub fn new() -> Self {
        Self {
            pos: 0,
            pattern: [
                Lane::Verification,
                Lane::Verification,
                Lane::Verification,
                Lane::Verification,
                Lane::Verification,
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Delete,
                Lane::Delete,
            ],
            fallback: [
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Normal,
                Lane::Delete,
                Lane::Delete,
                Lane::Delete,
            ],
        }
    }

    /// 选出下一车道。`has_work[i]` 表示该车道当前有到期任务。
    pub fn next(&mut self, has_work: [bool; 3]) -> Option<Lane> {
        let pattern = if has_work[Lane::Verification as usize] {
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
                return Lane::from_index(i);
            }
        }
        None
    }
}