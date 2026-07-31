mod upstream;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::{PgPool, Row};
use std::time::{Duration, Instant};
use tnexus_domain::agent::{
    build_director_system_prompt, build_image_prompt, parse_director_response_with_fallback,
    DirectorOutput,
};
use tnexus_domain::factors::FactorPoint;
use tnexus_domain::gen_config::GenConfig;
use tnexus_domain::job::{JobStatus, WorkflowPath};
use tnexus_storage::{AssetStorage, R2Config};
use upstream::{agent_prompt_text, api_model_name, keywords_json, ImageGenOptions, UpstreamClient};
use uuid::Uuid;

const JOB_QUEUE_KEY: &str = "tnexus:jobs";
const JOB_EVENTS_PREFIX: &str = "tnexus:job_events:";

#[derive(Clone)]
struct WorkerConfig {
    database_url: String,
    redis_url: String,
    gptimage_base: String,
    grok2api_base: String,
    director_model: String,
    chatgpt_image_model: String,
    grok_image_model: String,
    upstream_api_key: Option<String>,
    r2: Option<R2Config>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tnexus_worker=info".into()),
        )
        .init();

    let cfg = load_config()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&cfg.database_url)
        .await?;

    let redis_client = redis::Client::open(cfg.redis_url.as_str())?;
    let mut redis = ConnectionManager::new(redis_client).await?;

    let storage = if let Some(r2) = &cfg.r2 {
        Some(AssetStorage::from_config(r2).await?)
    } else {
        None
    };

    let upstream = UpstreamClient {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?,
        gptimage_base: cfg.gptimage_base.clone(),
        grok2api_base: cfg.grok2api_base.clone(),
        director_model: cfg.director_model.clone(),
        chatgpt_image_model: cfg.chatgpt_image_model.clone(),
        grok_image_model: cfg.grok_image_model.clone(),
        api_key: cfg.upstream_api_key.clone(),
    };

    tracing::info!("tnexus-worker started");

    loop {
        let job_id: Option<(String, String)> = redis.blpop(JOB_QUEUE_KEY, 5.0).await?;
        let Some((_key, job_id)) = job_id else {
            continue;
        };
        let job_id = Uuid::parse_str(&job_id).context("parse job id")?;
        if let Err(e) = process_job(
            &pool,
            &mut redis,
            &upstream,
            storage.as_ref(),
            job_id,
        )
        .await
        {
            let err_msg = format!("{e:#}");
            tracing::error!(%job_id, error = %err_msg, "job failed");
            let _ = set_status(&pool, job_id, JobStatus::Failed, Some(&err_msg)).await;
            let _ = publish_event(
                &mut redis,
                job_id,
                JobStatus::Failed,
                0,
                None,
                Some(&err_msg),
            )
            .await;
        }
    }
}

