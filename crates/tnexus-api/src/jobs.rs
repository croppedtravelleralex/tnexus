use crate::models::{JobDetail, JobListItem, JobRecord, JobResultRecord, JobResultView};
use crate::state::AppState;
use anyhow::{Context, Result};
use sqlx::Row;
use tnexus_domain::factors::FactorPoint;
use tnexus_domain::gen_config::GenConfig;
use uuid::Uuid;

pub async fn create_job(
    state: &AppState,
    user_id: Uuid,
    mode: &str,
    workflow_path: &str,
    ps_enabled: bool,
    provider: &str,
    director_models: Vec<String>,
    gen_config: GenConfig,
    director_factors: FactorPoint,
    ps_factors: FactorPoint,
    input_prompt: &str,
    conversation_id: Option<Uuid>,
    actor_image_counts: serde_json::Value,
) -> Result<JobRecord> {
    let row = sqlx::query(
        r#"INSERT INTO jobs (user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, director_factors, ps_factors, input_prompt, conversation_id, actor_image_counts, status)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'queued')
           RETURNING id, user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, director_factors, ps_factors, input_prompt, status, error_message, phase_timings_ms, created_at, updated_at"#,
    )
    .bind(user_id)
    .bind(mode)
    .bind(workflow_path)
    .bind(ps_enabled)
    .bind(provider)
    .bind(serde_json::to_value(&director_models)?)
    .bind(serde_json::to_value(&gen_config)?)
    .bind(serde_json::to_value(director_factors)?)
    .bind(serde_json::to_value(ps_factors)?)
    .bind(input_prompt)
    .bind(conversation_id)
    .bind(actor_image_counts)
    .fetch_one(&state.pool)
    .await?;
    row_to_job(row)
}

pub async fn get_job(state: &AppState, job_id: Uuid, user_id: Option<Uuid>) -> Result<Option<JobRecord>> {
    let row = if let Some(uid) = user_id {
        sqlx::query(
            "SELECT id, user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, director_factors, ps_factors, input_prompt, status, error_message, phase_timings_ms, created_at, updated_at FROM jobs WHERE id = $1 AND user_id = $2",
        )
        .bind(job_id)
        .bind(uid)
        .fetch_optional(&state.pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, director_factors, ps_factors, input_prompt, status, error_message, phase_timings_ms, created_at, updated_at FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&state.pool)
        .await?
    };
    row.map(row_to_job).transpose()
}

