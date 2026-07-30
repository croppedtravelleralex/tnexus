use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenConfig {
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default)]
    pub transparent_bg: bool,
}

fn default_quality() -> String {
    "auto".into()
}
fn default_width() -> u32 {
    1024
}
fn default_height() -> u32 {
    1024
}
fn default_count() -> u32 {
    1
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            quality: default_quality(),
            width: default_width(),
            height: default_height(),
            count: default_count(),
            transparent_bg: false,
        }
    }
}

impl GenConfig {
    pub fn size_string(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}