async fn process_job(
    pool: &PgPool,
    redis: &mut ConnectionManager,
    upstream: &UpstreamClient,
    storage: Option<&AssetStorage>,
    job_id: Uuid,
) -> Result<()> {
    let wall_start = Instant::now();
    let job = load_job(pool, job_id).await?;
    let queue_wait_ms = (chrono::Utc::now() - job.created_at)
        .num_milliseconds()
        .max(0) as u64;
    let mut phase_timings = serde_json::Map::new();
    phase_timings.insert("task_queue_ms".into(), serde_json::json!(queue_wait_ms));

    publish_event(redis, job_id, JobStatus::Directing, 25, None, None).await?;
    set_status(pool, job_id, JobStatus::Directing, None).await?;

    let workflow = parse_workflow(&job.workflow_path)?;
    let director_factors: FactorPoint = serde_json::from_value(job.director_factors)?;
    let ps_factors: FactorPoint = serde_json::from_value(job.ps_factors)?;
    let director_params = director_factors.director_params();
    let ps_params = ps_factors.ps_params();

    let system = build_director_system_prompt(workflow, &director_params, &job.input_prompt);
    let director_model_ids = if job.director_models.is_empty() {
        vec!["gpt".to_string()]
    } else {
        job.director_models.clone()
    };
    let image_providers = image_providers_for(&job.provider);

    for (mi, model_id) in director_model_ids.iter().enumerate() {
        let count = actor_count_for(&job.actor_image_counts, model_id);
        let img_opts = ImageGenOptions {
            size: job.gen_config.size_string(),
            count,
            quality: if job.gen_config.quality == "auto" {
                None
            } else {
                Some(job.gen_config.quality.clone())
            },
            transparent_bg: job.gen_config.transparent_bg,
        };

        let api_model = if model_id == "gpt" {
            upstream.director_model.as_str()
        } else {
            api_model_name(model_id)
        };
        let director_start = Instant::now();
        let raw = match upstream
            .director_chat(api_model, &system, &job.input_prompt)
            .await
        {
            Ok(s) if !s.trim().is_empty() => s,
            Ok(_) | Err(_) => {
                tracing::warn!(model_id, "director chat empty/failed; using input prompt fallback");
                serde_json::json!({ "prompt": job.input_prompt }).to_string()
            }
        };
        phase_timings.insert(
            "ps_ms".into(),
            serde_json::json!(director_start.elapsed().as_millis() as u64),
        );
        let director_out = parse_director_response_with_fallback(workflow, &raw, &job.input_prompt)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let (image_prompt, ps_enabled) =
            build_image_prompt(workflow, &director_out, &ps_params, job.ps_enabled);
        let agent_prompt = agent_prompt_text(&director_out);

        publish_event(redis, job_id, JobStatus::Generating, 55, None, None).await?;
        set_status(pool, job_id, JobStatus::Generating, None).await?;

        for img_provider in &image_providers {
            let generate_start = Instant::now();
            let generated_list = match img_provider.as_str() {
                "chatgpt" => upstream.generate_chatgpt(&image_prompt, ps_enabled, &img_opts).await?,
                "grok" => upstream.generate_grok(&image_prompt, ps_enabled, &img_opts).await?,
                _ => continue,
            };
            phase_timings.insert(
                "sse_stream_ms".into(),
                serde_json::json!(generate_start.elapsed().as_millis() as u64),
            );

            publish_event(redis, job_id, JobStatus::Uploading, 85, None, None).await?;
            set_status(pool, job_id, JobStatus::Uploading, None).await?;

            let upload_start = Instant::now();

            let result_label = if director_model_ids.len() > 1 || image_providers.len() > 1 {
                format!("{model_id}:{img_provider}")
            } else {
                model_id.clone()
            };

            for (vi, generated) in generated_list.iter().enumerate() {
                if let Some(url) = &generated.source_url {
                    sqlx::query(
                        r#"INSERT INTO job_results (job_id, provider, variant_index, source_url, agent_prompt, revised_prompt, keywords)
                           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
                    )
                    .bind(job_id)
                    .bind(&result_label)
                    .bind(vi as i32)
                    .bind(url)
                    .bind(&agent_prompt)
                    .bind(&generated.revised_prompt)
                    .bind(keywords_json(&director_out))
                    .execute(pool)
                    .await?;
                    continue;
                }

                let bytes = generated
                    .bytes
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("image payload missing bytes and url"))?;

                let (orig, prev, thumb) = if let Some(storage) = storage {
                    let asset = storage
                        .store_image_variants(job.user_id, job_id, bytes)
                        .await?;
                    (
                        Some(asset.original_key),
                        Some(asset.preview_key),
                        Some(asset.thumb_key),
                    )
                } else {
                    (None, None, None)
                };

                let inline_preview = if storage.is_none() {
                    Some(STANDARD.encode(bytes))
                } else {
                    None
                };

                sqlx::query(
                    r#"INSERT INTO job_results (job_id, provider, variant_index, r2_key_original, r2_key_preview, r2_key_thumb, agent_prompt, revised_prompt, keywords, inline_preview_b64)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
                )
                .bind(job_id)
                .bind(&result_label)
                .bind(vi as i32)
                .bind(orig)
                .bind(prev)
                .bind(thumb)
                .bind(&agent_prompt)
                .bind(&generated.revised_prompt)
                .bind(keywords_json(&director_out))
                .bind(inline_preview)
                .execute(pool)
                .await?;
            }
            phase_timings.insert(
                "download_ms".into(),
                serde_json::json!(upload_start.elapsed().as_millis() as u64),
            );
            record_usage_event("studio@local", "default", "images_api", true);
        }

        if mi + 1 < director_model_ids.len() {
            publish_event(redis, job_id, JobStatus::Directing, 35, None, None).await?;
        }
    }

    phase_timings.insert(
        "wall_clock_ms".into(),
        serde_json::json!(wall_start.elapsed().as_millis() as u64),
    );
    save_phase_timings(pool, job_id, &phase_timings).await?;
    set_status(pool, job_id, JobStatus::Done, None).await?;
    publish_event(redis, job_id, JobStatus::Done, 100, None, None).await?;
    Ok(())
}

