//! 媒体与生图时间线域（对齐 Go `media` + `image-timeline`；数据源：
//! tnexus 图片归档 / `job_results` / `grok_image_pipeline`）。
//!
//! 内存 fake 可测；SQL 由 grok-storage 提供（TODO）。

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::AdminResult;

/// 单张媒体图片（对齐 Go `mediaResponse` 子集）。
#[derive(Debug, Clone, Serialize)]
pub struct MediaImageView {
    pub asset_id: String,
    pub account_id: i64,
    pub provider: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
}

/// 媒体统计。
#[derive(Debug, Clone, Default, Serialize)]
pub struct MediaStatsView {
    pub total_images: i64,
    pub total_bytes: i64,
    /// 近 24h 新增。
    pub recent_24h: i64,
}

/// 生图时间线条目（对齐 Go `image-timeline`）。
#[derive(Debug, Clone, Serialize)]
pub struct ImageTimelineEntry {
    pub account_name: String,
    pub provider: String,
    pub upstream_model: String,
    pub status: String,
    /// 耗时毫秒。
    pub latency_ms: i64,
    pub created_at: DateTime<Utc>,
}

/// 媒体/时间线存储抽象。
#[async_trait]
pub trait MediaStore: Send + Sync {
    async fn list_images(&self, page: i64, page_size: i64) -> AdminResult<Vec<MediaImageView>>;
    async fn media_stats(&self) -> AdminResult<MediaStatsView>;
    async fn timeline(&self, limit: usize) -> AdminResult<Vec<ImageTimelineEntry>>;
    /// 单张详情（不存在 → Ok(None)，service 转 404）。
    async fn get_image(&self, asset_id: &str) -> AdminResult<Option<MediaImageView>>;
    /// 大小分布汇总。
    async fn size_summary(&self) -> AdminResult<MediaSizeSummaryView>;
}

/// 图片大小分布汇总（对齐 Go `/media/size-summary`）。
#[derive(Debug, Clone, Serialize)]
pub struct MediaSizeSummaryView {
    pub total_images: i64,
    pub total_bytes: i64,
    /// 按 KB 分桶：<100KB / <1MB / <10MB / >=10MB。
    pub buckets: Vec<SizeBucket>,
}

/// 单桶统计。
#[derive(Debug, Clone, Serialize)]
pub struct SizeBucket {
    pub label: String,
    pub count: i64,
}

/// 媒体域服务。
pub struct MediaService {
    store: std::sync::Arc<dyn MediaStore>,
}

impl MediaService {
    pub fn new(store: std::sync::Arc<dyn MediaStore>) -> Self {
        Self { store }
    }

    pub async fn list_images(&self, page: i64, page_size: i64) -> AdminResult<Vec<MediaImageView>> {
        self.store
            .list_images(page.max(1), page_size.clamp(1, 100))
            .await
    }

    pub async fn media_stats(&self) -> AdminResult<MediaStatsView> {
        self.store.media_stats().await
    }

    pub async fn timeline(&self, limit: usize) -> AdminResult<Vec<ImageTimelineEntry>> {
        self.store.timeline(limit.clamp(1, 200)).await
    }

    /// 单张详情（无 → NotFound）。
    pub async fn get_image(&self, asset_id: &str) -> AdminResult<MediaImageView> {
        self.store
            .get_image(asset_id)
            .await?
            .ok_or_else(|| crate::error::AdminError::NotFound(format!("media {asset_id}")))
    }

    /// 大小分布汇总。
    pub async fn size_summary(&self) -> AdminResult<MediaSizeSummaryView> {
        self.store.size_summary().await
    }
}
