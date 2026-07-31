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
    fn self_fault_wins_over_upstream_message() {
        assert_eq!(
            classify_fault(Some("self"), Some("cf_edge_block")),
            ErrorClass::Self_,
            "self must never be masked as upstream — it gates promotion"
        );
    }
}
