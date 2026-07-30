use crate::factors::FactorPoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobMode {
    Director,
    Casting,
}

impl JobMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Director => "director",
            Self::Casting => "casting",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPath {
    FullAgent,
    KeywordPs,
}

impl WorkflowPath {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullAgent => "full_agent",
            Self::KeywordPs => "keyword_ps",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageProvider {
    Chatgpt,
    Grok,
    Both,
}

impl ImageProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chatgpt => "chatgpt",
            Self::Grok => "grok",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Directing,
    Generating,
    Uploading,
    Done,
    Failed,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Directing => "directing",
            Self::Generating => "generating",
            Self::Uploading => "uploading",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub mode: JobMode,
    pub workflow_path: WorkflowPath,
    pub ps_enabled: bool,
    pub provider: ImageProvider,
    pub director_factors: FactorPoint,
    pub ps_factors: FactorPoint,
    pub input_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEvent {
    pub job_id: uuid::Uuid,
    pub stage: JobStatus,
    pub progress: u8,
    pub message: Option<String>,
    pub preview_url: Option<String>,
    pub error: Option<String>,
}
