mod pipeline_telemetry;
mod upstream;

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::future::try_join_all;
use image::GenericImageView;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::{PgPool, Row};
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tnexus_domain::agent::{
    build_director_system_prompt, build_image_prompt, parse_director_response_with_fallback,
};
use tnexus_domain::factors::FactorPoint;
use tnexus_domain::gen_config::GenConfig;
use tnexus_domain::job::{JobStatus, WorkflowPath};
use tnexus_storage::ImageStore;
use tokio::sync::Semaphore;
use upstream::{
    agent_prompt_text, api_model_name, keywords_json, ImageGenOptions, SlotGenerateTask,
    UpstreamClient,
};
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
    image_response_format: String,
    image_parallel_concurrency: usize,
}

struct ActorPlan {
    model_id: String,
    image_prompt: String,
    ps_enabled: bool,
    agent_prompt: String,
    keywords: Option<serde_json::Value>,
    count: u32,
}

struct SlotPersistTask {
    result_label: String,
    variant_index: i32,
    agent_prompt: String,
    keywords: Option<serde_json::Value>,
    generated: upstream::GeneratedImage,
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
        .max_connections(32)
        .connect(&cfg.database_url)
        .await?;

    let redis_client = redis::Client::open(cfg.redis_url.as_str())?;
    let mut redis = ConnectionManager::new(redis_client).await?;

    let image_store = ImageStore::from_env().await?;
    if let Some(store) = &image_store {
        tracing::info!(backend = store.backend_name(), "image store enabled");
    } else {
        tracing::warn!("no image store configured; falling back to inline DB blobs");
    }

    let upstream = UpstreamClient {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .pool_max_idle_per_host(64)
            .build()?,
        gptimage_base: cfg.gptimage_base.clone(),
        grok2api_base: cfg.grok2api_base.clone(),
        director_model: cfg.director_model.clone(),
        chatgpt_image_model: cfg.chatgpt_image_model.clone(),
        grok_image_model: cfg.grok_image_model.clone(),
        api_key: cfg.upstream_api_key.clone(),
        image_response_format: cfg.image_response_format.clone(),
        image_parallel_concurrency: cfg.image_parallel_concurrency,
    };

    tracing::info!(
        image_format = %cfg.image_response_format,
        image_parallel = cfg.image_parallel_concurrency,
        "tnexus-worker started"
    );

