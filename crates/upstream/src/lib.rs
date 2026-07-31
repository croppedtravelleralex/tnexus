//! Rust upstream data plane — ported from `gptimage-panda` (primary) with
//! `../gptimage` cross-check for intentional drift.
//!
//! Phase 1 scope: TLS client, PoW, Turnstile VM, SSE parsing, chat-requirements.

pub mod account;
pub mod conversation;
pub mod estuary;
pub mod openai_stream;
pub mod poll;
pub mod pow;
pub mod requirements;
pub mod runtime;
pub mod sentinel;
pub mod sse;
pub mod tls;
pub mod turnstile;

pub use account::PinAccount;
pub use requirements::{ChatRequirements, RequirementsClient};
pub use openai_stream::OpenAiSseStream;
pub use poll::{poll_image_ready_from_tasks, query_tasks};
pub use runtime::UpstreamRuntime;
pub use sse::{
    consume_sse_until, ConsumedSse, ConversationState, ImageSseReady, SseConsumeMode, SseEvent,
    SseParser, TextSseReady,
};
