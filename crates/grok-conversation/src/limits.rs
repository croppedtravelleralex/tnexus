//! 对话协议上限常量。
//!
//! 对齐 Go `provider/web/attachments.go`（`maxChatImageAttachments = 8`）与
//! docs/39c §2 OCR 矩阵（G-OCR-4 9 图→400；G-OCR-5 超大图→400；G-OCR-6 file_id→400）。

/// 单次对话最多图片数。8 张 → 200；9 张 → 400（G-OCR-4）。
pub const MAX_CHAT_IMAGE_ATTACHMENTS: usize = 8;

/// 图片总大小上限（data URI 层可计算的字节上限），64 MiB（G-OCR-5）。
///
/// 注意：对 data:`image` URI 可在协议层按 base64 解码长度计算；对远端 HTTPS URL
/// 无法在无 IO 的 conversation 层得知大小，须由 provider-web / image-pipeline 在
/// 取图后复验（遗留边界，见 docs/39c §1 层级）。
pub const MAX_TOTAL_IMAGE_BYTES: u64 = 64 << 20;