    loop {
        let job_id: Option<(String, String)> = redis.blpop(JOB_QUEUE_KEY, 5.0).await?;
        let Some((_key, job_id)) = job_id else {
            continue;
        };
        let job_id = Uuid::parse_str(&job_id).context("parse job id")?;
        if let Err(e) =
            process_job(&pool, &mut redis, &upstream, image_store.as_ref(), job_id).await
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
    image_store: Option<&ImageStore>,
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

    let system = build_director_system_prompt(workflow, &director_params, &job.input_prompt);
    let director_model_ids = if job.director_models.is_empty() {
        vec!["gpt".to_string()]
    } else {
        job.director_models.clone()
    };
    let image_providers = image_providers_for(&job.provider);
    let img_opts_base = ImageGenOptions {
        size: job.gen_config.size_string(),
        quality: if job.gen_config.quality == "auto" {
            None
        } else {
            Some(job.gen_config.quality.clone())
        },
        transparent_bg: job.gen_config.transparent_bg,
    };

    let director_start = Instant::now();
    let ps_enabled_job = job.ps_enabled;
    let polish_factor = job.gen_config.polish_factor.clamp(0.0, 1.0);
    let upstream_enhance = ps_enabled_job || polish_factor >= 0.35;
    let ps_params = ps_factors.ps_params();
    let director_futs = director_model_ids.iter().map(|model_id| {
        let ps_params = ps_params.clone();
        let model_id = model_id.clone();
        let system = system.clone();
        let input = job.input_prompt.clone();
        let upstream = upstream.clone();
        let count = actor_count_for(&job.actor_image_counts, &model_id);
        async move {
            let api_model = if model_id == "gpt" {
                upstream.director_model.as_str()
            } else {
                api_model_name(&model_id)
            };
            let raw = match upstream.director_chat(api_model, &system, &input).await {
                Ok(s) if !s.trim().is_empty() => s,
                Ok(_) | Err(_) => {
                    tracing::warn!(
                        model_id,
                        "director chat empty/failed; using input prompt fallback"
                    );
                    serde_json::json!({ "prompt": input }).to_string()
                }
            };
            let director_out = parse_director_response_with_fallback(workflow, &raw, &input)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let (mut image_prompt, _) =
                build_image_prompt(workflow, &director_out, &ps_params, upstream_enhance);
            image_prompt = tnexus_domain::append_polish_intensity(&image_prompt, polish_factor);
            Ok::<ActorPlan, anyhow::Error>(ActorPlan {
                model_id,
                image_prompt,
                ps_enabled: upstream_enhance,
                agent_prompt: agent_prompt_text(&director_out),
                keywords: keywords_json(&director_out),
                count,
            })
        }
    });
    let actors: Vec<ActorPlan> = try_join_all(director_futs).await?;
    phase_timings.insert(
        "ps_ms".into(),
        serde_json::json!(director_start.elapsed().as_millis() as u64),
    );

    let mut gen_tasks = Vec::new();
    let mut persist_plan: Vec<(String, i32, String, Option<serde_json::Value>)> = Vec::new();
    let mut variant_index = 0i32;
    for actor in &actors {
        for img_provider in &image_providers {
            let result_label = if actors.len() > 1 || image_providers.len() > 1 {
                format!("{}:{img_provider}", actor.model_id)
            } else {
                actor.model_id.clone()
            };
            for _ in 0..actor.count {
                let hinted_prompt = tnexus_domain::append_image_generation_hints(
                    &actor.image_prompt,
                    &img_opts_base.size,
                    img_opts_base.quality.as_deref().unwrap_or("auto"),
                    img_opts_base.transparent_bg,
                );
                // Parallel slots share the same director prompt — tag each slot so gateway
                // duplicate_prompt gate does not 429 casting batches.
                let slot_prompt = format!(
                    "{}\n[tnexus-slot:{}]",
                    hinted_prompt.trim_end(),
                    variant_index
                );
                gen_tasks.push(SlotGenerateTask {
                    img_provider: img_provider.clone(),
                    prompt: slot_prompt,
                    ps_enabled: actor.ps_enabled,
                    opts: img_opts_base.clone(),
                });
                persist_plan.push((
                    result_label.clone(),
                    variant_index,
                    actor.agent_prompt.clone(),
                    actor.keywords.clone(),
                ));
                variant_index += 1;
            }
        }
    }

    publish_event(redis, job_id, JobStatus::Generating, 55, None, None).await?;
    set_status(pool, job_id, JobStatus::Generating, None).await?;

    let generate_start = Instant::now();
    let total_slots = gen_tasks.len().max(1);
    let parallel_cap = if upstream.image_parallel_concurrency == 0 {
        total_slots
    } else {
        upstream.image_parallel_concurrency
    };
    let sem = Arc::new(Semaphore::new(parallel_cap));
    let redis_slots = redis.clone();
    let slot_futs =
        gen_tasks
            .into_iter()
            .zip(persist_plan)
            .enumerate()
            .map(|(slot_index, (task, plan))| {
                let sem = sem.clone();
                let upstream = upstream.clone();
                let pool = pool.clone();
                let redis_cm = redis_slots.clone();
                let image_store = image_store.cloned();
                let user_id = job.user_id;
                let (result_label, variant_index, agent_prompt, keywords) = plan;
                async move {
                    let _permit = sem
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("parallel semaphore closed"))?;
                    let slot_start = Instant::now();
                    let generated = upstream.generate_slot(&task).await?;
                    let generation_ms = slot_start.elapsed().as_millis() as u64;

                    if let Some(pipeline) = &generated.pipeline {
                        let email = pipeline
                            .get("account_email")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        record_usage_event(email, "", "images_api", true);
                        pipeline_telemetry::append_event(&pipeline_telemetry::PipelineEvent {
                            ts: pipeline_telemetry::now_rfc3339(),
                            kind: "worker_slot".into(),
                            email: email.to_string(),
                            job_id: Some(job_id.to_string()),
                            slot_index: Some(slot_index as i32),
                            ok: true,
                            quota_before: pipeline.get("quota_before").and_then(|v| v.as_i64()),
                            quota_after: pipeline.get("quota_after").and_then(|v| v.as_i64()),
                            timings_ms: pipeline.get("timings_ms").cloned(),
                            bytes: pipeline.get("bytes").cloned(),
                            extra: None,
                        });
                    }

                    let result_id = persist_slot(
                        &pool,
                        image_store.as_ref(),
                        user_id,
                        job_id,
                        SlotPersistTask {
                            result_label,
                            variant_index,
                            agent_prompt,
                            keywords,
                            generated,
                        },
                        generation_ms,
                    )
                    .await?;

                    let completed = slot_index + 1;
                    let progress = 55 + ((30 * completed) / total_slots) as u8;
                    let mut redis_pub = redis_cm.clone();
                    publish_slot_done(
                        &mut redis_pub,
                        job_id,
                        slot_index,
                        variant_index,
                        result_id,
                        generation_ms,
                        progress,
                    )
                    .await?;

                    Ok::<(usize, u64), anyhow::Error>((slot_index, generation_ms))
                }
            });
    let slot_outcomes = try_join_all(slot_futs).await?;
    phase_timings.insert(
        "sse_stream_ms".into(),
        serde_json::json!(generate_start.elapsed().as_millis() as u64),
    );

