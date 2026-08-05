//! grok-conversation — OpenAI → grok 内部对话表示的协议层（纯逻辑，无 IO）。
//!
//! 端口自 Go `provider/web/chat.go` 的对话归一化与 OCR 路径（docs/39d §4.1/4.2，
//! docs/39c §2 G-OCR-*）。本 crate 只做协议翻译与校验，不做任何 IO：
//!   - `normalize_chat_input`：OpenAI `messages` → `NormalizedChatInput{prompt, images}`
//!   - `content_text_and_images`：多模态 content → 文本 + 图片清单
//!   - `limits`：单次图片上限（8 张 / 64 MiB）
//!
//! 边界：`prepareChatAttachments`（upload-file）、data URI 解码取字节、远端 HTTPS 图
//! 的 SSRF 与大小校验属 provider-web / image-pipeline（有 IO），不在本 crate。

mod error;
mod limits;
mod normalize;

pub use error::{ConversationError, ConversationResult};
pub use limits::{MAX_CHAT_IMAGE_ATTACHMENTS, MAX_TOTAL_IMAGE_BYTES};
pub use normalize::{normalize_chat_input, ChatMessage, NormalizedChatInput};
