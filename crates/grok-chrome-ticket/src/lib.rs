//! Chrome 长效票池（G4-P3，对齐 Go `application/chrometicket/pool.go`）。
//!
//! 管理本机 Chrome 预捕获的长效 `statsig_meta` 资产：入池（push）、按账号取票并
//! 标记 consumed（pop）、过期清扫（sweep）与状态汇总（stats）。
//!
//! 持久化抽象为 [`ChromeTicketRepository`] trait，测试注入内存 fake
//! （Go 侧用 SQLite repo + 同样的用例）。

pub mod domain;
pub mod pool;

pub use domain::{AccountCount, PushInput, Stats, Ticket, TicketSummary};
pub use pool::{
    normalize_push_input, normalize_push_input_from_fields, ChromeTicketRepository, Pool,
    TicketError, MemoryChromeTicketRepository,
};
