//! grok-domain — Grok 子系统领域类型（骨架）。
//!
//! 模块映射见 docs/39d-grok-go-rust-map.md §7：
//! - `account`/`provider`/`quota` ← Go `domain/account`
//! - `egress::Scope` ← Go `domain/egress`
//! - `audit` ← Go `domain/audit`
//! - `pipeline::Stage` ← Go `domain/imagepipeline`
//! - `chrome_ticket` ← Go `domain/chrometicket`
//! - `model_route` ← Go `domain/model`
//!
//! 后续 Phase 在对应模块内补齐字段与行为；本骨架仅保证编译与类型占位。

pub mod account;
pub mod audit;
pub mod chrome_ticket;
pub mod egress;
pub mod imagine_quota;
pub mod model_route;
pub mod pipeline;
pub mod provider;

pub use account::*;
pub use audit::*;
pub use chrome_ticket::*;
pub use egress::*;
pub use imagine_quota::*;
pub use model_route::*;
pub use pipeline::*;
pub use provider::*;