    let mut slot_metrics: Vec<serde_json::Value> = Vec::new();
    for (slot_index, generation_ms) in &slot_outcomes {
        slot_metrics.push(serde_json::json!({
            "slot_index": slot_index,
            "job_id": job_id.to_string(),
            "generation_ms": generation_ms,
        }));
    }
    if !slot_metrics.is_empty() {
        phase_timings.insert("slots".into(), serde_json::Value::Array(slot_metrics));
    }
    if let Some(lat) = aggregate_latency_percentiles_from_slot_ms(&slot_outcomes) {
        phase_timings.insert("latency_percentiles_ms".into(), lat);
    }

    publish_event(redis, job_id, JobStatus::Uploading, 85, None, None).await?;
    set_status(pool, job_id, JobStatus::Uploading, None).await?;

    let upload_start = Instant::now();
    phase_timings.insert(
        "download_ms".into(),
        serde_json::json!(upload_start.elapsed().as_millis() as u64),
    );

    phase_timings.insert(
        "wall_clock_ms".into(),
        serde_json::json!(wall_start.elapsed().as_millis() as u64),
    );
    save_phase_timings(pool, job_id, &phase_timings).await?;
    set_status(pool, job_id, JobStatus::Done, None).await?;
    publish_event(redis, job_id, JobStatus::Done, 100, None, None).await?;
    Ok(())
}

