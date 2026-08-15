//! Image dispatch gates — full gptimage `image_schedule_core` + task service subset.

mod binding_inflight;
mod cooldown;
mod deadlock_guard;
mod dispatch_gate;
mod dispatch_interval;
mod humanlike;
mod pipeline_watchdog;
mod pre_ticket;
mod proxy_cf;
mod ready_buffer;
mod return_window;
mod runtime_config;
mod sediment;
mod slot_ledger;
mod trace;
mod workload;

pub use binding_inflight::{BindingInflightGuard, BindingInflightLedger};
pub use cooldown::CooldownRegistry;
pub use deadlock_guard::DeadlockGuard;
pub use dispatch_gate::should_wait;
pub use dispatch_interval::{DispatchIntervalGate, DispatchMarkGuard};
pub use humanlike::{
    default_epsilon, pick_account_index, score_account, AccountScoreInput, hour_weight,
};
pub use pipeline_watchdog::PipelineWatchdog;
pub use pre_ticket::PreTicketPool;
pub use proxy_cf::ProxyCfRegistry;
pub use ready_buffer::{ReadyBuffer, ReadyBufferGuard};
pub use return_window::{ReturnWindow, ReturnWindowPermit};
pub use runtime_config::{ImageRuntimeConfig, ImageRuntimeSnapshot};
pub use sediment::SedimentParser;
pub use slot_ledger::{AccountSlotGuard, SharedSlotLedger, SlotLedger, SsSlotGuard};
pub use trace::{ImageScheduleTrace, TraceEventKind};
pub use workload::{poisson_delay_ms, WorkloadPolicy, WorkloadRoute};

/// Apply hot-reloaded snapshot to live gate objects.
pub fn apply_runtime_snapshot(
    snap: &ImageRuntimeSnapshot,
    binding: &BindingInflightLedger,
    dispatch: &DispatchIntervalGate,
) {
    if let Some(v) = snap.image_binding_inflight_max {
        binding.set_max(v);
    }
    if let Some(v) = snap.image_dispatch_interval_ms {
        dispatch.set_interval_ms(v);
    }
}
