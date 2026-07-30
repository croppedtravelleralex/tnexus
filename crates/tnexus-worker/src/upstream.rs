use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use tnexus_domain::agent::DirectorOutput;

#[derive(Clone)]
pub struct UpstreamClient {
    pub http: reqwest::Client,
    pub gptimage_base: String,
    pub grok2api_base: String,
    pub director_model: String,
    pub chatgpt_image_model: String,
    pub grok_image_model: String,
    pub api_key: Option<String>,
}

impl UpstreamClient {
    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            req.bearer_auth(key)
        } else {
            req
        }
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
}

#[derive(Debug, Deserialize)]
struct ImageDataItem {
    b64_json: Option<String>,
    url: Option<String>,
    revised_prompt: Option<String>,
}

pub struct ImageGenOptions {
    pub size: String,
    pub count: u32,
    pub quality: Option<String>,
    pub transparent_bg: bool,
}

pub struct GeneratedImage {
    pub bytes: Option<Vec<u8>>,
    pub source_url: Option<String>,
    pub revised_prompt: Option<String>,
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

    pub async fn generate_chatgpt(
        &self,
        prompt: &str,
        prompt_enhance: bool,
        opts: &ImageGenOptions,
    ) -> Result<Vec<GeneratedImage>> {
        self.generate_images(
            &self.gptimage_base,
            &self.chatgpt_image_model,
            prompt,
            prompt_enhance,
            "prompt_enhance",
            opts,
            "url",
        )
        .await
    }

    pub async fn generate_grok(
        &self,
        prompt: &str,
        prompt_enhance: bool,
        opts: &ImageGenOptions,
    ) -> Result<Vec<GeneratedImage>> {
        self.generate_images(
            &self.grok2api_base,
            &self.grok_image_model,
            prompt,
            prompt_enhance,
            "prompt_enhance",
            opts,
            "b64_json",
        )
        .await
    }

    async fn generate_images(
        &self,
        base: &str,
        model: &str,
        prompt: &str,
        prompt_enhance: bool,
        enhance_field: &str,
        opts: &ImageGenOptions,
        response_format: &str,
    ) -> Result<Vec<GeneratedImage>> {
        let mut body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "n": opts.count.max(1).min(10),
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
            return Err(anyhow::anyhow!("image generation HTTP {status}: {text}"));
        }
        let resp: ImageGenerationResponse =
            serde_json::from_str(&text).context("image generation json")?;
        let mut out = Vec::new();
        for item in resp.data {
            if let Some(url) = item
                .url
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                out.push(GeneratedImage {
                    bytes: None,
                    source_url: Some(url.to_string()),
                    revised_prompt: item.revised_prompt.clone(),
                });
                continue;
            }
            let bytes = if let Some(b64) = &item.b64_json {
                STANDARD.decode(b64).context("decode b64")?
            } else {
                continue;
            };
            out.push(GeneratedImage {
                bytes: Some(bytes),
                source_url: None,
                revised_prompt: item.revised_prompt.clone(),
            });
        }
        if out.is_empty() {
            return Err(anyhow::anyhow!("no image payload"));
        }
        Ok(out)
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
