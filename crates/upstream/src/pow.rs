use anyhow::{bail, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{Datelike, Local, Timelike};
use rand::prelude::*;
use regex::Regex;
use serde_json::{json, Value};
use sha3::{Digest, Sha3_512};
use uuid::Uuid;

pub const DEFAULT_POW_SCRIPT: &str = "https://chatgpt.com/backend-api/sentinel/sdk.js";

static CORES: [i64; 4] = [8, 16, 24, 32];
static DOCUMENT_KEYS: [&str; 3] = [
    "__reactContainer$fzelfjyxej8",
    "_reactListening5dehydibo78",
    "location",
];
static SCREEN_RESOLUTIONS: [[i64; 2]; 4] = [[1920, 1080], [1440, 900], [2560, 1440], [3840, 2160]];

static NAVIGATOR_KEYS: [&str; 34] = [
    "registerProtocolHandler−function registerProtocolHandler() { [native code] }",
    "storage−[object StorageManager]",
    "locks−[object LockManager]",
    "appCodeName−Mozilla",
    "permissions−[object Permissions]",
    "share−function share() { [native code] }",
    "webdriver−false",
    "managed−[object NavigatorManagedData]",
    "canShare−function canShare() { [native code] }",
    "vendor−Google Inc.",
    "mediaDevices−[object MediaDevices]",
    "vibrate−function vibrate() { [native code] }",
    "storageBuckets−[object StorageBucketManager]",
    "mediaCapabilities−[object MediaCapabilities]",
    "cookieEnabled−true",
    "virtualKeyboard−[object VirtualKeyboard]",
    "product−Gecko",
    "presentation−[object Presentation]",
    "onLine−true",
    "mimeTypes−[object MimeTypeArray]",
    "credentials−[object CredentialsContainer]",
    "serviceWorker−[object ServiceWorkerContainer]",
    "keyboard−[object Keyboard]",
    "gpu−[object GPU]",
    "doNotTrack",
    "serial−[object Serial]",
    "pdfViewerEnabled−true",
    "language−zh-CN",
    "geolocation−[object Geolocation]",
    "userAgentData−[object NavigatorUAData]",
    "getUserMedia−function getUserMedia() { [native code] }",
    "sendBeacon−function sendBeacon() { [native code] }",
    "hardwareConcurrency−32",
    "windowControlsOverlay−[object WindowControlsOverlay]",
];

static WINDOW_KEYS: [&str; 43] = [
    "0",
    "window",
    "self",
    "document",
    "name",
    "location",
    "customElements",
    "history",
    "navigation",
    "innerWidth",
    "innerHeight",
    "scrollX",
    "scrollY",
    "visualViewport",
    "screenX",
    "screenY",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "screen",
    "chrome",
    "navigator",
    "onresize",
    "performance",
    "crypto",
    "indexedDB",
    "sessionStorage",
    "localStorage",
    "scheduler",
    "alert",
    "atob",
    "btoa",
    "fetch",
    "matchMedia",
    "postMessage",
    "queueMicrotask",
    "requestAnimationFrame",
    "setInterval",
    "setTimeout",
    "caches",
    "__NEXT_DATA__",
    "__BUILD_MANIFEST",
    "__NEXT_PRELOADREADY",
];

/// Parse PoW script sources + data-build from chatgpt.com homepage HTML.
pub fn parse_pow_resources(html_content: &str) -> (Vec<String>, String) {
    let script_re = Regex::new(r#"(?i)<script[^>]*\ssrc=["']([^"']+)["']"#).unwrap();
    let mut script_sources = Vec::new();
    let mut data_build = String::new();
    let build_re = Regex::new(r"c/[^/]*/_").unwrap();
    for cap in script_re.captures_iter(html_content) {
        let src = cap[1].to_string();
        if build_re.is_match(&src) {
            data_build = build_re
                .find(&src)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
        script_sources.push(src);
    }
    if script_sources.is_empty() {
        script_sources.push(DEFAULT_POW_SCRIPT.to_string());
    }
    if data_build.is_empty() {
        let html_build = Regex::new(r#"(?i)<html[^>]*data-build=["']([^"']*)["']"#).unwrap();
        if let Some(cap) = html_build.captures(html_content) {
            data_build = cap[1].to_string();
        }
    }
    (script_sources, data_build)
}

fn legacy_parse_time() -> String {
    let now = Local::now();
    format!(
        "{} {} {:02} {} {:02}:{:02}:{:02} GMT-0500 (Eastern Standard Time)",
        now.format("%a"),
        now.format("%b"),
        now.day(),
        now.year(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

/// Build PoW config array (same shape as `utils/pow.py::build_pow_config`).
pub fn build_pow_config(
    user_agent: &str,
    script_sources: Option<&[String]>,
    data_build: &str,
    rng: &mut impl Rng,
) -> Vec<Value> {
    let script_source = script_sources
        .and_then(|s| s.choose(rng))
        .cloned()
        .unwrap_or_else(|| DEFAULT_POW_SCRIPT.to_string());
    let screen = SCREEN_RESOLUTIONS.choose(rng).copied().unwrap();
    let screen_sum = screen[0] + screen[1];
    let perf = std::time::Instant::now().elapsed().as_secs_f64() * 1000.0;
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    vec![
        json!(screen_sum),
        json!(legacy_parse_time()),
        json!(4_294_705_152_i64),
        json!(1),
        json!(user_agent),
        json!(script_source),
        json!(data_build),
        json!("en-US"),
        json!("en-US,es-US,en,es"),
        json!(rng.random::<f64>()),
        json!(NAVIGATOR_KEYS.choose(rng).unwrap()),
        json!(DOCUMENT_KEYS.choose(rng).unwrap()),
        json!(WINDOW_KEYS.choose(rng).unwrap()),
        json!(perf),
        json!(Uuid::new_v4().to_string()),
        json!(""),
        json!(CORES.choose(rng).copied().unwrap()),
        json!(wall - perf),
        json!(0),
        json!(0),
        json!(0),
        json!(0),
        json!(0),
        json!(0),
        json!(0),
    ]
}

/// Serialize JSON like Python `json.dumps(..., separators=(",", ":"), ensure_ascii=False)`.
pub fn python_compact_json(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => escape_python_string(s),
        Value::Array(items) => {
            let inner = items.iter().map(python_compact_json).collect::<Vec<_>>();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (k, v) in map {
                pairs.push(format!(
                    "{}:{}",
                    escape_python_string(k),
                    python_compact_json(v)
                ));
            }
            format!("{{{}}}", pairs.join(","))
        }
    }
}

fn escape_python_string(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn pow_generate(seed: &str, difficulty: &str, config: &[Value], limit: usize) -> (String, bool) {
    let target = hex::decode(difficulty).unwrap_or_default();
    let diff_len = difficulty.len() / 2;
    let static_1 = format!(
        "{},",
        python_compact_json(&Value::Array(config[..3].to_vec())).trim_end_matches(']')
    );
    let mid = python_compact_json(&Value::Array(config[4..9].to_vec()));
    let static_2 = format!(",{},", &mid[1..mid.len() - 1]);
    let tail = python_compact_json(&Value::Array(config[10..].to_vec()));
    let static_3 = format!(",{}", &tail[1..]);

    let seed_bytes = seed.as_bytes();
    for i in 0..limit {
        let final_json = format!("{static_1}{i}{static_2}{}{static_3}", i >> 1);
        let encoded = B64.encode(final_json.as_bytes());
        let mut hasher = Sha3_512::new();
        hasher.update(seed_bytes);
        hasher.update(encoded.as_bytes());
        let digest = hasher.finalize();
        if digest[..diff_len] <= target[..] {
            return (encoded, true);
        }
    }
    let fallback = format!(
        "wQ8Lk5FbGpA2NcR9dShT6gYjU7VxZ4D{}",
        B64.encode(format!("\"{seed}\"").as_bytes())
    );
    (fallback, false)
}

pub fn build_legacy_requirements_token(
    user_agent: &str,
    script_sources: Option<&[String]>,
    data_build: &str,
    rng: &mut impl Rng,
) -> String {
    let config = build_pow_config(user_agent, script_sources, data_build, rng);
    let body = python_compact_json(&Value::Array(config));
    format!("gAAAAAC{}", B64.encode(body.as_bytes()))
}

pub fn build_proof_token(
    seed: &str,
    difficulty: &str,
    user_agent: &str,
    script_sources: Option<&[String]>,
    data_build: &str,
    rng: &mut impl Rng,
) -> Result<String> {
    let config = build_pow_config(user_agent, script_sources, data_build, rng);
    let (answer, solved) = pow_generate(seed, difficulty, &config, 500_000);
    if !solved {
        bail!("failed to solve proof token: difficulty={difficulty}");
    }
    Ok(format!("gAAAAAB{answer}"))
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    use super::*;

    #[test]
    fn parse_pow_resources_finds_scripts() {
        let html =
            r#"<html data-build="prod-abc"><script src="/cdn/c/xyz/_/app.js"></script></html>"#;
        let (sources, build) = parse_pow_resources(html);
        assert!(!sources.is_empty());
        assert_eq!(build, "c/xyz/_");
    }

    #[test]
    fn proof_token_low_difficulty() {
        let mut rng = StdRng::seed_from_u64(42);
        let token = build_proof_token(
            "seed",
            "00",
            "Mozilla/5.0",
            Some(&[DEFAULT_POW_SCRIPT.to_string()]),
            "c/test/_",
            &mut rng,
        )
        .expect("solve easy pow");
        assert!(token.starts_with("gAAAAAB"));
    }
}
