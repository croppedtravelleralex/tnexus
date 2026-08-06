//! G5-P4 视频生成端点（对齐 Go `gateway/video.go` 的创建 / 轮询语义）。
//!
//! 端点：
//! - `POST /v1/videos`：创建视频任务，返回 `VideoJob`（status=queued/processing）。
//! - `GET  /v1/videos/{id}`：轮询任务状态，最终 processing → completed/failed。
//!
//! 上游 IO（真实 provider 的视频生成 / 状态持久化）抽象为 [`VideoBackend`] trait，
//! 测试注入 fake（对齐 Go `video_test.go` 的 `videoUsageRepository` + adapter 模式）。
//! 本模块只做请求校验、状态映射与 HTTP 形状；轮询由调用方多次 GET 驱动。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::GatewayError;
use crate::router::AppState;

/// 视频任务状态（对齐 Go `media.Status` 的 queued/processing/completed/failed）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

impl VideoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoStatus::Queued => "queued",
            VideoStatus::Processing => "processing",
            VideoStatus::Completed => "completed",
            VideoStatus::Failed => "failed",
        }
    }
}

/// 任务失败详情（对齐 Go `media.Job.ErrorCode/ErrorMessage`）。
#[derive(Debug, Clone, Serialize)]
pub struct VideoError {
    pub code: String,
    pub message: String,
}

/// 视频任务快照（HTTP 返回形状；对齐 Go `media.Job` 的对外字段）。
#[derive(Debug, Clone, Serialize)]
pub struct VideoJob {
    pub id: String,
    pub object: &'static str,
    pub status: VideoStatus,
    /// 创建时刻 epoch 秒。
    pub created: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// 目标时长秒。
    pub duration: i64,
    pub aspect_ratio: String,
    pub resolution: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reference_urls: Vec<String>,
    /// 生成进度 0..=100（queued 时可为 0）。
    pub progress: i32,
    /// completed 后有效。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// failed 后有效。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<VideoError>,
}

/// `POST /v1/videos` 请求体（对齐 Go `VideoInput` 的子集）。
#[derive(Debug, Deserialize)]
pub struct VideoRequest {
    /// 对外模型名（本阶段透传给 backend）。
    #[serde(default)]
    pub model: String,
    /// 文本生视频必须提供；图片生视频可省略。
    #[serde(default)]
    pub prompt: Option<String>,
    /// 目标时长秒（默认 8，对齐 Go 常见默认）。
    #[serde(default)]
    pub duration: Option<i64>,
    /// 宽高比（如 "16:9"）。
    #[serde(default)]
    pub aspect_ratio: String,
    /// 分辨率（如 "720p"）。
    #[serde(default)]
    pub resolution: String,
    /// 图生视频参考图 URL。
    #[serde(default)]
    pub reference_urls: Vec<String>,
}

/// 视频生成上游抽象（对齐 Go `CreateVideo` / `GetVideo` 的 IO 面）。
#[async_trait::async_trait]
pub trait VideoBackend: Send + Sync {
    /// 创建视频任务，返回任务快照（status=queued/processing）。
    async fn create_video(&self, input: VideoRequest) -> Result<VideoJob, GatewayError>;
    /// 轮询任务当前状态；id 未命中返回 `NotFound`。
    async fn poll_video(&self, id: &str) -> Result<VideoJob, GatewayError>;
}

/// 提示词长度上限（对齐 Go `CreateVideo` 的 100000 校验）。
const MAX_PROMPT_CHARS: usize = 100_000;

/// `POST /v1/videos`（G5-P4）。校验 → backend.create_video → 201 + job。
pub async fn create_video(
    State(state): State<Arc<AppState>>,
    Json(req): Json<VideoRequest>,
) -> Result<Response, GatewayError> {
    // 文本生视频必须提供 prompt；图片生视频可省略（对齐 Go）。
    let has_prompt = req.prompt.as_deref().is_some_and(|p| !p.trim().is_empty());
    if !has_prompt && req.reference_urls.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "文本生视频必须提供 prompt；图片生视频可以省略 prompt".into(),
        ));
    }
    if let Some(prompt) = &req.prompt {
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(GatewayError::InvalidRequest(format!(
                "prompt 超过 {MAX_PROMPT_CHARS} 字符上限"
            )));
        }
    }
    if let Some(duration) = req.duration {
        if duration <= 0 {
            return Err(GatewayError::InvalidRequest("duration 必须为正数".into()));
        }
    }
    let backend = state
        .video_backend
        .as_ref()
        .ok_or_else(|| GatewayError::Internal("VideoBackend not configured".into()))?;
    let job = backend.create_video(req).await?;
    Ok(Json(job).into_response())
}

/// `GET /v1/videos/{id}`（G5-P4）。backend.poll_video；未命中 → 404。
pub async fn get_video(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, GatewayError> {
    if id.trim().is_empty() {
        return Err(GatewayError::NotFound("video id 为空".into()));
    }
    let backend = state
        .video_backend
        .as_ref()
        .ok_or_else(|| GatewayError::Internal("VideoBackend not configured".into()))?;
    let job = backend.poll_video(&id).await?;
    Ok(Json(job).into_response())
}

/// 由 create/poll 结果构造失败快照的便利函数（backend 用）。
pub fn failed_job(id: &str, code: &str, message: &str) -> VideoJob {
    VideoJob {
        id: id.to_string(),
        object: "video",
        status: VideoStatus::Failed,
        created: now_epoch(),
        prompt: None,
        duration: 0,
        aspect_ratio: String::new(),
        resolution: String::new(),
        reference_urls: Vec::new(),
        progress: 0,
        url: None,
        content_type: None,
        error: Some(VideoError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}

/// 当前 epoch 秒（job.created 用）。
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 让 GatewayError 能直接作为错误 JSON（测试断言用）。
pub fn error_json(err: &GatewayError) -> serde_json::Value {
    json!({ "error": { "message": err.to_string() } })
}
