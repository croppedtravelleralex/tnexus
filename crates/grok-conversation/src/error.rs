//! grok-conversation 错误模型。
//!
//! 纯协议层校验失败应为 HTTP 400（调用方映射），见 docs/39c §1 层级约定：
//! 协议校验错误不当作 upstream 失败。每个变体保留 Go `chat.go`/`contentTextAndImages`
//! 的语义，便于单测断言 message 内容。

use thiserror::Error;

/// 对话协议校验错误。全部映射 HTTP 400（由 grok-gateway 转码）。
#[derive(Debug, Error, PartialEq)]
pub enum ConversationError {
    /// `messages` 列表为空。
    #[error("messages 不能为空")]
    EmptyMessages,

    /// `input`（responses 操作）为空或 null。
    #[error("input 不能为空")]
    EmptyInput,

    /// `input` 非字符串也非消息数组。
    #[error("input 必须是字符串或消息数组")]
    InvalidInput,

    /// `input` 字符串反序列化失败。
    #[error("input 格式无效")]
    InvalidInputString,

    /// `content` 单个字符串反序列化失败。
    #[error("消息 content 字符串无效")]
    InvalidContentString,

    /// `content` 既非字符串也非内容数组。
    #[error("消息 content 必须是字符串或内容数组")]
    InvalidContent,

    /// 消息中没有可发送的文本或图片。
    #[error("消息中没有可发送的文本或图片")]
    NoContent,

    /// 图片数量超过单次上限 `{max}`。
    #[error("单次对话最多支持 {max} 张图片")]
    TooManyImages { max: usize },

    /// `input_image.file_id` 不支持。
    #[error("Grok Web 对话暂不支持 input_image.file_id，请使用 image_url 或 Base64 data URI")]
    UnsupportedFileId,

    /// 图片部分缺少 image_url。
    #[error("图片内容缺少 image_url")]
    MissingImageUrl,

    /// 不支持的 content.type，如 `input_audio` / `file` / `input_file`。
    #[error("Grok Web 对话暂不支持 {type_name} 内容")]
    UnsupportedContentType { type_name: String },

    /// 未知 content.type。
    #[error("Grok Web 对话暂不支持 content.type=\"{type_name}\"")]
    UnknownContentType { type_name: String },

    /// 图片总大小超过 `{max_bytes}` 字节（data URI 层的可计算校验；远端 URL 大小在 pipeline 层校验）。
    #[error("图片总大小超过 {max_bytes} 字节")]
    ImagesTooLarge { max_bytes: u64 },
}

pub type ConversationResult<T> = Result<T, ConversationError>;
