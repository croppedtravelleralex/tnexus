use anyhow::{bail, Context, Result};
use image::GenericImageView;
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use crate::conversation::ImageReference;
use crate::requirements::{RequirementsClient, BASE_URL};

#[derive(Debug, Clone)]
pub struct UploadedImage {
    pub file_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize)]
struct FileCreateResponse {
    file_id: String,
    upload_url: String,
}

fn mime_for_format(format: image::ImageFormat) -> &'static str {
    match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        image::ImageFormat::Gif => "image/gif",
        _ => "image/png",
    }
}

fn image_meta(data: &[u8], file_name: &str) -> Result<(u32, u32, String)> {
    let format = image::guess_format(data).context("guess image format")?;
    let img = image::load_from_memory(data).context("decode image for upload")?;
    let (width, height) = img.dimensions();
    let mime_type = mime_for_format(format).to_string();
    if width == 0 || height == 0 {
        bail!("invalid image dimensions for {file_name}");
    }
    Ok((width, height, mime_type))
}

fn resource_put_headers(mime_type: &str) -> Vec<(String, String)> {
    vec![
        ("Content-Type".into(), mime_type.into()),
        ("x-ms-blob-type".into(), "BlockBlob".into()),
        ("x-ms-version".into(), "2020-04-08".into()),
        ("Origin".into(), BASE_URL.into()),
        ("Referer".into(), format!("{BASE_URL}/")),
        ("Accept".into(), "application/json, text/plain, */*".into()),
    ]
}

/// Upload raw image bytes for multimodal image edit (`openai_backend_api._upload_image_once`).
pub async fn upload_image_bytes(
    client: &RequirementsClient,
    data: &[u8],
    file_name: &str,
) -> Result<UploadedImage> {
    let (width, height, mime_type) = image_meta(data, file_name)?;
    let path = "/backend-api/files";
    let body = json!({
        "file_name": file_name,
        "file_size": data.len(),
        "use_case": "multimodal",
        "width": width,
        "height": height,
    });
    let headers = client.api_headers(path);
    let mut api_headers = headers;
    api_headers.push(("Content-Type".into(), "application/json".into()));
    api_headers.push(("Accept".into(), "application/json".into()));
    let resp = RequirementsClient::apply_headers(
        client
            .client()
            .post(format!("{BASE_URL}{path}"))
            .body(serde_json::to_string(&body)?),
        &api_headers,
    )
    .send()
    .await
    .context("files create")?;
    let status = resp.status();
    let text = resp.text().await.context("files create body")?;
    if !status.is_success() {
        bail!(
            "files create HTTP {status}: {}",
            &text[..text.len().min(240)]
        );
    }
    let meta: FileCreateResponse =
        serde_json::from_str(&text).context("parse files create response")?;
    if meta.file_id.trim().is_empty() || meta.upload_url.trim().is_empty() {
        bail!("files create missing file_id or upload_url");
    }

    let put_headers = resource_put_headers(&mime_type);
    let put_resp = RequirementsClient::apply_headers(
        client
            .client()
            .put(meta.upload_url.clone())
            .body(data.to_vec()),
        &put_headers,
    )
    .send()
    .await
    .context("resource put")?;
    let put_status = put_resp.status();
    if !put_status.is_success() {
        let put_text = put_resp.text().await.unwrap_or_default();
        bail!(
            "resource put HTTP {put_status}: {}",
            &put_text[..put_text.len().min(240)]
        );
    }

    let uploaded_path = format!("/backend-api/files/{}/uploaded", meta.file_id);
    let uploaded_headers = client.api_headers(&uploaded_path);
    let mut finalize_headers = uploaded_headers;
    finalize_headers.push(("Content-Type".into(), "application/json".into()));
    finalize_headers.push(("Accept".into(), "application/json".into()));
    let fin_resp = RequirementsClient::apply_headers(
        client
            .client()
            .post(format!("{BASE_URL}{uploaded_path}"))
            .body("{}"),
        &finalize_headers,
    )
    .send()
    .await
    .context("files uploaded")?;
    let fin_status = fin_resp.status();
    if !fin_status.is_success() {
        let fin_text = fin_resp.text().await.unwrap_or_default();
        bail!(
            "files uploaded HTTP {fin_status}: {}",
            &fin_text[..fin_text.len().min(240)]
        );
    }

    info!(
        file_id = %meta.file_id,
        width,
        height,
        bytes = data.len(),
        "image uploaded for edit"
    );

    Ok(UploadedImage {
        file_id: meta.file_id,
        file_name: file_name.to_string(),
        file_size: data.len() as u64,
        mime_type,
        width,
        height,
    })
}

pub fn uploaded_to_reference(uploaded: &UploadedImage) -> ImageReference {
    ImageReference {
        file_id: uploaded.file_id.clone(),
        width: uploaded.width,
        height: uploaded.height,
        file_size: uploaded.file_size,
        mime_type: uploaded.mime_type.clone(),
        file_name: uploaded.file_name.clone(),
    }
}

pub fn decode_image_payload(raw: &str) -> Result<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("image payload is empty");
    }
    let b64 = if let Some(rest) = trimmed.strip_prefix("data:") {
        rest.split_once(',').map(|(_, data)| data).unwrap_or(rest)
    } else {
        trimmed
    };
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::STANDARD_NO_PAD, b64))
        .context("decode image base64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_data_url_base64() {
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        let data_url = format!("data:image/png;base64,{b64}");
        let decoded = decode_image_payload(&data_url).expect("decode");
        assert_eq!(decoded, png);
    }
}
