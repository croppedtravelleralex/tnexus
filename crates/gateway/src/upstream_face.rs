//! Gateway adapter for the Rust upstream data plane.

use anyhow::Result;
use helper_client::PinAccount as HelperPinAccount;
use upstream::{OpenAiSseStream, PinAccount as UpstreamPinAccount, UpstreamRuntime};

fn to_upstream(account: &HelperPinAccount) -> UpstreamPinAccount {
    UpstreamPinAccount {
        email: account.email.clone(),
        access_token: account.access_token.clone(),
        device_id: account.device_id.clone().unwrap_or_default(),
        proxy: account.proxy.clone().unwrap_or_default(),
        user_agent: account.user_agent.clone().unwrap_or_default(),
        impersonate: String::new(),
    }
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
) -> Result<Vec<u8>> {
    let mut runtime = UpstreamRuntime::new(to_upstream(account))?;
    runtime.run_image(&prompt, &model).await
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
        };
        let up = to_upstream(&helper);
        assert_eq!(up.email, "a@b.c");
        assert_eq!(up.access_token, "tok");
        assert!(up.device_id.is_empty());
        assert!(up.proxy.is_empty());
    }
}
