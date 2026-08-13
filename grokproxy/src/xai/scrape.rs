//! Parsing the consent page.
//!
//! The device-approval page is a Next.js app, so approving it over HTTP means
//! reading two things out of the HTML that a browser would have supplied from
//! its own React state:
//!
//! * the server-action id, which is the only way to invoke the approval without
//!   running the page's JavaScript
//! * the account's principal id, which the rendered form ships empty and React
//!   fills client-side. Approving without it returns success and then the token
//!   poll fails with `invalid_grant: Access denied` — a failure that looks like
//!   a bad credential rather than a missing field.
//!
//! Everything here is pure so it can be tested against captured pages.

use std::collections::BTreeMap;

/// A POST form found in the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlForm {
    pub method: String,
    pub action: String,
    pub fields: BTreeMap<String, String>,
}

/// What kind of page the flow has landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    /// Approval is still pending.
    Consent,
    /// Already approved; the token poll will succeed.
    Done,
    /// The SSO cookie was not accepted.
    SignIn,
    /// The user_code entry page.
    Device,
    /// Cloudflare, not the application.
    Blocked,
    Unknown,
}

pub fn classify_page(url: &str, html: &str) -> PageKind {
    let url = url.to_lowercase();
    let text = html.to_lowercase();
    if is_blocked(&text) {
        return PageKind::Blocked;
    }
    if url.contains("/oauth2/device/done") {
        return PageKind::Done;
    }
    if url.contains("/oauth2/device/consent") {
        return PageKind::Consent;
    }
    if url.contains("/sign-in") || text.contains("使用邮箱登录") {
        return PageKind::SignIn;
    }
    if url.contains("/oauth2/device") || text.contains("name=\"user_code\"") {
        if text.contains("authorize grok build") || text.contains("授权 grok build") {
            return PageKind::Consent;
        }
        return PageKind::Device;
    }
    if text.contains("device authorized") || text.contains("设备已授权") {
        return PageKind::Done;
    }
    if text.contains("sign in") {
        return PageKind::SignIn;
    }
    PageKind::Unknown
}

fn is_blocked(lowered: &str) -> bool {
    lowered.contains("attention required")
        || lowered.contains("you have been blocked")
        || lowered.contains("blocked due to abusive")
        || lowered.contains("error code 520")
        || (lowered.contains("cf-error-details") && lowered.contains("cloudflare"))
}

/// First POST form in the page, with its inputs.
pub fn find_post_form(html: &str, base_url: &str) -> Option<HtmlForm> {
    let mut rest = html;
    while let Some(open) = find_ci(rest, "<form") {
        let after = &rest[open..];
        let Some(gt) = after.find('>') else { break };
        let attrs = &after[..gt];
        let body_start = open + gt + 1;
        let Some(close) = find_ci(&rest[body_start..], "</form>") else {
            break;
        };
        let body = &rest[body_start..body_start + close];

        let method = attr_value(attrs, "method").unwrap_or_else(|| "get".into());
        let action = attr_value(attrs, "action").unwrap_or_default();
        let fields = input_fields(body);
        if !fields.is_empty() || !action.is_empty() {
            return Some(HtmlForm {
                method: method.to_lowercase(),
                action: resolve_url(base_url, &action),
                fields,
            });
        }
        rest = &rest[body_start + close..];
    }
    None
}

/// Every `<input name=... value=...>` in a fragment. Inputs without a value
/// still matter: the form submits them empty, and a missing key changes what
/// the server sees.
fn input_fields(body: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let mut rest = body;
    while let Some(open) = find_ci(rest, "<input") {
        let after = &rest[open..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        if let Some(name) = attr_value(tag, "name") {
            let value = attr_value(tag, "value").unwrap_or_default();
            fields.entry(name).or_insert(value);
        }
        rest = &rest[open + gt + 1..];
    }
    fields
}

/// Value of `attr="..."` or `attr='...'`, HTML-unescaped.
fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lowered = tag.to_lowercase();
    let mut from = 0usize;
    loop {
        let at = lowered[from..].find(attr)? + from;
        // Must be a whole attribute name, not a suffix of another one.
        let before_ok = at == 0
            || lowered.as_bytes()[at - 1].is_ascii_whitespace()
            || lowered.as_bytes()[at - 1] == b'<';
        let after = &lowered[at + attr.len()..];
        let trimmed = after.trim_start();
        if before_ok && trimmed.starts_with('=') {
            let eq = at + attr.len() + (after.len() - trimmed.len()) + 1;
            let value = tag[eq..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let end = value[1..].find(quote)? + 1;
                return Some(unescape(&value[1..end]));
            }
            let end = value
                .find(|c: char| c.is_whitespace())
                .unwrap_or(value.len());
            return Some(unescape(&value[..end]));
        }
        from = at + attr.len();
    }
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_lowercase().find(needle)
}

