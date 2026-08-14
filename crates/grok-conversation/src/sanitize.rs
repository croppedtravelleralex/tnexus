//! 剥离 grok.com 上游 SSE 中的专有 markup（`<grok:render>` 等），避免透传到客户端。

/// 去掉完整的 `<grok:…>` 标签；流式场景下未闭合的尾部标签一并丢弃。
pub fn strip_grok_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(start) = rest.find("<grok:") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let tag = &rest[start..];
        match grok_tag_end(tag) {
            Some(end) => rest = &tag[end..],
            None => break, // 未闭合：流式增量尚未到齐，不把半截标签发出去
        }
    }
    out
}

/// `tag` 以 `<grok:` 开头。返回该标签（含闭合）占用的字节数。
fn grok_tag_end(tag: &str) -> Option<usize> {
    let gt = tag.find('>')?;
    if tag.as_bytes().get(gt.saturating_sub(1)) == Some(&b'/') {
        return Some(gt + 1);
    }
    let after_prefix = tag.get(6..)?;
    let name_len = after_prefix
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .unwrap_or(after_prefix.len());
    if name_len == 0 {
        return Some(gt + 1);
    }
    let name = &after_prefix[..name_len];
    let close = format!("</grok:{name}>");
    tag.find(&close).map(|i| i + close.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_plain_markdown() {
        let s = "**是的**，价格一样。\n\n### 其他方面\n- **速度**";
        assert_eq!(strip_grok_markup(s), s);
    }

    #[test]
    fn strips_inline_citation_card() {
        let s = concat!(
            "**是的**，目前公开标价一致",
            r#"<grok:render card_id="69b0ae" card_type="citation_card" type="render_inline_citation">"#,
            r#"<argument name="citation_id">0</argument>"#,
            "</grok:render>",
            "。",
        );
        assert_eq!(strip_grok_markup(s), "**是的**，目前公开标价一致。");
    }

    #[test]
    fn strips_multiple_and_self_closing() {
        let s = "A<grok:render x=\"1\"/>B<grok:render>c</grok:render>C";
        assert_eq!(strip_grok_markup(s), "ABC");
    }

    #[test]
    fn drops_incomplete_trailing_tag() {
        assert_eq!(strip_grok_markup("hello<grok:ren"), "hello");
        assert_eq!(
            strip_grok_markup("hello<grok:render card_id=\"x\">partial"),
            "hello"
        );
    }

    #[test]
    fn nested_argument_body() {
        let s = "x<grok:card><argument>1</argument><argument>2</argument></grok:card>y";
        assert_eq!(strip_grok_markup(s), "xy");
    }

    #[test]
    fn empty_name_open_tag_is_dropped() {
        assert_eq!(strip_grok_markup("a<grok:>b"), "ab");
    }

    #[test]
    fn empty_and_no_tags() {
        assert_eq!(strip_grok_markup(""), "");
        assert_eq!(strip_grok_markup("<div>not grok</div>"), "<div>not grok</div>");
    }

    #[test]
    fn dotted_tag_name() {
        let s = "a<grok:render.card>z</grok:render.card>b";
        assert_eq!(strip_grok_markup(s), "ab");
    }
}
