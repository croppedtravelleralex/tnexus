//! Egress Scope 领域类型，对应 Go `domain/egress`。
//! 允许值见 docs/39b §3.2：grok_build / grok_web / grok_web_asset / grok_console。
//! `grok_web_expand` 仅运行时并发闸门，不入 DB CHECK。

use serde::{Deserialize, Serialize};

/// Egress 出口 Scope。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    GrokBuild,
    GrokWeb,
    GrokWebAsset,
    GrokConsole,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::GrokBuild => "grok_build",
            Scope::GrokWeb => "grok_web",
            Scope::GrokWebAsset => "grok_web_asset",
            Scope::GrokConsole => "grok_console",
        }
    }
}