fn unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// Join a possibly relative form action against the page it came from.
fn resolve_url(base: &str, action: &str) -> String {
    if action.is_empty() {
        return base.to_string();
    }
    if action.starts_with("http://") || action.starts_with("https://") {
        return action.to_string();
    }
    let origin = origin_of(base);
    if action.starts_with('/') {
        return format!("{origin}{action}");
    }
    let path = base.split('?').next().unwrap_or(base);
    let dir = path.rfind('/').map(|i| &path[..i + 1]).unwrap_or(path);
    format!("{dir}{action}")
}

pub fn origin_of(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after = &url[scheme_end + 3..];
    let host_end = after.find('/').unwrap_or(after.len());
    format!("{}://{}", &url[..scheme_end], &after[..host_end])
}

/// The Next.js server-action id for the consent submission.
///
/// Without running the page's JavaScript this is the only handle on the
/// approval endpoint, so its absence means the page is not the one expected.
pub fn find_server_action_id(html: &str) -> Option<String> {
    // Prefer the action wired to consent submission; fall back to any action
    // id, since the bundle's shape changes between deploys.
    for anchor in ["submitOAuth2Consent", "device"] {
        if let Some(id) = action_id_near(html, Some(anchor)) {
            return Some(id);
        }
    }
    action_id_near(html, None)
}

fn action_id_near(html: &str, anchor: Option<&str>) -> Option<String> {
    let marker = "createServerReference)(\"";
    let mut from = 0usize;
    while let Some(at) = html[from..].find(marker) {
        let start = from + at + marker.len();
        let rest = &html[start..];
        let end = rest.find('"')?;
        let id = &rest[..end];
        let hexish = id.len() >= 40 && id.len() <= 44 && id.chars().all(|c| c.is_ascii_hexdigit());
        if hexish {
            match anchor {
                None => return Some(id.to_string()),
                Some(word) => {
                    // Only within this call expression. A fixed-width lookahead
                    // bleeds into the next `createServerReference` and would
                    // return the id of whichever action happens to come first.
                    let tail = &html[start + end..];
                    let window = tail.len().min(200);
                    let window = tail[..window]
                        .find(marker)
                        .or_else(|| tail[..window].find(");"))
                        .unwrap_or(window);
                    if tail[..window].contains(word) {
                        return Some(id.to_string());
                    }
                }
            }
        }
        from = start + end;
    }
    None
}

/// The account's principal id, matched to the email when one is known.
///
/// The id is embedded in the page's flight payload, where quoting varies: raw
/// JSON in some chunks, backslash-escaped in others.
pub fn find_principal_id(html: &str, email: Option<&str>) -> Option<String> {
    let want = email.map(|e| e.trim().to_lowercase());
    for (id, found_email) in uuid_email_pairs(html) {
        match &want {
            Some(want) if &found_email.to_lowercase() != want => continue,
            _ => return Some(id.to_lowercase()),
        }
    }
    None
}

/// Every (uuid, email) pair that appears close together, in either order.
fn uuid_email_pairs(html: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let ids = field_occurrences(html, "id");
    let emails = field_occurrences(html, "email");
    for (id_at, id) in &ids {
        if !looks_like_uuid(id) {
            continue;
        }
        for (email_at, email) in &emails {
            if email.contains('@') && id_at.abs_diff(*email_at) <= 200 {
                pairs.push((id.clone(), email.clone()));
            }
        }
    }
    pairs
}

/// Occurrences of `"field":"value"`, tolerating backslash-escaped quoting.
fn field_occurrences(html: &str, field: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (open, close) in [("\"", "\""), ("\\\"", "\\\"")] {
        let key = format!("{open}{field}{close}");
        let mut from = 0usize;
        while let Some(at) = html[from..].find(&key) {
            let start = from + at + key.len();
            let rest = html[start..].trim_start();
            let Some(rest) = rest.strip_prefix(':') else {
                from = start;
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix(open) else {
                from = start;
                continue;
            };
            let value_start = html.len() - rest.len();
            match rest.find(close) {
                Some(end) => {
                    out.push((value_start, rest[..end].to_string()));
                    from = value_start + end;
                }
                None => break,
            }
        }
    }
    out
}

fn looks_like_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && value.matches('-').count() == 4
}

