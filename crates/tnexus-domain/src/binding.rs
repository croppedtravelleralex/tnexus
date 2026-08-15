//! Proxy / egress binding keys — aligned with gptimage `binding_key_for_account`.

/// Stable binding key for scheduling gates and usage heatmaps.
pub fn binding_key_for_account_fields(
    proxy_binding_hash: Option<&str>,
    proxy: Option<&str>,
    egress_ip: Option<&str>,
) -> String {
    if let Some(hash) = proxy_binding_hash.map(str::trim).filter(|s| !s.is_empty()) {
        return hash.to_string();
    }
    if let Some(ip) = egress_ip.map(str::trim).filter(|s| !s.is_empty()) {
        return format!("egress:{ip}");
    }
    let raw = proxy.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("");
    if raw.is_empty() {
        return "default".to_string();
    }
    let stripped = raw
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("socks5://");
    let host_part = stripped.split('/').next().unwrap_or(stripped);
    let host_part = host_part.split('@').last().unwrap_or(host_part);
    if host_part.is_empty() {
        "default".to_string()
    } else {
        format!("proxy:{host_part}")
    }
}

pub fn binding_key_for_proxy(proxy: Option<&str>, egress_ip: Option<&str>) -> String {
    binding_key_for_account_fields(None, proxy, egress_ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_binding_hash() {
        assert_eq!(
            binding_key_for_account_fields(Some("hash123"), Some("http://a"), Some("1.2.3.4")),
            "hash123"
        );
    }

    #[test]
    fn proxy_host_strips_scheme_and_auth() {
        assert_eq!(
            binding_key_for_proxy(Some("socks5://user:pass@70.1.2.3:30000"), None),
            "proxy:70.1.2.3:30000"
        );
    }
}
