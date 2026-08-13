//! Can a Rust HTTP client reach accounts.x.ai?
//!
//! The mint flow's browser-free path runs through accounts.x.ai, which refuses
//! curl's TLS fingerprint (403) while answering Python's stdlib client (200).
//! That makes the question empirical rather than a matter of "browser or not",
//! and it decides whether porting the flow needs an impersonating client.
//!
//!   cargo run --example tls_probe             # direct
//!   cargo run --example tls_probe -- <proxy>  # through a relay

use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";

fn verdict(body: &str) -> &'static str {
    let lowered = body.to_lowercase();
    for (needle, label) in [
        ("blocked due to abusive", "IP BLOCK"),
        ("just a moment", "CF CHALLENGE"),
        ("enable javascript and cookies", "CF CHALLENGE"),
        ("cf-challenge", "CF CHALLENGE"),
    ] {
        if lowered.contains(needle) {
            return label;
        }
    }
    if lowered.contains("sign-in") || lowered.contains("sign-up") {
        return "reached the app (signed out)";
    }
    "reached the app"
}

async fn probe(label: &str, client: reqwest::Client, url: &str) {
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            println!(
                "  {label:<22} HTTP {status}  {}  ({} bytes)",
                verdict(&body),
                body.len()
            );
        }
        Err(err) => println!("  {label:<22} ERROR {err}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proxy = std::env::args().nth(1);
    println!(
        "egress: {}",
        proxy.as_deref().unwrap_or("direct from this machine")
    );

    let build = |accept_browser_headers: bool| -> reqwest::Result<reqwest::Client> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .user_agent(UA);
        if accept_browser_headers {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                    .parse()
                    .unwrap(),
            );
            headers.insert("accept-language", "en-US,en;q=0.9".parse().unwrap());
            builder = builder.default_headers(headers);
        }
        if let Some(proxy) = &proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy)?);
        }
        builder.build()
    };

    probe("reqwest + headers", build(true)?, "https://accounts.x.ai/").await;
    probe("reqwest bare", build(false)?, "https://accounts.x.ai/").await;

    // auth.x.ai is known to answer anything; it anchors the comparison so a
    // network fault is not mistaken for a block.
    probe(
        "auth.x.ai (control)",
        build(true)?,
        "https://auth.x.ai/.well-known/openid-configuration",
    )
    .await;

    // wreq drives BoringSSL and can present a real Chrome TLS/HTTP2 signature,
    // which is the only Rust option if the plain client is refused.
    for emulation in [
        wreq_util::Emulation::Chrome137,
        wreq_util::Emulation::Firefox136,
    ] {
        let mut builder = wreq::Client::builder().emulation(emulation);
        if let Some(proxy) = &proxy {
            builder = builder.proxy(wreq::Proxy::all(proxy)?);
        }
        let client = builder.build()?;
        match client.get("https://accounts.x.ai/").send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                println!(
                    "  {:<22} HTTP {status}  {}  ({} bytes)",
                    format!("wreq {emulation:?}"),
                    verdict(&body),
                    body.len()
                );
            }
            Err(err) => println!("  {:<22} ERROR {err}", format!("wreq {emulation:?}")),
        }
    }

    println!();
    println!("reqwest reaching the app -> plain reqwest is enough for the port.");
    println!("only wreq reaching it   -> the port needs the impersonating client.");
    Ok(())
}