/// `xxxx-xxxx`, upper-cased, as the device endpoints expect.
pub fn normalize_user_code(code: &str) -> String {
    code.trim().to_uppercase().replace(' ', "")
}

/// Strip a `sso=` prefix, any trailing cookie attributes, and control bytes.
pub fn normalize_sso(value: &str) -> String {
    let mut token = value.trim();
    if let Some(rest) = token
        .strip_prefix("sso=")
        .or_else(|| token.strip_prefix("SSO="))
    {
        token = rest.trim();
    }
    if let Some((first, _)) = token.split_once(';') {
        token = first.trim();
    }
    token
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_post_form_is_found_with_its_inputs() {
        let html = r#"
          <form method="POST" action="/oauth2/device/approve">
            <input type="hidden" name="user_code" value="ABCD-EFGH">
            <input type="hidden" name="principal_id" value="">
            <input type="submit" name="action" value="allow">
          </form>"#;
        let form = find_post_form(html, "https://accounts.x.ai/oauth2/device/consent").unwrap();
        assert_eq!(form.method, "post");
        assert_eq!(form.action, "https://accounts.x.ai/oauth2/device/approve");
        assert_eq!(form.fields.get("user_code").unwrap(), "ABCD-EFGH");
        assert_eq!(form.fields.get("action").unwrap(), "allow");
        // Present but empty is different from absent; the page ships it empty
        // and React fills it, which is exactly the field we must supply.
        assert_eq!(form.fields.get("principal_id").unwrap(), "");
    }

    #[test]
    fn a_relative_action_resolves_against_the_page() {
        let html = r#"<form method="post" action="approve"><input name="a" value="1"></form>"#;
        let form = find_post_form(html, "https://accounts.x.ai/oauth2/device/consent?x=1").unwrap();
        assert_eq!(form.action, "https://accounts.x.ai/oauth2/device/approve");
    }

    #[test]
    fn an_empty_action_posts_back_to_the_same_page() {
        let html = r#"<form method="post"><input name="a" value="1"></form>"#;
        let url = "https://accounts.x.ai/oauth2/device/consent";
        assert_eq!(find_post_form(html, url).unwrap().action, url);
    }

    #[test]
    fn html_entities_in_values_are_decoded() {
        let html = r#"<form method="post" action="/a?x=1&amp;y=2"><input name="t" value="a&quot;b"></form>"#;
        let form = find_post_form(html, "https://accounts.x.ai/").unwrap();
        assert_eq!(form.action, "https://accounts.x.ai/a?x=1&y=2");
        assert_eq!(form.fields.get("t").unwrap(), "a\"b");
    }

    #[test]
    fn a_page_with_no_form_yields_nothing() {
        assert!(find_post_form("<div>nothing here</div>", "https://x/").is_none());
    }

    #[test]
    fn pages_are_classified_by_url_first_then_content() {
        assert_eq!(
            classify_page("https://accounts.x.ai/oauth2/device/done", ""),
            PageKind::Done
        );
        assert_eq!(
            classify_page(
                "https://accounts.x.ai/oauth2/device/consent?user_code=A",
                ""
            ),
            PageKind::Consent
        );
        assert_eq!(
            classify_page("https://accounts.x.ai/sign-in", ""),
            PageKind::SignIn
        );
        assert_eq!(
            classify_page(
                "https://accounts.x.ai/oauth2/device",
                "Authorize Grok Build"
            ),
            PageKind::Consent
        );
        assert_eq!(
            classify_page("https://accounts.x.ai/oauth2/device", "enter your code"),
            PageKind::Device
        );
    }

    #[test]
    fn a_cloudflare_page_is_never_mistaken_for_the_app() {
        // Misreading a block as "unknown" would charge the failure to the
        // account instead of the egress.
        for body in [
            "<h1>Attention Required!</h1> Cloudflare",
            "<p>Blocked due to abusive traffic patterns</p>",
            "Error code 520",
        ] {
            assert_eq!(
                classify_page("https://accounts.x.ai/oauth2/device/consent", body),
                PageKind::Blocked,
                "{body}"
            );
        }
    }

    #[test]
    fn the_server_action_id_is_found_and_prefers_the_consent_one() {
        let html = r#"
          var a = (0,r.createServerReference)("00112233445566778899aabbccddeeff00112233", n, void 0, void 0, "other");
          var b = (0,r.createServerReference)("aabbccddeeff00112233445566778899aabbccdd", n, void 0, void 0, "submitOAuth2Consent");
        "#;
        assert_eq!(
            find_server_action_id(html).unwrap(),
            "aabbccddeeff00112233445566778899aabbccdd"
        );
    }

    #[test]
    fn any_action_id_is_accepted_when_none_is_named() {
        let html = r#"(0,r.createServerReference)("00112233445566778899aabbccddeeff00112233",n)"#;
        assert_eq!(
            find_server_action_id(html).unwrap(),
            "00112233445566778899aabbccddeeff00112233"
        );
    }

    #[test]
    fn a_page_without_an_action_id_reports_none() {
        assert!(find_server_action_id("<html>plain</html>").is_none());
        // Too short to be an action id.
        assert!(find_server_action_id(r#"createServerReference)("abc123")"#).is_none());
    }

    #[test]
    fn the_principal_id_is_read_from_plain_json() {
        let html = r#"{"user":{"id":"3f2504e0-4f89-11d3-9a0c-0305e82c3301","email":"a@b.c"}}"#;
        assert_eq!(
            find_principal_id(html, Some("a@b.c")).unwrap(),
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
        );
    }

    #[test]
    fn the_principal_id_is_read_from_escaped_flight_chunks() {
        let html = r#"self.__next_f.push([1,"{\"id\":\"3f2504e0-4f89-11d3-9a0c-0305e82c3301\",\"email\":\"a@b.c\"}"])"#;
        assert_eq!(
            find_principal_id(html, Some("a@b.c")).unwrap(),
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
        );
    }

    #[test]
    fn the_email_and_id_may_appear_in_either_order() {
        let html = r#"{"email":"a@b.c","name":"x","id":"3f2504e0-4f89-11d3-9a0c-0305e82c3301"}"#;
        assert_eq!(
            find_principal_id(html, Some("a@b.c")).unwrap(),
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301"
        );
    }

    #[test]
    fn a_principal_belonging_to_another_account_is_rejected() {
        // Approving with someone else's principal id succeeds and then the
        // token poll fails with "Access denied", which reads as a dead
        // credential rather than the wrong field.
        let html = r#"{"id":"3f2504e0-4f89-11d3-9a0c-0305e82c3301","email":"other@b.c"}"#;
        assert!(find_principal_id(html, Some("mine@b.c")).is_none());
    }

    #[test]
    fn without_an_expected_email_the_first_pair_is_taken() {
        let html = r#"{"id":"3f2504e0-4f89-11d3-9a0c-0305e82c3301","email":"any@b.c"}"#;
        assert!(find_principal_id(html, None).is_some());
    }

    #[test]
    fn a_uuid_far_from_any_email_is_not_a_principal() {
        let filler = "x".repeat(400);
        let html = format!(
            r#"{{"id":"3f2504e0-4f89-11d3-9a0c-0305e82c3301"}}{filler}{{"email":"a@b.c"}}"#
        );
        assert!(find_principal_id(&html, Some("a@b.c")).is_none());
    }

    #[test]
    fn sso_tokens_survive_being_pasted_as_a_cookie() {
        assert_eq!(normalize_sso("  abc123  "), "abc123");
        assert_eq!(normalize_sso("sso=abc123"), "abc123");
        assert_eq!(normalize_sso("sso=abc123; Path=/; HttpOnly"), "abc123");
        assert_eq!(normalize_sso("abc\r\n123"), "abc123");
    }

    #[test]
    fn user_codes_are_upper_cased_and_stripped() {
        assert_eq!(normalize_user_code(" 2gst-dw8v "), "2GST-DW8V");
        assert_eq!(normalize_user_code("2GST DW8V"), "2GSTDW8V");
    }

    #[test]
    fn origins_are_extracted_for_the_origin_header() {
        assert_eq!(
            origin_of("https://accounts.x.ai/oauth2/device/consent?x=1"),
            "https://accounts.x.ai"
        );
        assert_eq!(origin_of("https://auth.x.ai"), "https://auth.x.ai");
    }
}
