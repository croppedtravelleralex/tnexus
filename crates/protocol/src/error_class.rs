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
