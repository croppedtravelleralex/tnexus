//! Map upstream fault strings to PROTOCOL_CONTRACT error classes.

/// Contract error class per docs/00-contract.md §1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Upstream,
    Client,
    Self_,
    Gate,
}

impl ErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::Upstream => "upstream",
            ErrorClass::Client => "client",
            ErrorClass::Self_ => "self",
            ErrorClass::Gate => "gate",
        }
    }
}

/// Classify a bridge/helper fault string into contract taxonomy.
///
/// An explicit `fault` always wins. When it is absent the message is probed,
/// and only then does it fall back to `upstream` — defaulting to `upstream`
/// up front made the `client` arm unreachable and let genuine client errors
/// count as upstream failures, which biased the `self=0` gate.
pub fn classify_fault(fault: Option<&str>, message: Option<&str>) -> ErrorClass {
    let msg = message.unwrap_or("").to_ascii_lowercase();

    if let Some(f) = fault.map(|f| f.to_ascii_lowercase()) {
        match f.as_str() {
            "self" => return ErrorClass::Self_,
            "quota" | "gate" => return ErrorClass::Gate,
            "client" => return ErrorClass::Client,
            "upstream" => return ErrorClass::Upstream,
            _ => {}
        }
    }

    if msg.contains("sticky miss") || msg.contains("email lock") {
        return ErrorClass::Self_;
    }
    if msg.contains("admission") || msg.contains("duplicate-prompt") || msg.contains("quota") {
        return ErrorClass::Gate;
    }
    // Upstream image quota exhaustion is a rate limit, not a channel outage; as a 502
    // it counts against channel health and hides the fact that a retry would work.
    if msg.contains("image_instant_limit") || msg.contains("image creation limit") {
        return ErrorClass::Gate;
    }
    // Oversized payloads are the caller's input, not an upstream fault. Reporting
    // them as `upstream` surfaces a 502 that NewAPI counts against channel health.
    if msg.contains("message_length_exceeds_limit")
        || msg.contains("payload too large")
        || msg.contains("http 413")
    {
        return ErrorClass::Client;
    }
    // Moderation refusals are about the caller's prompt. As `upstream` they became
    // 502s that dragged down channel success rate and could get the channel demoted.
    if msg.contains("content_policy_violation")
        || msg.contains("防护限制")
        || msg.contains("missing_reference_image")
    {
        return ErrorClass::Client;
    }
    if msg.contains("invalid_request")
        || msg.contains("must include")
        || msg.contains("unsupported")
    {
        return ErrorClass::Client;
    }
    ErrorClass::Upstream
}

/// OpenAI-compatible `error.type` for SDK retry semantics.
pub fn openai_error_type_for_class(class: ErrorClass, code: &str) -> &'static str {
    let code_lower = code.to_ascii_lowercase();
    if code_lower.contains("auth") || code_lower.contains("session") {
        return "authentication_error";
    }
    match class {
        ErrorClass::Client => "invalid_request_error",
        ErrorClass::Gate => "rate_limit_error",
        ErrorClass::Upstream | ErrorClass::Self_ => "server_error",
    }
}

/// Default wait hint (seconds) for admission / concurrency 429s.
pub fn default_rate_limit_wait_secs(class: ErrorClass) -> Option<u32> {
    if class == ErrorClass::Gate {
        Some(30)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_self_fault() {
        assert_eq!(
            classify_fault(Some("self"), Some("sticky miss")),
            ErrorClass::Self_
        );
    }

    #[test]
    fn maps_quota_gate() {
        assert_eq!(classify_fault(Some("quota"), None), ErrorClass::Gate);
    }

    #[test]
    fn maps_cf_upstream() {
        assert_eq!(
            classify_fault(None, Some("cf_edge_block: 403")),
            ErrorClass::Upstream
        );
    }

    #[test]
    fn client_fault_is_reachable() {
        assert_eq!(classify_fault(Some("client"), None), ErrorClass::Client);
        assert_eq!(
            classify_fault(Some("client"), Some("cloudflare said no")),
            ErrorClass::Client,
            "explicit fault must win over message heuristics"
        );
    }

    #[test]
    fn client_inferred_from_message_without_fault() {
        assert_eq!(
            classify_fault(None, Some("messages must include a user text")),
            ErrorClass::Client
        );
    }

    #[test]
    fn unknown_fault_falls_through_to_message() {
        assert_eq!(
            classify_fault(Some("weird"), Some("sticky miss")),
            ErrorClass::Self_
        );
    }

    #[test]
    fn empty_input_defaults_to_upstream() {
        assert_eq!(classify_fault(None, None), ErrorClass::Upstream);
    }

    #[test]
    fn image_quota_exhaustion_is_gate() {
        assert_eq!(
            classify_fault(
                None,
                Some(
                    "upstream image generation failed (image_instant_limit): You've hit the limit"
                )
            ),
            ErrorClass::Gate
        );
    }

    #[test]
    fn oversized_payload_is_client() {
        for msg in [
            "conversation HTTP 413 Payload Too Large: {\"detail\":{\"code\":\"message_length_exceeds_limit\"}}",
            "upstream returned payload too large",
        ] {
            assert_eq!(
                classify_fault(None, Some(msg)),
                ErrorClass::Client,
                "{msg}"
            );
        }
    }

    #[test]
    fn moderation_refusal_is_client() {
        // 线上真实文案：一种带 code 前缀，一种只有中文正文。
        for msg in [
            "upstream image generation failed (content_policy_violation): 抱歉，我不能帮助生成包含强迫性色情行为的图像。",
            "upstream image generation failed: 非常抱歉，生成的图片可能违反了关于暴力内容的防护限制。",
        ] {
            assert_eq!(classify_fault(None, Some(msg)), ErrorClass::Client, "{msg}");
        }
    }

    #[test]
    fn moderation_refusal_does_not_shadow_quota_gate() {
        assert_eq!(
            classify_fault(
                None,
                Some("upstream image generation failed (image_instant_limit): limit reached")
            ),
            ErrorClass::Gate,
            "额度耗尽仍应记 Gate，不能被内容审查规则抢走"
        );
    }

    #[test]
    fn openai_type_mapping() {
        assert_eq!(
            openai_error_type_for_class(ErrorClass::Gate, "image_service_busy"),
            "rate_limit_error"
        );
        assert_eq!(
            openai_error_type_for_class(ErrorClass::Client, "invalid_request"),
            "invalid_request_error"
        );
        assert_eq!(
            openai_error_type_for_class(ErrorClass::Upstream, "upstream_unreachable"),
            "server_error"
        );
        assert_eq!(
            openai_error_type_for_class(ErrorClass::Self_, "semaphore_closed"),
            "server_error"
        );
        assert_eq!(
            openai_error_type_for_class(ErrorClass::Gate, "invalid_session"),
            "authentication_error"
        );
    }
}
