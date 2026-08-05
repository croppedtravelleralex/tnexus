//! 图片附件准备（docs/39d §4.1 `attachments.go` → Rust）。
//!
//! `prepareChatAttachments` 的 G1 版本：对归一化后的每张图（HTTPS URL 或 data URI）
//! 经 bridge 下载字节，推断 MIME，base64 编码为 `FileAttachment`。data URI 本地直解，
//! 远端 HTTPS 走 bridge（`BridgeClient::fetch_bytes`）。
//!
//! G1 用 mock/simplified bridge 即可；完整 upload-file（`POST .../upload-file`
//! 拿 `fileMetadataId`）在浏览器桥内部，本 crate 只需组装附件。

use base64::Engine;

use crate::bridge::BridgeClient;
use crate::error::ProviderError;

/// 单张图片附件（进入 chat payload 的 `fileAttachments`）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FileAttachment {
    /// 原始来源（image_url 或 data URI），保留供审计/追溯。
    pub source_url: String,
    /// 附件文件名（grok 端展示用）。
    pub file_name: String,
    /// MIME 类型（image/png 等）。
    pub mime_type: String,
    /// base64 编码的图像字节。
    pub data_b64: String,
}

impl FileAttachment {
    /// 附件字节数（base64 解码后长度，供总大小复验）。
    pub fn byte_len(&self) -> u64 {
        match base64::engine::general_purpose::STANDARD.decode(&self.data_b64) {
            Ok(bytes) => bytes.len() as u64,
            Err(_) => 0,
        }
    }
}

/// 将归一化后的 image 清单转为附件列表（保持输入顺序，最多 `MAX` 张由上层保证）。
pub async fn prepare_file_attachments<B: BridgeClient + ?Sized>(
    images: &[String],
    bridge: &B,
) -> Result<Vec<FileAttachment>, ProviderError> {
    let mut out = Vec::with_capacity(images.len());
    for (i, url) in images.iter().enumerate() {
        let bytes = bridge.fetch_bytes(url).await?;
        if bytes.is_empty() {
            return Err(ProviderError::Bridge(format!(
                "downloaded empty image for {url}"
            )));
        }
        let (mime, ext) = guess_mime(url);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        out.push(FileAttachment {
            source_url: url.clone(),
            file_name: format!("attachment_{}.{}", i + 1, ext),
            mime_type: mime.to_string(),
            data_b64: encoded,
        });
    }
    Ok(out)
}

/// 根据 data URI 前缀 / URL 扩展名推断 MIME 与扩展名。
fn guess_mime(url: &str) -> (&'static str, &'static str) {
    // data URI：`data:image/<type>;base64,`
    if let Some(rest) = url.strip_prefix("data:image/") {
        for (prefix, (mime, ext)) in MIME_BY_PREFIX {
            if rest.starts_with(prefix) {
                return (*mime, *ext);
            }
        }
    }
    // URL 拓展名。
    let lower = url.to_ascii_lowercase();
    for (prefix, (mime, ext)) in EXT_BY_SUFFIX {
        if lower.ends_with(prefix) {
            return (*mime, *ext);
        }
    }
    ("image/png", "png")
}

const MIME_BY_PREFIX: &[(&str, (&str, &str))] = &[
    ("png", ("image/png", "png")),
    ("jpeg", ("image/jpeg", "jpg")),
    ("jpg", ("image/jpeg", "jpg")),
    ("webp", ("image/webp", "webp")),
    ("gif", ("image/gif", "gif")),
];

const EXT_BY_SUFFIX: &[(&str, (&str, &str))] = &[
    (".png", ("image/png", "png")),
    (".jpg", ("image/jpeg", "jpg")),
    (".jpeg", ("image/jpeg", "jpg")),
    (".webp", ("image/webp", "webp")),
    (".gif", ("image/gif", "gif")),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::MockBridgeClient;
    use std::collections::HashMap;

    const DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    #[tokio::test]
    async fn data_uri_attachment_local_decode() {
        let b = MockBridgeClient::new();
        let files = prepare_file_attachments(&[DATA_URI.to_string()], &b)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].mime_type, "image/png");
        assert!(!files[0].data_b64.is_empty());
        assert!(files[0].byte_len() > 0);
    }

    #[tokio::test]
    async fn remote_uses_bridge_bytes() {
        let mut b = MockBridgeClient::new();
        b.images = HashMap::from([(
            "https://x.com/a.png".to_string(),
            vec![0x89, 0x50, 0x4E, 0x47], // PNG magic
        )]);
        let files = prepare_file_attachments(&["https://x.com/a.png".to_string()], &b)
            .await
            .unwrap();
        assert_eq!(files[0].mime_type, "image/png");
        assert_eq!(files[0].byte_len(), 4);
    }

    #[tokio::test]
    async fn missing_bridge_bytes_errors() {
        let b = MockBridgeClient::new();
        let r = prepare_file_attachments(&["https://x.com/miss.png".to_string()], &b).await;
        assert!(r.is_err());
    }

    #[test]
    fn mime_inference() {
        assert_eq!(guess_mime("https://x.com/a.jpeg"), ("image/jpeg", "jpg"));
        assert_eq!(guess_mime(DATA_URI), ("image/png", "png"));
        assert_eq!(
            guess_mime("data:image/webp;base64,AA=="),
            ("image/webp", "webp")
        );
        assert_eq!(guess_mime("https://x.com/noext"), ("image/png", "png"));
    }
}
