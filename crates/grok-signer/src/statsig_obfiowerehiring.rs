//! Python `generate_statsig` 的 Rust 移植（对齐 scripts/grok_pure_http_client.py）。
//!
//! 输入：meta48（48 字节）+ fingerprint + method/path → `x-statsig-id`。

use base64::Engine;
use sha2::{Digest, Sha256};

/// grok 前端 epoch（秒）。
pub const EPOCH: i64 = 1_682_924_400;

/// 生成 `x-statsig-id`（与 Python `generate_statsig` 对齐）。
pub fn generate_statsig(
    method: &str,
    path: &str,
    meta48: &[u8],
    fingerprint: &str,
    trailer: &[u8],
) -> Result<String, String> {
    if meta48.len() != 48 {
        return Err(format!("meta48 len={}", meta48.len()));
    }
    let n = (chrono::Utc::now().timestamp() - EPOCH) as u32;
    generate_statsig_with_n(method, path, meta48, fingerprint, n, None, trailer)
}

/// 可注入时间桶 `n`（测试/重放用）。
pub fn generate_statsig_with_n(
    method: &str,
    path: &str,
    meta48: &[u8],
    fingerprint: &str,
    n: u32,
    key: Option<u8>,
    trailer: &[u8],
) -> Result<String, String> {
    if meta48.len() != 48 {
        return Err(format!("meta48 len={}", meta48.len()));
    }
    let dig = format!("{method}!{path}!{n}obfiowerehiring{fingerprint}");
    let digest = Sha256::digest(dig.as_bytes());
    let sha16 = &digest[..16];
    let xor_key = key.unwrap_or(digest[0]);
    let mut block = Vec::with_capacity(69);
    block.extend_from_slice(meta48);
    block.extend_from_slice(&n.to_le_bytes());
    block.extend_from_slice(sha16);
    block.extend_from_slice(if trailer.is_empty() { &[0x03] } else { trailer });
    let enc: Vec<u8> = std::iter::once(xor_key)
        .chain(block.iter().map(|b| b ^ xor_key))
        .collect();
    Ok(base64::engine::general_purpose::STANDARD
        .encode(enc)
        .trim_end_matches('=')
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn deterministic_with_fixed_n() {
        let meta = [0u8; 48];
        let fp = "abc";
        let a = generate_statsig_with_n("POST", "/x", &meta, fp, 42, Some(7), b"\x03").unwrap();
        let b = generate_statsig_with_n("POST", "/x", &meta, fp, 42, Some(7), b"\x03").unwrap();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn roundtrip_block_layout() {
        let meta48 = (0u8..48).collect::<Vec<_>>();
        let fp = "11669e100f5c28f5";
        let n = 103_252_946u32;
        let sig = generate_statsig_with_n("GET", "/rest/app-chat/conversations", &meta48, fp, n, None, b"\x03")
            .unwrap();
        let raw = base64::engine::general_purpose::STANDARD.decode(sig + "==").unwrap();
        assert_eq!(raw[0] ^ raw[1], meta48[0]);
    }
}
