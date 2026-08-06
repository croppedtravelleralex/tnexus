//! 鉴权：`Authorization: Bearer <key>` 常量时间比较（对齐 Python `hmac.compare_digest`）。
//!
//! `GROK_BRIDGE_KEY` 未配置 → 禁止所有非 `/health` 请求（401），与安全红线一致
//! （凭据缺失时拒绝服务而不是裸奔）。

/// 读取配置的 bridge key（`GROK_BRIDGE_KEY`）。
pub fn configured_key() -> String {
    std::env::var("GROK_BRIDGE_KEY").unwrap_or_default()
}

/// 常量时间比较（长度差异也走同路径，不提前返回）。
pub fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        // 长度不同仍走一轮异或，避免泄露长度比较分支时序。
        let n = a.len().min(b.len());
        let mut acc: u8 = (a.len() ^ b.len()) as u8;
        for i in 0..n {
            acc |= a[i] ^ b[i];
        }
        return acc == 0;
    }
    let mut acc: u8 = 0;
    for i in 0..a.len() {
        acc |= a[i] ^ b[i];
    }
    acc == 0
}

/// 校验 `Authorization` 头是否为 `Bearer <key>`。
pub fn authorized(header: Option<&str>, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    match header {
        Some(h) => {
            let expected = format!("Bearer {key}");
            ct_eq(h, &expected)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_missing_denies_all() {
        assert!(!authorized(Some("Bearer x"), ""));
    }

    #[test]
    fn correct_bearer_passes() {
        assert!(authorized(Some("Bearer secret123"), "secret123"));
        // 大小写敏感。
        assert!(!authorized(Some("bearer secret123"), "secret123"));
        assert!(!authorized(Some("Bearer secret12"), "secret123"));
        assert!(!authorized(None, "secret123"));
    }

    #[test]
    fn ct_eq_length_mismatch_is_false() {
        assert!(!ct_eq("ab", "abc"));
        assert!(ct_eq("abc", "abc"));
    }
}
