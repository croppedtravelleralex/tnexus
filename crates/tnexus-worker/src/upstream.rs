use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures::future::try_join_all;
use serde::Deserialize;
use std::sync::Arc;
use tnexus_domain::agent::DirectorOutput;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct UpstreamClient {
    pub http: reqwest::Client,
    pub gptimage_base: String,
    pub grok2api_base: String,
    pub director_model: String,
    pub chatgpt_image_model: String,
    pub grok_image_model: String,
    pub api_key: Option<String>,
    /// `url` (default) or `b64_json`
    pub image_response_format: String,
    /// 0 = unlimited parallel image HTTP requests
    pub image_parallel_concurrency: usize,
}

impl UpstreamClient {
    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            req.bearer_auth(key)
        } else {
            req
        }
    }

    fn url_mode(&self) -> bool {
        !self.image_response_format.eq_ignore_ascii_case("b64_json")
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageGenerationResponse {
    data: Vec<ImageDataItem>,
    #[serde(default)]
    _tnexus_pipeline: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ImageDataItem {
    b64_json: Option<String>,
    url: Option<String>,
    revised_prompt: Option<String>,
}

#[derive(Clone)]
pub struct ImageGenOptions {
    pub size: String,
    pub quality: Option<String>,
    pub transparent_bg: bool,
}

pub struct GeneratedImage {
    pub bytes: Option<Vec<u8>>,
    pub source_url: Option<String>,
    pub revised_prompt: Option<String>,
    pub pipeline: Option<serde_json::Value>,
}

pub struct SlotGenerateTask {
    pub img_provider: String,
    pub prompt: String,
    pub ps_enabled: bool,
    pub opts: ImageGenOptions,
}

impl UpstreamClient {
    pub async fn director_chat(
        &self,
        model: &str,
        system_prompt: &str,
        user_input: &str,
    ) -> Result<String> {
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_input}
            ],
            "temperature": 0.7
        });
        let resp = self
            .authed(
                self.http
                    .post(format!(
                        "{}/v1/chat/completions",
                        self.gptimage_base.trim_end_matches('/')
                    ))
                    .json(&body),
            )
            .send()
            .await
            .context("director chat request")?;
        let status = resp.status();
        let text = resp.text().await.context("director chat body")?;
        if !status.is_success() {
            return Err(anyhow::anyhow!("director chat HTTP {status}: {text}"));
        }
        let resp: ChatCompletionResponse =
            serde_json::from_str(&text).context("director chat json")?;
        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("empty director response"))?;
        Ok(content)
    }

    pub async fn generate_slot(&self, task: &SlotGenerateTask) -> Result<GeneratedImage> {
        let format = if self.url_mode() { "url" } else { "b64_json" };
        let base = match task.img_provider.as_str() {
            "grok" => &self.grok2api_base,
            _ => &self.gptimage_base,
        };
        let model = match task.img_provider.as_str() {
            "grok" => &self.grok_image_model,
            _ => &self.chatgpt_image_model,
        };
        let enhance_field = "prompt_enhance";
        self.generate_one(
            base,
            model,
            &task.prompt,
            task.ps_enabled,
            enhance_field,
            &task.opts,
            format,
        )
        .await
    }

    pub async fn generate_slots_parallel(
        &self,
        tasks: Vec<SlotGenerateTask>,
    ) -> Result<Vec<GeneratedImage>> {
        if tasks.is_empty() {
            return Ok(vec![]);
        }
        let sem = if self.image_parallel_concurrency == 0 {
            None
        } else {
            Some(Arc::new(Semaphore::new(self.image_parallel_concurrency)))
        };
        let futs = tasks.into_iter().map(|task| {
            let upstream = self.clone();
            let sem = sem.clone();
            async move {
                if let Some(s) = sem {
                    let _permit = s
                        .acquire()
                        .await
                        .map_err(|_| anyhow::anyhow!("parallel semaphore closed"))?;
                }
                upstream.generate_slot(&task).await
            }
        });
        try_join_all(futs).await
    }

    async fn generate_one(
        &self,
        base: &str,
        model: &str,
        prompt: &str,
        prompt_enhance: bool,
        enhance_field: &str,
        opts: &ImageGenOptions,
        response_format: &str,
    ) -> Result<GeneratedImage> {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": opts.size,
            "response_format": response_format
        });
        if let Some(obj) = body.as_object_mut() {
            obj.insert(enhance_field.to_string(), serde_json::json!(prompt_enhance));
            if let Some(q) = &opts.quality {
                if q != "auto" {
                    obj.insert("quality".to_string(), serde_json::json!(q));
                }
            }
            if opts.transparent_bg {
                obj.insert("background".to_string(), serde_json::json!("transparent"));
            }
        }
        let resp = self
            .authed(
                self.http
                    .post(format!(
                        "{}/v1/images/generations",
                        base.trim_end_matches('/')
                    ))
                    .json(&body),
            )
            .send()
            .await
            .context("image generation request")?;
        let status = resp.status();
        let text = resp.text().await.context("image generation body")?;
        if !status.is_success() {
            let hint = if status.as_u16() == 401 && text.contains("invalid session") {
                "（Gateway JWT 过期：在 Panda 运行 deploy/panda/refresh_upstream_jwt.sh 后重启 worker）"
            } else {
                ""
            };
            return Err(anyhow::anyhow!("image generation HTTP {status}: {text}{hint}"));
        }
        let resp: ImageGenerationResponse =
            serde_json::from_str(&text).context("image generation json")?;
        let pipeline = resp._tnexus_pipeline.clone();
        let item = resp
            .data
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no image payload"))?;

        if let Some(url) = item
            .url
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if self.url_mode() {
                return Ok(GeneratedImage {
                    bytes: None,
                    source_url: Some(url.to_string()),
                    revised_prompt: item.revised_prompt,
                    pipeline,
                });
            }
            let resp = self
                .http
                .get(url)
                .send()
                .await
                .with_context(|| format!("fetch image url {url}"))?;
            let status = resp.status();
            let bytes = resp
                .bytes()
                .await
                .with_context(|| format!("read image body from {url}"))?;
            if !status.is_success() {
                return Err(anyhow::anyhow!(
                    "fetch image url HTTP {status}: {}",
                    String::from_utf8_lossy(&bytes)
                ));
            }
            return Ok(GeneratedImage {
                bytes: Some(bytes.to_vec()),
                source_url: Some(url.to_string()),
                revised_prompt: item.revised_prompt,
                pipeline,
            });
        }

        let bytes = if let Some(b64) = &item.b64_json {
            STANDARD.decode(b64).context("decode b64")?
        } else {
            return Err(anyhow::anyhow!("no image payload"));
        };
        Ok(GeneratedImage {
            bytes: Some(bytes),
            source_url: None,
            revised_prompt: item.revised_prompt,
            pipeline,
        })
    }
}

pub fn api_model_name(id: &str) -> &str {
    match id {
        "gpt" => "gpt-5-mini",
        "grok" => "gpt-5-mini",
        "deepseek" => "deepseek-v4-flash",
        "mimo" => "mimo-v2.5",
        "hy3" => "hy3",
        _ => "gpt-5-mini",
    }
}

pub fn agent_prompt_text(output: &DirectorOutput) -> String {
    match output {
        DirectorOutput::FullAgent(o) => o.prompt.clone(),
        DirectorOutput::KeywordPs(o) => format!("{} | {:?}", o.user_intent, o.keywords),
    }
}

pub fn keywords_json(output: &DirectorOutput) -> Option<serde_json::Value> {
    match output {
        DirectorOutput::KeywordPs(o) => Some(serde_json::to_value(&o.keywords).ok()?),
        _ => None,
    }
}
