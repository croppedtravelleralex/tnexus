//! 单层环形时间轮（Go `timing_wheel.go` 移植）。
//!
//! 短冷却到期回调：单层环形槽，tick 粒度默认 1 秒，覆盖约 `slot_count * tick` 窗口；
//! 超出放 overflow 最小堆语义列表。`Schedule` 用可注入时钟（测试固定），`Advance` 用外部传入 `now`。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// 时钟抽象：返回当前时间。默认 `SystemTime::now`，测试可注入固定时钟。
pub type Clock = Box<dyn Fn() -> SystemTime + Send + Sync>;

fn real_clock() -> Clock {
    Box::new(SystemTime::now)
}

/// 内部状态。
struct Inner {
    tick: Duration,
    slots: Vec<HashMap<u64, SystemTime>>,
    cursor: usize,
    started_at: SystemTime,
    /// 超窗（steps >= slots.len()）放这里，键值到期时间。
    overflow: HashMap<u64, SystemTime>,
    /// id -> slot index，-1 表示 overflow。
    pending: HashMap<u64, i64>,
}

/// 单层环形时间轮（Go `TimingWheel`）。
pub struct TimingWheel {
    inner: Mutex<Inner>,
    now: Clock,
}

impl TimingWheel {
    pub fn new(tick: Duration, slot_count: usize) -> Self {
        Self::with_clock(tick, slot_count, real_clock())
    }

    /// 用可注入时钟构造（测试确定性）。
    #[doc(hidden)]
    pub fn with_clock(tick: Duration, slot_count: usize, now: Clock) -> Self {
        let tick = if tick <= Duration::ZERO {
            Duration::from_secs(1)
        } else {
            tick
        };
        let slot_count = if slot_count < 8 { 64 } else { slot_count };
        Self {
            inner: Mutex::new(Inner {
                tick,
                slots: (0..slot_count).map(|_| HashMap::new()).collect(),
                cursor: 0,
                started_at: (now)(),
                overflow: HashMap::new(),
                pending: HashMap::new(),
            }),
            now,
        }
    }

    pub fn tick(&self) -> Duration {
        self.inner.lock().unwrap().tick
    }

    pub fn slot_count(&self) -> usize {
        self.inner.lock().unwrap().slots.len()
    }

    /// 在 `due_at` 到期时弹出 `id`（覆盖同 id 旧闹钟）。
    pub fn schedule(&self, id: u64, due_at: SystemTime) {
        let mut g = self.inner.lock().unwrap();
        self.remove_locked(&mut g, id);
        let now = (self.now)();
        let due = if due_at <= now { now + g.tick } else { due_at };
        let delay = due
            .duration_since(now)
            .unwrap_or_else(|_| g.tick);
        let steps = (delay.as_nanos() / g.tick.as_nanos()) as usize;
        let steps = if steps < 1 { 1 } else { steps };
        if steps >= g.slots.len() {
            g.overflow.insert(id, due);
            g.pending.insert(id, -1);
            return;
        }
        let slot = (g.cursor + steps) % g.slots.len();
        g.slots[slot].insert(id, due);
        g.pending.insert(id, slot as i64);
    }

    /// 取消 `id` 的闹钟。
    pub fn cancel(&self, id: u64) {
        let mut g = self.inner.lock().unwrap();
        self.remove_locked(&mut g, id);
    }

    fn remove_locked(&self, g: &mut Inner, id: u64) {
        match g.pending.remove(&id) {
            Some(slot) if slot < 0 => {
                g.overflow.remove(&id);
            }
            Some(slot) => {
                g.slots[slot as usize].remove(&id);
            }
            None => {}
        }
    }

    /// 推进到 `now`，返回到期账号 id。
    pub fn advance(&self, now: SystemTime) -> Vec<u64> {
        let mut g = self.inner.lock().unwrap();
        let elapsed = now
            .duration_since(g.started_at)
            .ok()
            .unwrap_or(Duration::ZERO);
        let elapsed_ticks = (elapsed.as_nanos() / g.tick.as_nanos()) as usize;
        let target_cursor = elapsed_ticks % g.slots.len();
        let mut due = Vec::new();
        let mut steps = 0;
        while g.cursor != target_cursor && steps < g.slots.len() {
            g.cursor = (g.cursor + 1) % g.slots.len();
            steps += 1;
            let ci = g.cursor;
            let tick_map = std::mem::take(&mut g.slots[ci]);
            for (id, at) in tick_map {
                if at <= now {
                    g.pending.remove(&id);
                    due.push(id);
                } else {
                    g.overflow.insert(id, at);
                    g.pending.insert(id, -1);
                }
            }
        }
        // overflow 到期者弹出。
        let expired: Vec<u64> = g
            .overflow
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            g.overflow.remove(&id);
            g.pending.remove(&id);
            due.push(id);
        }
        due
    }
}