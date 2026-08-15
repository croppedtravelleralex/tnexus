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
    /// 0.0–1.0 润色强度；控制上游 prompt_enhance
    #[serde(default)]
    pub polish_factor: f32,
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
            polish_factor: 0.0,
        }
    }
}

impl GenConfig {
    pub fn size_string(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

/// Align with gptimage `conversation.build_image_prompt` — upstream ChatGPT reads size/quality from prompt hints.
pub fn append_image_generation_hints(
    prompt: &str,
    size: &str,
    quality: &str,
    transparent_bg: bool,
) -> String {
    let base = prompt.trim();
    let mut hints = Vec::new();
    let size = size.trim();
    if !size.is_empty() {
        hints.push(format!("输出图片尺寸为 {size}。"));
    }
    let quality = quality.trim();
    if !quality.is_empty() && !quality.eq_ignore_ascii_case("auto") {
        hints.push(format!("输出图片质量为 {quality}。"));
    }
    if transparent_bg {
        hints.push("输出图片背景为透明。".to_string());
    }
    if hints.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        hints.join("")
    } else {
        format!("{base}\n\n{}", hints.join(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_image_generation_hints_matches_gptimage_shape() {
        let out = append_image_generation_hints("a cat", "1792x1024", "high", true);
        assert!(out.contains("a cat"));
        assert!(out.contains("输出图片尺寸为 1792x1024"));
        assert!(out.contains("输出图片质量为 high"));
        assert!(out.contains("透明"));
    }
}
