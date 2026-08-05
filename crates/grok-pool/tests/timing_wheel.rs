//! G3-P1 `TimingWheel` 集成测试。
//!
//! Go 侧无对应单测（`poolindex_test.go` 不覆盖 timing wheel），此处为 Rust 移植补充
//! 确定性行为测试：槽内到期、溢出到期、cancel、同 id 覆盖、`advance` 幂等。

use std::time::{Duration, SystemTime};

use grok_pool::poolindex::TimingWheel;

/// 注入固定起点时钟（`now` 恒等于 epoch + offset）。
fn fixed_clock(offset: Duration) -> impl Fn() -> SystemTime {
    let base = SystemTime::UNIX_EPOCH + offset;
    move || base
}

#[test]
fn schedule_and_advance_emits_due_id() {
    let now0 = Duration::from_secs(1000);
    let wheel = TimingWheel::with_clock(Duration::from_secs(1), 64, Box::new(fixed_clock(now0)));
    let t0 = SystemTime::UNIX_EPOCH + now0;

    wheel.schedule(1, t0 + Duration::from_secs(3));
    wheel.schedule(2, t0 + Duration::from_secs(10));

    // 未到 3s：不应弹出。
    assert_eq!(wheel.advance(t0 + Duration::from_secs(2)), Vec::<u64>::new());

    // 到 3s：id=1 弹出。
    let due = wheel.advance(t0 + Duration::from_secs(3));
    assert_eq!(due, vec![1]);

    // 继续到 10s：id=2 弹出。
    let due = wheel.advance(t0 + Duration::from_secs(10));
    assert_eq!(due, vec![2]);
}

#[test]
fn schedule_then_cancel_suppresses_id() {
    let now0 = Duration::from_secs(2000);
    let wheel = TimingWheel::with_clock(Duration::from_secs(1), 64, Box::new(fixed_clock(now0)));
    let t0 = SystemTime::UNIX_EPOCH + now0;

    wheel.schedule(7, t0 + Duration::from_secs(2));
    wheel.cancel(7);
    assert_eq!(wheel.advance(t0 + Duration::from_secs(5)), Vec::<u64>::new());
}

#[test]
fn overslot_goes_to_overflow_and_emits_on_due() {
    let now0 = Duration::from_secs(3000);
    // 64 槽 × 1s：steps>=64 进 overflow。
    let wheel = TimingWheel::with_clock(Duration::from_secs(1), 64, Box::new(fixed_clock(now0)));
    let t0 = SystemTime::UNIX_EPOCH + now0;

    wheel.schedule(9, t0 + Duration::from_secs(70));
    // 到 70s 前不应弹。
    assert_eq!(wheel.advance(t0 + Duration::from_secs(69)), Vec::<u64>::new());
    let due = wheel.advance(t0 + Duration::from_secs(70));
    assert_eq!(due, vec![9]);
}

#[test]
fn reschedule_same_id_overrides_old_alarm() {
    let now0 = Duration::from_secs(4000);
    let wheel = TimingWheel::with_clock(Duration::from_secs(1), 64, Box::new(fixed_clock(now0)));
    let t0 = SystemTime::UNIX_EPOCH + now0;

    // 原定 5s；改到 20s。
    wheel.schedule(5, t0 + Duration::from_secs(5));
    wheel.schedule(5, t0 + Duration::from_secs(20));

    // 5s 时不应弹（旧闹钟被覆盖）。
    assert_eq!(wheel.advance(t0 + Duration::from_secs(5)), Vec::<u64>::new());
    assert_eq!(wheel.advance(t0 + Duration::from_secs(20)), vec![5]);
}
