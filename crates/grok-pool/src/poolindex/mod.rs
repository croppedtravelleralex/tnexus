//! `poolindex` — 调度索引原语（G3-P1）。
//!
//! Rust 移植 Go `application/account/poolindex/*.go`（权威对照）：
//! - `dispatch`：`DispatchIndex`（BTree 有序集 + byID 旁表 + 可选 mirror）与
//!   `DispatchQuota`（从 Billing/QuotaRecovery 推导调度额度序）
//! - `heap`：`DueHeap`（按到期时间的最小堆）
//! - `drr` / `web_drr`：加权轮询（verify:normal:delete = 5:3:2，fallback 7:3）
//! - `timing_wheel`：单层环形时间轮
//! - `mirror`：`DispatchMirror` trait + Redis ZSET 镜像（可选，失败不影响内存索引）
//!
//! 并发模型对齐 Go：`std::sync::Mutex` 内省锁；mirror 调用在释放锁后执行。

pub mod dispatch;
pub mod drr;
pub mod heap;
pub mod mirror;
pub mod timing_wheel;
pub mod web_drr;

pub use dispatch::{dispatch_quota, DispatchIndex, DispatchEntry, DispatchMirror};
pub use drr::{DRRScheduler, Lane};
pub use heap::{DueHeap, DueItem};
pub use mirror::{dispatch_score, RedisDispatchMirror};
pub use timing_wheel::TimingWheel;
pub use web_drr::{WebDRRScheduler, WebMaintenanceLane};