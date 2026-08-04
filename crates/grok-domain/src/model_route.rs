//! 模型路由领域类型，对应 Go `domain/model` 与 `grok_model_routes` / `grok_model_route_aliases`。

use serde::{Deserialize, Serialize};

/// 对外模型路由（骨架）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    pub id: i64,
    pub public_id: String,
    pub upstream_model: String,
    pub enabled: bool,
}

/// 模型别名（含 G1 拟议 `grok-vision-ocr`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouteAlias {
    pub id: i64,
    pub alias: String,
    pub route_id: i64,
}