async fn persist_slot(
    pool: &PgPool,
    image_store: Option<&ImageStore>,
    user_id: Uuid,
    job_id: Uuid,
    task: SlotPersistTask,
    generation_ms: u64,
) -> Result<Uuid> {
    let SlotPersistTask {
        result_label,
        variant_index,
        agent_prompt,
        keywords,
        generated,
    } = task;

    let source_url = generated.source_url.clone();
    let bytes = if let Some(b) = generated.bytes {
        Some(b)
    } else if let Some(url) = &source_url {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        let resp = http
            .get(url.as_str())
            .send()
            .await
            .with_context(|| format!("download image {url}"))?;
        let status = resp.status();
        let body = resp.bytes().await.context("read downloaded image")?;
        if !status.is_success() {
            anyhow::bail!("download image HTTP {status}");
        }
        Some(body.to_vec())
    } else {
        return Err(anyhow::anyhow!("image payload missing bytes and url"));
    };

    let (orig, prev, thumb) = if let (Some(store), Some(bytes)) = (image_store, bytes.as_ref()) {
        let asset = store.store_image_variants(user_id, job_id, bytes).await?;
        (
            Some(asset.original_key),
            Some(asset.preview_key),
            Some(asset.thumb_key),
        )
    } else {
        (None, None, None)
    };

    let inline_preview = if image_store.is_none() {
        bytes.as_ref().map(|b| STANDARD.encode(b))
    } else {
        None
    };

    let (width, height, size_bytes) = bytes
        .as_ref()
        .map(|b| {
            let dims = image::ImageReader::new(Cursor::new(b.as_slice()))
                .with_guessed_format()
                .ok()
                .and_then(|reader| reader.decode().ok())
                .map(|img| img.dimensions());
            (
                dims.map(|(w, _)| w as i32),
                dims.map(|(_, h)| h as i32),
                Some(b.len() as i64),
            )
        })
        .unwrap_or((None, None, None));

    let row = sqlx::query(
        r#"INSERT INTO job_results (job_id, provider, variant_index, r2_key_original, r2_key_preview, r2_key_thumb, agent_prompt, revised_prompt, keywords, inline_preview_b64, source_url, width, height, size_bytes, generation_ms, pipeline)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
           RETURNING id"#,
    )
    .bind(job_id)
    .bind(&result_label)
    .bind(variant_index)
    .bind(orig)
    .bind(prev)
    .bind(thumb)
    .bind(&agent_prompt)
    .bind(&generated.revised_prompt)
    .bind(keywords)
    .bind(inline_preview)
    .bind(source_url)
    .bind(width)
    .bind(height)
    .bind(size_bytes)
    .bind(generation_ms as i64)
    .bind(&generated.pipeline)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
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
    let models: Vec<String> =
        serde_json::from_value(director_models).unwrap_or_else(|_| vec!["gpt".into()]);
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

fn aggregate_latency_percentiles_from_slot_ms(slots: &[(usize, u64)]) -> Option<serde_json::Value> {
    if slots.is_empty() {
        return None;
    }
    let mut walls: Vec<u64> = slots.iter().map(|(_, ms)| *ms).collect();
    walls.sort_unstable();
    let n = walls.len();
    let p = |pct: f64| -> u64 {
        if n == 1 {
            return walls[0];
        }
        let idx = pct * (n as f64 - 1.0);
        let lo = idx.floor() as usize;
        let hi = idx.ceil() as usize;
        let frac = idx - lo as f64;
        (walls[lo] as f64 + frac * (walls[hi] as f64 - walls[lo] as f64)).round() as u64
    };
    Some(serde_json::json!({
        "p50": p(0.50),
        "p95": p(0.95),
        "p99": p(0.99),
        "min": walls[0],
        "max": walls[n - 1],
        "samples": n,
    }))
}

fn record_usage_event(email: &str, binding: &str, metric: &str, ok: bool) {
    let path =
        std::env::var("USAGE_EVENTS_FILE").unwrap_or_else(|_| "data/usage_events.ndjson".into());
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
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
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
    sqlx::query(
        "UPDATE jobs SET status = $2, error_message = $3, updated_at = NOW() WHERE id = $1",
    )
    .bind(job_id)
    .bind(status.as_str())
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

async fn publish_slot_done(
    redis: &mut ConnectionManager,
    job_id: Uuid,
    slot_index: usize,
    variant_index: i32,
    result_id: Uuid,
    generation_ms: u64,
    progress: u8,
) -> Result<()> {
    let payload = serde_json::json!({
        "event": "slot_done",
        "job_id": job_id,
        "stage": JobStatus::Generating.as_str(),
        "progress": progress,
        "slot_index": slot_index,
        "variant_index": variant_index,
        "result_id": result_id,
        "generation_ms": generation_ms,
        "preview_url": format!("/api/images/thumb/{result_id}?w=512"),
        "thumb_url": format!("/api/images/thumb/{result_id}?w=240"),
        "download_url": format!("/api/images/original/{result_id}"),
    });
    let channel = format!("{JOB_EVENTS_PREFIX}{job_id}");
    redis
        .publish::<_, _, ()>(channel, payload.to_string())
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

const MAX_ACTOR_IMAGE_COUNT: u32 = 40;

fn actor_count_for(counts: &serde_json::Value, model_id: &str) -> u32 {
    counts
        .get(model_id)
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, MAX_ACTOR_IMAGE_COUNT as u64) as u32
}

fn load_config() -> Result<WorkerConfig> {
    let image_parallel_concurrency = std::env::var("IMAGE_PARALLEL_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    Ok(WorkerConfig {
        database_url: std::env::var("DATABASE_URL")?,
        redis_url: std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
        gptimage_base: std::env::var("GPTIMAGE_BASE")
            .or_else(|_| std::env::var("GATEWAY_BASE"))
            .or_else(|_| std::env::var("UPSTREAM_API_BASE"))
            .unwrap_or_else(|_| "http://127.0.0.1:8014".into()),
        grok2api_base: std::env::var("GROK2API_BASE")
            .or_else(|_| std::env::var("UPSTREAM_API_BASE"))
            .unwrap_or_else(|_| "http://127.0.0.1:8000".into()),
        director_model: std::env::var("DIRECTOR_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        chatgpt_image_model: std::env::var("CHATGPT_IMAGE_MODEL")
            .unwrap_or_else(|_| "gpt-image-2".into()),
        grok_image_model: std::env::var("GROK_IMAGE_MODEL")
            .unwrap_or_else(|_| "grok-imagine-image".into()),
        upstream_api_key: std::env::var("UPSTREAM_API_KEY").ok(),
        image_response_format: std::env::var("IMAGE_RESPONSE_FORMAT")
            .unwrap_or_else(|_| "url".into()),
        image_parallel_concurrency,
    })
}
