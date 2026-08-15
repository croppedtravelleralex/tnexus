//! Gateway adapter for the Rust upstream data plane.

use anyhow::Result;
use helper_client::PinAccount as HelperPinAccount;
use upstream::conversation::ImageReference;
use upstream::{OpenAiSseStream, PinAccount as UpstreamPinAccount, UpstreamRuntime};

fn to_upstream(account: &HelperPinAccount) -> UpstreamPinAccount {
    UpstreamPinAccount {
        email: account.email.clone(),
        access_token: account.access_token.clone(),
        device_id: account.device_id.clone().unwrap_or_default(),
        proxy: account.proxy.clone().unwrap_or_default(),
        user_agent: account.user_agent.clone().unwrap_or_default(),
        impersonate: account
            .impersonate
            .clone()
            .unwrap_or_default(),
    }
}

pub fn file_ids_to_references(file_ids: &[String]) -> Vec<ImageReference> {
    file_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(|id| ImageReference {
            file_id: id.to_string(),
            width: 1024,
            height: 1024,
            file_size: 0,
            mime_type: "image/png".into(),
            file_name: "reference.png".into(),
        })
        .collect()
}

pub async fn run_text(account: &HelperPinAccount, prompt: String, model: String) -> Result<String> {
    let mut runtime = UpstreamRuntime::new(to_upstream(account))?;
    runtime.run_text(&prompt, &model).await
}

pub async fn run_text_stream(
    account: &HelperPinAccount,
    prompt: String,
    model: String,
) -> Result<OpenAiSseStream> {
    let mut runtime = UpstreamRuntime::new(to_upstream(account))?;
    let resp = runtime.start_text_stream(&prompt, &model).await?;
    Ok(OpenAiSseStream::from_upstream_sse(resp, model))
}

pub async fn run_image(
    account: &HelperPinAccount,
    prompt: String,
    model: String,
    asset_ids: &[String],
) -> Result<(Vec<u8>, upstream::ImageRunMetrics)> {
    let refs = file_ids_to_references(asset_ids);
    let mut runtime = UpstreamRuntime::new(to_upstream(account))?;
    runtime
        .run_image_with_references(&prompt, &model, &refs)
        .await
}

pub async fn run_image_edit(
    account: &HelperPinAccount,
    prompt: String,
    model: String,
    image_bytes: Vec<u8>,
    file_name: String,
    mask_bytes: Option<Vec<u8>>,
    extra_asset_ids: &[String],
) -> Result<(Vec<u8>, upstream::ImageRunMetrics)> {
    let mut runtime = UpstreamRuntime::new(to_upstream(account))?;
    let mask_ref = mask_bytes.as_deref();
    runtime
        .run_image_edit_with_metrics_and_assets(
            &prompt,
            &model,
            &image_bytes,
            &file_name,
            mask_ref,
            extra_asset_ids,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_account_mapping_fills_defaults() {
        let helper = HelperPinAccount {
            email: "a@b.c".into(),
            access_token: "tok".into(),
            device_id: None,
            proxy: None,
            user_agent: None,
            impersonate: Some("chrome124".into()),
        };
        let up = to_upstream(&helper);
        assert_eq!(up.email, "a@b.c");
        assert_eq!(up.access_token, "tok");
        assert!(up.device_id.is_empty());
        assert!(up.proxy.is_empty());
        assert_eq!(up.impersonate, "chrome124");
    }

    #[test]
    fn file_ids_skip_empty() {
        let refs = file_ids_to_references(&["file-abc".into(), "".into(), "  ".into()]);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].file_id, "file-abc");
    }
}
