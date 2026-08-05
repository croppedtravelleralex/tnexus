//! grok-ops — Grok 运维后台任务（G3-P5 / G3-P6）。
//!
//! 移植自 Go `internal/app/startup.go` 中的后台任务，聚焦三个最小闭环：
//!
//! - [`probe::WebDispatchProbe`] ← Go `runWebDispatchProbe` / `WebDispatchProbeTick`
//! - [`quota::WebQuotaRefresh`] ← Go `runWebQuotaRefresh` / `refreshQuota`
//! - [`pins::PinSyncTask`]    ← Go `SyncImageDispatchPins` / `web_pool_pins.go`
//!
//! 每个后台任务都暴露两个入口：
//! - `run_once()`：手动触发单轮（幂等，便于单测 / 集成）
//! - `spawn_loop()`：包成 `tokio` task 按 interval 循环（G3-A4 需 24h 无 panic）
//!
//! 边界：本 crate **不依赖 grok-gateway**，避免循环依赖；对上微量 IO（探针、上游
//! quota、route pin DB）通过本 crate 内定义的 trait 抽象，测试注入 mock fake。

pub mod error;
pub mod pins;
pub mod probe;
pub mod quota;
pub mod build_probe;
pub mod four_pool;

pub use error::{OpsError, OpsResult};
#[doc(hidden)]
pub use pins::{PinSyncResult, PinSyncTask, RoutePinRepository};
#[doc(hidden)]
pub use probe::{ProbeBackend, WebDispatchProbe};
#[doc(hidden)]
pub use quota::{QuotaRefreshResult, QuotaStore, WebQuotaRefresh};
#[doc(hidden)]
pub use build_probe::{
    BuildProbeLaneAttempts, BuildProbeMode, BuildProbeMonitor, BuildProbeOutcome, BuildProbeResult,
    BuildProbeStatistics, BuildProbeStatus, ProbeFailure,
};
#[doc(hidden)]
pub use four_pool::{BuildFourPool, BuildProbeOps, TickResult};

#[cfg(test)]
mod ops_tests;