pub async fn list_jobs(state: &AppState, user_id: Uuid, limit: i64) -> Result<Vec<JobRecord>> {
    let rows = sqlx::query(
        "SELECT id, user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, director_factors, ps_factors, input_prompt, status, error_message, phase_timings_ms, created_at, updated_at FROM jobs WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter().map(row_to_job).collect()
}

pub async fn list_job_summaries(state: &AppState, user_id: Uuid, limit: i64) -> Result<Vec<JobListItem>> {
    let rows = sqlx::query(
        r#"SELECT j.id, j.input_prompt, j.status, j.created_at, j.updated_at,
                  COUNT(r.id)::bigint AS result_count,
                  (SELECT id FROM job_results WHERE job_id = j.id ORDER BY variant_index LIMIT 1) AS thumb_result_id,
                  (SELECT r2_key_thumb FROM job_results WHERE job_id = j.id ORDER BY variant_index LIMIT 1) AS thumb_key,
                  (SELECT source_url FROM job_results WHERE job_id = j.id ORDER BY variant_index LIMIT 1) AS source_url
           FROM jobs j
           LEFT JOIN job_results r ON r.job_id = j.id
           WHERE j.user_id = $1
           GROUP BY j.id
           ORDER BY j.created_at DESC
           LIMIT $2"#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.get("id");
        let thumb_result_id: Option<Uuid> = row.get("thumb_result_id");
        let source_url: Option<String> = row.get("source_url");
        let thumb_url = if let Some(url) = source_url.filter(|s| !s.is_empty()) {
            if url.contains("/v1/images/assets/") {
                thumb_result_id.map(|id| thumb_api_url(id, 120))
            } else {
                Some(url)
            }
        } else if let Some(id) = thumb_result_id {
            Some(thumb_api_url(id, 120))
        } else {
            None
        };
        out.push(JobListItem {
            id,
            input_prompt: row.get("input_prompt"),
            status: row.get("status"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            result_count: row.get("result_count"),
            thumb_url,
        });
    }
    Ok(out)
}

pub async fn delete_jobs(state: &AppState, user_id: Uuid, ids: &[Uuid]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM jobs WHERE user_id = $1 AND id = ANY($2)")
        .bind(user_id)
        .bind(ids)
        .execute(&state.pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn update_job_status(
    state: &AppState,
    job_id: Uuid,
    status: &str,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = $2, error_message = $3, updated_at = NOW() WHERE id = $1",
    )
    .bind(job_id)
    .bind(status)
    .bind(error_message)
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn list_results(state: &AppState, job_id: Uuid) -> Result<Vec<JobResultRecord>> {
    let rows = sqlx::query(
        "SELECT id, job_id, provider, variant_index, r2_key_original, r2_key_preview, r2_key_thumb, agent_prompt, revised_prompt, keywords, inline_preview_b64, source_url, width, height, size_bytes, created_at FROM job_results WHERE job_id = $1 ORDER BY provider, variant_index",
    )
    .bind(job_id)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter().map(row_to_result).collect()
}

pub async fn get_job_detail(state: &AppState, job_id: Uuid, user_id: Uuid) -> Result<Option<JobDetail>> {
    let job = get_job(state, job_id, Some(user_id)).await?;
    let Some(job) = job else {
        return Ok(None);
    };
    let results = list_results(state, job_id).await?;
    let mut views = Vec::new();
    for r in results {
        views.push(result_to_view(state, r).await?);
    }
    Ok(Some(JobDetail { job, results: views }))
}

pub async fn get_job_status(
    state: &AppState,
    job_id: Uuid,
    user_id: Uuid,
) -> Result<Option<(String, Option<String>)>> {
    let row = sqlx::query(
        "SELECT status, error_message FROM jobs WHERE id = $1 AND user_id = $2",
    )
    .bind(job_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| (r.get("status"), r.get("error_message"))))
}

fn thumb_api_url(result_id: Uuid, width: u32) -> String {
    format!("/api/images/thumb/{result_id}?w={width}")
}

fn original_api_url(result_id: Uuid) -> String {
    format!("/api/images/original/{result_id}")
}

fn result_image_meta(r: &JobResultRecord) -> (Option<i32>, Option<i32>, Option<i64>) {
    (r.width, r.height, r.size_bytes)
}

pub async fn result_to_view(_state: &AppState, r: JobResultRecord) -> Result<JobResultView> {
    let (width, height, size_bytes) = result_image_meta(&r);
    let has_persisted = r
        .inline_preview_b64
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || r
            .r2_key_thumb
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        || r
            .r2_key_preview
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        || r
            .r2_key_original
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
    if let Some(url) = r
        .source_url
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
    {
        let keywords = r.keywords.and_then(|v| serde_json::from_value(v).ok());
        if url.contains("/v1/images/assets/") {
            if has_persisted {
                return Ok(JobResultView {
                    id: r.id,
                    provider: r.provider,
                    preview_url: Some(thumb_api_url(r.id, 1280)),
                    download_url: Some(original_api_url(r.id)),
                    thumb_url: Some(thumb_api_url(r.id, 240)),
                    preview_b64: None,
                    b64_json: None,
                    agent_prompt: r.agent_prompt,
                    revised_prompt: r.revised_prompt,
                    keywords,
                    width,
                    height,
                    size_bytes,
                });
            }
            return Ok(JobResultView {
                id: r.id,
                provider: r.provider,
                preview_url: Some(thumb_api_url(r.id, 512)),
                download_url: Some(url),
                thumb_url: Some(thumb_api_url(r.id, 240)),
                preview_b64: None,
                b64_json: None,
                agent_prompt: r.agent_prompt,
                revised_prompt: r.revised_prompt,
                keywords,
                width,
                height,
                size_bytes,
            });
        }
        if has_persisted {
            let thumb = thumb_api_url(r.id, 1280);
            return Ok(JobResultView {
                id: r.id,
                provider: r.provider,
                preview_url: Some(thumb.clone()),
                download_url: Some(original_api_url(r.id)),
                thumb_url: Some(thumb_api_url(r.id, 240)),
                preview_b64: None,
                b64_json: None,
                agent_prompt: r.agent_prompt,
                revised_prompt: r.revised_prompt,
                keywords,
                width,
                height,
                size_bytes,
            });
        }
        return Ok(JobResultView {
            id: r.id,
            provider: r.provider,
            preview_url: Some(url.clone()),
            download_url: Some(url.clone()),
            thumb_url: Some(url),
            preview_b64: None,
            b64_json: None,
            agent_prompt: r.agent_prompt,
            revised_prompt: r.revised_prompt,
            keywords,
            width,
            height,
            size_bytes,
        });
    }
    let has_inline = r
        .inline_preview_b64
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let has_file = r
        .r2_key_original
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        || r
            .r2_key_preview
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
        || r
            .r2_key_thumb
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
    let preview_url = if has_file || has_inline {
        Some(thumb_api_url(r.id, 1280))
    } else {
        None
    };
    let download_url = if has_file || has_inline {
        Some(original_api_url(r.id))
    } else {
        None
    };
    let thumb_url = if has_file || has_inline {
        Some(thumb_api_url(r.id, 240))
    } else {
        None
    };
    let keywords = r.keywords.and_then(|v| serde_json::from_value(v).ok());
    Ok(JobResultView {
        id: r.id,
        provider: r.provider,
        preview_url,
        download_url,
        thumb_url,
        preview_b64: None,
        b64_json: None,
        agent_prompt: r.agent_prompt,
        revised_prompt: r.revised_prompt,
        keywords,
        width,
        height,
        size_bytes,
    })
}

fn row_to_job(row: sqlx::postgres::PgRow) -> Result<JobRecord> {
    let director_factors: serde_json::Value = row.get("director_factors");
    let ps_factors: serde_json::Value = row.get("ps_factors");
    let director_models: serde_json::Value = row.get("director_models");
    let gen_config: serde_json::Value = row.get("gen_config");
    Ok(JobRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        mode: row.get("mode"),
        workflow_path: row.get("workflow_path"),
        ps_enabled: row.get("ps_enabled"),
        provider: row.get("provider"),
        director_models: serde_json::from_value(director_models).unwrap_or_else(|_| vec!["gpt".into()]),
        gen_config: serde_json::from_value(gen_config).unwrap_or_default(),
        director_factors: serde_json::from_value(director_factors).unwrap_or_default(),
        ps_factors: serde_json::from_value(ps_factors).unwrap_or_default(),
        input_prompt: row.get("input_prompt"),
        status: row.get("status"),
        error_message: row.get("error_message"),
        phase_timings_ms: row.get("phase_timings_ms"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_result(row: sqlx::postgres::PgRow) -> Result<JobResultRecord> {
    Ok(JobResultRecord {
        id: row.get("id"),
        job_id: row.get("job_id"),
        provider: row.get("provider"),
        r2_key_original: row.get("r2_key_original"),
        r2_key_preview: row.get("r2_key_preview"),
        r2_key_thumb: row.get("r2_key_thumb"),
        agent_prompt: row.get("agent_prompt"),
        revised_prompt: row.get("revised_prompt"),
        keywords: row.get("keywords"),
        inline_preview_b64: row.get("inline_preview_b64"),
        source_url: row.get("source_url"),
        width: row.get("width"),
        height: row.get("height"),
        size_bytes: row.get("size_bytes"),
        created_at: row.get("created_at"),
    })
}