struct JobRow {
    user_id: Uuid,
    mode: String,
    workflow_path: String,
    ps_enabled: bool,
    provider: String,
    director_models: Vec<String>,
    gen_config: GenConfig,
    actor_image_counts: serde_json::Value,
    director_factors: serde_json::Value,
    ps_factors: serde_json::Value,
    input_prompt: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn load_job(pool: &PgPool, job_id: Uuid) -> Result<JobRow> {
    let row = sqlx::query(
        "SELECT user_id, mode, workflow_path, ps_enabled, provider, director_models, gen_config, actor_image_counts, director_factors, ps_factors, input_prompt, created_at FROM jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    let director_models: serde_json::Value = row.get("director_models");
    let gen_config: serde_json::Value = row.get("gen_config");
    let actor_image_counts: serde_json::Value = row.get("actor_image_counts");
    let models: Vec<String> = serde_json::from_value(director_models).unwrap_or_else(|_| vec!["gpt".into()]);
    Ok(JobRow {
        user_id: row.get("user_id"),
        mode: row.get("mode"),
        workflow_path: row.get("workflow_path"),
        ps_enabled: row.get("ps_enabled"),
        provider: row.get("provider"),
        director_models: models,
        gen_config: serde_json::from_value(gen_config).unwrap_or_default(),
        actor_image_counts,
        director_factors: row.get("director_factors"),
        ps_factors: row.get("ps_factors"),
        input_prompt: row.get("input_prompt"),
        created_at: row.get("created_at"),
    })
}

async fn save_phase_timings(
    pool: &PgPool,
    job_id: Uuid,
    timings: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    sqlx::query("UPDATE jobs SET phase_timings_ms = $2, updated_at = NOW() WHERE id = $1")
        .bind(job_id)
        .bind(serde_json::Value::Object(timings.clone()))
        .execute(pool)
        .await?;
    Ok(())
}

fn record_usage_event(email: &str, binding: &str, metric: &str, ok: bool) {
    let path = std::env::var("USAGE_EVENTS_FILE")
        .unwrap_or_else(|_| "data/usage_events.ndjson".into());
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let payload = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "email": email,
        "binding": binding,
        "metric": metric,
        "ok": ok,
    });
    if let Ok(line) = serde_json::to_string(&payload) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

async fn set_status(
    pool: &PgPool,
    job_id: Uuid,
    status: JobStatus,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE jobs SET status = $2, error_message = $3, updated_at = NOW() WHERE id = $1")
        .bind(job_id)
        .bind(status.as_str())
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

async fn publish_event(
    redis: &mut ConnectionManager,
    job_id: Uuid,
    stage: JobStatus,
    progress: u8,
    preview_url: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let payload = serde_json::json!({
        "job_id": job_id,
        "stage": stage.as_str(),
        "progress": progress,
        "preview_url": preview_url,
        "error": error,
    });
    let channel = format!("{JOB_EVENTS_PREFIX}{job_id}");
    redis
        .publish::<_, _, ()>(channel, payload.to_string())
        .await?;
    Ok(())
}

fn parse_workflow(s: &str) -> Result<WorkflowPath> {
    match s {
        "full_agent" => Ok(WorkflowPath::FullAgent),
        "keyword_ps" => Ok(WorkflowPath::KeywordPs),
        _ => Err(anyhow::anyhow!("bad workflow")),
    }
}

fn image_providers_for(provider: &str) -> Vec<String> {
    match provider {
        "grok" => vec!["grok".into()],
        "both" => vec!["chatgpt".into(), "grok".into()],
        _ => vec!["chatgpt".into()],
    }
}

fn actor_count_for(counts: &serde_json::Value, model_id: &str) -> u32 {
    counts
        .get(model_id)
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 10) as u32
}

fn load_config() -> Result<WorkerConfig> {
    let r2 = if std::env::var("R2_BUCKET").is_ok() {
        Some(R2Config {
            account_id: std::env::var("R2_ACCOUNT_ID")?,
            access_key_id: std::env::var("R2_ACCESS_KEY_ID")?,
            secret_access_key: std::env::var("R2_SECRET_ACCESS_KEY")?,
            bucket: std::env::var("R2_BUCKET")?,
            endpoint: std::env::var("R2_ENDPOINT").ok(),
        })
    } else {
        None
    };
    Ok(WorkerConfig {
        database_url: std::env::var("DATABASE_URL")?,
        redis_url: std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
        gptimage_base: std::env::var("GPTIMAGE_BASE")
            .or_else(|_| std::env::var("UPSTREAM_API_BASE"))
            .unwrap_or_else(|_| "http://127.0.0.1:8012".into()),
        grok2api_base: std::env::var("GROK2API_BASE")
            .or_else(|_| std::env::var("UPSTREAM_API_BASE"))
            .unwrap_or_else(|_| "http://127.0.0.1:18000".into()),
        director_model: std::env::var("DIRECTOR_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        chatgpt_image_model: std::env::var("CHATGPT_IMAGE_MODEL")
            .unwrap_or_else(|_| "gpt-image-2".into()),
        grok_image_model: std::env::var("GROK_IMAGE_MODEL")
            .unwrap_or_else(|_| "grok-imagine-image".into()),
        upstream_api_key: std::env::var("UPSTREAM_API_KEY").ok(),
        r2,
    })
}
