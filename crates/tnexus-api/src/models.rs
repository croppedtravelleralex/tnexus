use serde::{Deserialize, Serialize};
use tnexus_domain::factors::FactorPoint;
use tnexus_domain::gen_config::GenConfig;
use tnexus_domain::job::{ImageProvider, JobMode, WorkflowPath};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub mode: String,
    pub workflow_path: String,
    pub ps_enabled: bool,
    pub provider: String,
    pub director_models: Vec<String>,
    #[serde(default)]
    pub gen_config: GenConfig,
    pub director_factors: FactorPoint,
    pub ps_factors: FactorPoint,
    pub input_prompt: String,
    pub status: String,
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_timings_ms: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobListItem {
    pub id: Uuid,
    pub input_prompt: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub result_count: i64,
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResultRecord {
    pub id: Uuid,
    pub job_id: Uuid,
    pub provider: String,
    pub r2_key_original: Option<String>,
    pub r2_key_preview: Option<String>,
    pub r2_key_thumb: Option<String>,
    pub agent_prompt: Option<String>,
    pub revised_prompt: Option<String>,
    pub keywords: Option<serde_json::Value>,
    pub inline_preview_b64: Option<String>,
    pub source_url: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobDetail {
    #[serde(flatten)]
    pub job: JobRecord,
    pub results: Vec<JobResultView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobResultView {
    pub id: Uuid,
    pub provider: String,
    pub preview_url: Option<String>,
    pub download_url: Option<String>,
    pub thumb_url: Option<String>,
    /// Raw base64 PNG (no data: prefix) when inline preview is available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    pub agent_prompt: Option<String>,
    pub revised_prompt: Option<String>,
    pub keywords: Option<Vec<String>>,
}

pub fn parse_mode(s: &str) -> Option<JobMode> {
    match s {
        "director" => Some(JobMode::Director),
        "casting" => Some(JobMode::Casting),
        _ => None,
    }
}

pub fn parse_workflow(s: &str) -> Option<WorkflowPath> {
    match s {
        "full_agent" => Some(WorkflowPath::FullAgent),
        "keyword_ps" => Some(WorkflowPath::KeywordPs),
        _ => None,
    }
}

pub fn parse_director_models(models: &[String]) -> bool {
    const ALLOWED: &[&str] = &["gpt", "grok", "deepseek", "mimo", "hy3"];
    !models.is_empty() && models.iter().all(|m| ALLOWED.contains(&m.as_str()))
}

pub fn parse_provider(s: &str) -> Option<ImageProvider> {
    match s {
        "chatgpt" => Some(ImageProvider::Chatgpt),
        "grok" => Some(ImageProvider::Grok),
        "both" => Some(ImageProvider::Both),
        _ => None,
    }
}
