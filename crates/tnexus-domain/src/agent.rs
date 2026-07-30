use crate::factors::{DirectorParams, PsParams};
use crate::job::WorkflowPath;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorFullAgentOutput {
    pub prompt: String,
    pub style_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorKeywordOutput {
    pub keywords: Vec<String>,
    pub user_intent: String,
}

#[derive(Debug, Clone)]
pub enum DirectorOutput {
    FullAgent(DirectorFullAgentOutput),
    KeywordPs(DirectorKeywordOutput),
}

pub fn build_director_system_prompt(
    workflow: WorkflowPath,
    params: &DirectorParams,
    user_input: &str,
) -> String {
    let mode = match workflow {
        WorkflowPath::FullAgent => "full_agent",
        WorkflowPath::KeywordPs => "keyword_ps",
    };
    format!(
        r#"You are a visual director for AI image generation.
User input: {user_input}
Creative factors (0.0-1.0):
- divergence (exploratory vs concrete): {divergence:.2}
- specificity: {specificity:.2}
- mood (emotional atmosphere): {mood:.2}
- technical (technical detail): {technical:.2}

Mode: {mode}
If full_agent: respond ONLY with JSON {{"prompt":"english image prompt","style_notes":"brief notes"}}
If keyword_ps: respond ONLY with JSON {{"keywords":["kw1","kw2","kw3"],"user_intent":"short intent"}}
Use 2-4 keywords for keyword_ps. Prompts must be in English."#,
        user_input = user_input.trim(),
        divergence = params.divergence,
        specificity = params.specificity,
        mood = params.mood,
        technical = params.technical,
    )
}

pub fn build_image_prompt(
    workflow: WorkflowPath,
    director: &DirectorOutput,
    ps: &PsParams,
    ps_enabled: bool,
) -> (String, bool) {
    match (workflow, director) {
        (WorkflowPath::FullAgent, DirectorOutput::FullAgent(out)) => {
            let mut prompt = out.prompt.clone();
            if ps_enabled {
                prompt.push_str(&format_ps_suffix(ps));
            }
            (prompt, ps_enabled)
        }
        (WorkflowPath::KeywordPs, DirectorOutput::KeywordPs(out)) => {
            let keywords = out.keywords.join(", ");
            let mut prompt = format!(
                "{}. Style anchors: {}.",
                out.user_intent.trim(),
                keywords
            );
            prompt.push_str(&format_ps_suffix(ps));
            (prompt, true)
        }
        _ => (
            "A beautiful cinematic scene".to_string(),
            ps_enabled,
        ),
    }
}

fn format_ps_suffix(ps: &PsParams) -> String {
    format!(
        " [style modifiers: detail={:.2}, lighting={:.2}]",
        ps.detail_level, ps.lighting_drama
    )
}

pub fn parse_director_response(
    workflow: WorkflowPath,
    raw: &str,
) -> Result<DirectorOutput, String> {
    parse_director_response_with_fallback(workflow, raw, "")
}

pub fn parse_director_response_with_fallback(
    workflow: WorkflowPath,
    raw: &str,
    user_input: &str,
) -> Result<DirectorOutput, String> {
    let json = extract_json_value(raw)?;
    parse_director_json(workflow, &json, user_input)
}

fn extract_json_value(raw: &str) -> Result<serde_json::Value, String> {
    let mut trimmed = raw.trim();
    if trimmed.starts_with("```") {
        trimmed = trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_object() {
            return Ok(v);
        }
    }
    let json_start = trimmed.find('{').ok_or("no json in director response")?;
    let json_end = trimmed.rfind('}').ok_or("no json end in director response")?;
    serde_json::from_str(&trimmed[json_start..=json_end])
        .map_err(|e| format!("invalid director json: {e}"))
}

fn parse_director_json(
    workflow: WorkflowPath,
    json: &serde_json::Value,
    user_input: &str,
) -> Result<DirectorOutput, String> {
    match workflow {
        WorkflowPath::FullAgent => {
            let mut prompt = json
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if prompt.is_empty() {
                prompt = json
                    .get("image_prompt")
                    .or_else(|| json.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
            }
            if prompt.is_empty() && !user_input.trim().is_empty() {
                prompt = user_input.trim().to_string();
            }
            if prompt.is_empty() {
                return Err("empty prompt from director".into());
            }
            Ok(DirectorOutput::FullAgent(DirectorFullAgentOutput {
                prompt,
                style_notes: json
                    .get("style_notes")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }))
        }
        WorkflowPath::KeywordPs => {
            let keywords: Vec<String> = json
                .get("keywords")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let user_intent = json
                .get("user_intent")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if keywords.is_empty() || user_intent.is_empty() {
                return Err("invalid keyword_ps director output".into());
            }
            Ok(DirectorOutput::KeywordPs(DirectorKeywordOutput {
                keywords,
                user_intent,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_ps_forces_enhance() {
        let out = DirectorOutput::KeywordPs(DirectorKeywordOutput {
            keywords: vec!["neon".into()],
            user_intent: "cyber city".into(),
        });
        let (prompt, enhance) = build_image_prompt(
            WorkflowPath::KeywordPs,
            &out,
            &PsParams {
                detail_level: 0.5,
                lighting_drama: 0.5,
            },
            false,
        );
        assert!(enhance);
        assert!(prompt.contains("Style anchors"));
    }
}
