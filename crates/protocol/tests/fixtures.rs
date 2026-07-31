//! Golden fixture diff tests for protocol shapes (Python-captured goldens).

use protocol::{
    assert_json_matches_except, build_estuary_download_headers, build_image_prepare_body_opts,
    build_image_start_body_opts, build_image_start_body_with_refs_opts,
    build_text_conversation_body_opts, validate_estuary_headers, validate_resource_put_headers,
    ContractOptions, ImageRef,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/protocol")
        .join(name)
}

fn load_json(name: &str) -> Value {
    let raw = fs::read_to_string(fixture_path(name)).expect("read fixture");
    serde_json::from_str(&raw).expect("parse fixture json")
}

fn fixture_opts() -> ContractOptions {
    let mut o = ContractOptions::fixture();
    o.fixed_message_id = Some("00000000-0000-4000-8000-000000000004".into());
    o
}

const VOLATILE: &[&str] = &[
    "messages[0].id",
    "partial_query.id",
    "messages[0].create_time",
];

#[test]
fn chat_body_matches_python_golden() {
    let mut opts = ContractOptions::fixture();
    opts.fixed_message_id = Some("00000000-0000-4000-8000-000000000001".into());
    let built = build_text_conversation_body_opts("hello fixture", "gpt-4o-mini", &opts);
    let golden = load_json("chat_body.json");
    assert_json_matches_except(&built, &golden, &["messages[0].id"]);
}

#[test]
fn image_prepare_matches_python_golden() {
    let mut opts = fixture_opts();
    opts.fixed_message_id = Some("00000000-0000-4000-8000-000000000002".into());
    let built = build_image_prepare_body_opts("sunset over ocean", "gpt-image-2", &opts);
    let golden = load_json("image_prepare_body.json");
    assert_json_matches_except(&built, &golden, &["partial_query.id"]);
}

#[test]
fn image_start_matches_python_golden() {
    let mut opts = fixture_opts();
    opts.fixed_message_id = Some("00000000-0000-4000-8000-000000000003".into());
    let built = build_image_start_body_opts("a red cube on white background", "gpt-image-2", &opts);
    let golden = load_json("image_start_body.json");
    assert_json_matches_except(&built, &golden, VOLATILE);
}

#[test]
fn image_start_with_refs_matches_python_golden() {
    let refs = [ImageRef {
        file_id: "file-fixture-001".into(),
        mime_type: "image/png".into(),
        file_name: "input.png".into(),
        file_size: 204800,
        width: 1024,
        height: 1024,
    }];
    let mut opts = fixture_opts();
    opts.fixed_message_id = Some("00000000-0000-4000-8000-000000000005".into());
    let built = build_image_start_body_with_refs_opts(
        "edit: make the sky sunset orange",
        "gpt-image-2",
        &refs,
        &opts,
    );
    let golden = load_json("image_start_body_with_refs.json");
    assert_json_matches_except(&built, &golden, VOLATILE);
}

#[test]
fn estuary_headers_require_bearer() {
    let golden = load_json("estuary_headers.json");
    let built = build_estuary_download_headers("REDACTED");
    assert!(validate_estuary_headers(&built).is_ok());
    assert!(golden["must_include"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k.as_str() == Some("Authorization")));
}

#[test]
fn sse_fixture_has_skipped_mainline() {
    let raw = fs::read_to_string(fixture_path("sse_skipped_mainline.ndjson")).unwrap();
    assert!(raw.contains("skipped_mainline"));
    assert!(raw.contains("conversation.done"));
}

#[test]
fn upload_fixture_forbids_bearer_on_resource() {
    let v = load_json("upload_api_vs_resource.json");
    let must_not: Vec<_> = v["resource_put"]["must_not_include"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(must_not.contains(&"Authorization"));
    let resource_headers = serde_json::json!({"Content-Type": "image/png"});
    assert!(validate_resource_put_headers(&resource_headers).is_ok());
    let bad = serde_json::json!({"authorization": "Bearer x"});
    assert!(validate_resource_put_headers(&bad).is_err());
}

#[test]
fn sentinel_headers_fixture_present() {
    let v = load_json("sentinel_headers.json");
    assert!(v.get("OpenAI-Sentinel-Chat-Requirements-Token").is_some());
}
