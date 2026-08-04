//! 生图流水线阶段，对应 Go `domain/imagepipeline` 与 `grok_pipeline_segments.stage`。

use serde::{Deserialize, Serialize};

/// 流水线阶段（G2 起写 `grok_pipeline_segments`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    PromptEnhance,
    Dispatch,
    Inflight,
    Completed,
}

/// 流水线段（骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSegment {
    pub trace_id: i64,
    pub sequence: i32,
    pub stage: Stage,
    pub duration_ms: i64,
}
