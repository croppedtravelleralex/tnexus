//! grok-bridge — browser-bridge 的 Rust 实现（对齐 Go 协议）。
//!
//! 替换外部 Python bridge（`browser-bridge/app.py`，bottle/waitress + FlareSolverr
//! 镜像）：自写 CDP 客户端（tokio-tungstenite 直连 Chrome/Edge remote-debugging），
//! 提供与 Python 版一致的 4 个端点：
//!
//! - `GET  /health` → `{status, sessions}`
//! - `POST /v1/sign` → 生成 grok.com `/rest/*` 的 `x-statsig-id`
//! - `POST /v1/fetch` → 浏览器内 fetch（自动附加 x-statsig-id）
//! - `POST /v1/websocket` → 浏览器内 WebSocket 收集生图帧
//!
//! 鉴权：`GROK_BRIDGE_KEY`（缺省未配置时非 `/health` 请求一律 401）。
//! 会话池：TTL 1800s、每会话并发 1（信号量）、按键隔离（sessionKey 派生）。
//!
//! 边界（如实标注）：Cloudflare 无 cookie 新号引导（Python 版 `_evil_logic`）
//! 无法纯 Rust 复刻——生产走 `light_bootstrap=true`（复用已过 CF 的 cookie 直接
//! 导航）路径，与 Python 版生产主路径一致。

pub mod auth;
pub mod cdp;
pub mod chrome;
pub mod error;
pub mod handlers;
pub mod js;
pub mod session;

pub use error::BridgeError;
pub use handlers::BridgeState;
pub use session::{CdpFactory, SessionPool, SessionPoolConfig};
