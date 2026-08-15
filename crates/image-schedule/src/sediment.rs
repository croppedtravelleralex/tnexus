//! Streaming `sediment://` extractor — gptimage `sediment.rs` subset.

use regex::Regex;
use std::sync::OnceLock;

fn sediment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"sediment://([A-Za-z0-9_-]+)").unwrap())
}

#[derive(Debug, Default, Clone)]
pub struct SedimentParser {
    ids: Vec<String>,
}

impl SedimentParser {
    pub fn feed(&mut self, chunk: &str) -> bool {
        let mut found = false;
        for cap in sediment_re().captures_iter(chunk) {
            let id = cap[1].to_string();
            if !id.is_empty() && !self.ids.contains(&id) {
                self.ids.push(id);
                found = true;
            }
        }
        found
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn ids_json(&self) -> String {
        serde_json::to_string(&self.ids).unwrap_or_else(|_| "[]".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sediment_ids() {
        let mut p = SedimentParser::default();
        assert!(p.feed("prefix sediment://abc123 suffix"));
        assert_eq!(p.ids(), &["abc123"]);
    }
}
