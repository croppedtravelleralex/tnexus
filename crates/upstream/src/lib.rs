//! Rust upstream data plane — ported from `gptimage-panda` (primary) with
//! `../gptimage` cross-check for intentional drift.
//!
//! Phase 1 scope: TLS client, PoW, Turnstile VM, SSE parsing, chat-requirements.

pub mod account;
pub mod conversation;
pub mod estuary;
pub mod image_metrics;
pub mod openai_stream;
pub mod poll;
pub mod pow;
pub mod requirements;
pub mod runtime;
pub mod sentinel;
pub mod sse;
pub mod tls;
pub mod turnstile;
pub mod upload;

pub use account::PinAccount;
pub use image_metrics::ImageRunMetrics;
pub use openai_stream::{chat_image_b64_sse_stream, OpenAiSseStream};
pub use poll::{
    extract_image_ids_from_conversation, get_conversation, poll_image_conversation,
    poll_image_ready_from_tasks, query_tasks, ImagePollConfig, ImagePollOutcome,
};
pub use requirements::{ChatRequirements, RequirementsClient};
pub use runtime::UpstreamRuntime;
pub use sse::{
    consume_sse_until, ConsumedSse, ConversationState, ImageSseReady, SseConsumeMode, SseEvent,
    SseParser, TextSseReady,
};
