//! G5-P1 stored response 往返集成测试（迁移 Go `cli/adapter_test.go` 核心用例）。
//!
//! mock 方式：本地 `TcpListener` 充当上游，捕获请求头/体后返回 canned 响应
//! （对齐 Go 的 `roundTripFunc` 注入 Transport）。

use std::sync::{Arc, Mutex};

use reqwest::Method;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use grok_provider_build::{BuildAdapter, Config, ForwardRequest};

/// 捕获到的上游请求（头 + 体）。
#[derive(Debug, Default)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

/// 启动 mock 上游，返回基地址。每次请求捕获到 `captured`，并按 `respond` 回包。
async fn spawn_mock(
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    respond: fn(&CapturedRequest) -> (u16, &'static str),
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            tokio::spawn(async move {
                // 读请求头（直到空行）
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                loop {
                    if socket.read(&mut byte).await.unwrap() == 0 {
                        return;
                    }
                    buf.push(byte[0]);
                    if buf.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let mut lines = head.split("\r\n");
                let request_line = lines.next().unwrap_or_default();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let mut headers = Vec::new();
                let mut content_length = 0usize;
                for line in lines {
                    if let Some((k, v)) = line.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            content_length = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                // 读 body
                let mut body = Vec::new();
                if content_length > 0 {
                    body.resize(content_length, 0);
                    let mut read = 0;
                    while read < content_length {
                        let n = socket.read(&mut body[read..]).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        read += n;
                    }
                }
                captured.lock().unwrap().push(CapturedRequest {
                    method,
                    path,
                    headers,
                    body: String::from_utf8_lossy(&body).to_string(),
                });

                let (status, body) = respond(captured.lock().unwrap().last().unwrap());
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

fn test_config(base_url: String) -> Config {
    Config {
        base_url,
        client_version: "0.2.99".into(),
        client_identifier: "grok-shell".into(),
        token_auth: "xai-grok-cli".into(),
        user_agent: "grok-shell/0.2.99 (linux; x86_64)".into(),
    }
}

#[tokio::test]
async fn stored_response_round_trip_with_build_headers() {
    // Go TestForwardResponseMatchesGrokBuildHeadersAndPreservesReasoning + stored 往返
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(
        captured.clone(),
        |_| (200, r#"{"id":"resp_1","model":"grok-4.5","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"Reply with OK only."}]}]}"#),
    )
    .await;

    let adapter = BuildAdapter::new(test_config(base));
    let response = adapter
        .forward_stored(
            "grok-4.5",
            json!({"role": "user", "content": "hi"}),
            16,
            "access-token",
            "official-key",
        )
        .await
        .expect("stored round trip");

    assert_eq!(response.id, "resp_1");
    assert_eq!(response.model.as_deref(), Some("grok-4.5"));
    assert_eq!(response.text(), "Reply with OK only.");

    // 请求侧断言（对齐 Go 头部契约）
    let reqs = captured.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/responses");
    assert_eq!(
        req.header("Authorization").as_deref(),
        Some("Bearer access-token")
    );
    assert_eq!(
        req.header("x-grok-client-version").as_deref(),
        Some("0.2.99")
    );
    assert_eq!(
        req.header("x-grok-client-identifier").as_deref(),
        Some("grok-shell")
    );
    assert_eq!(req.header("x-grok-client-surface").as_deref(), Some("tui"));
    assert_eq!(
        req.header("x-grok-client-name").as_deref(),
        Some("grok-shell")
    );
    assert_eq!(
        req.header("User-Agent").as_deref(),
        Some("grok-shell/0.2.99 (linux; x86_64)")
    );
    assert_eq!(
        req.header("x-grok-conv-id").as_deref(),
        Some("official-key"),
        "prompt cache key → conv id"
    );
    assert_eq!(
        req.header("x-grok-conversation-id").as_deref(),
        Some("official-key")
    );
    assert_eq!(req.header("Accept").as_deref(), Some("application/json"));
    assert_eq!(req.header("Accept-Encoding").as_deref(), Some("gzip"));
    assert_eq!(
        req.header("x-grok-agent-id").as_deref().map(str::len),
        Some(32)
    );
    assert_eq!(
        req.header("x-grok-session-id").as_deref().map(str::len),
        Some(36)
    );
    assert_eq!(
        req.header("x-grok-req-id").as_deref().map(str::len),
        Some(32)
    );
    assert_eq!(
        req.header("x-grok-request-id").as_deref(),
        req.header("x-grok-req-id").as_deref()
    );
    assert_eq!(
        req.header("x-grok-session-id-legacy").as_deref(),
        req.header("x-grok-session-id").as_deref()
    );
    assert_eq!(req.header("traceparent").as_deref().map(str::len), Some(55));

    // 请求体：模型被覆盖为 grok-4.5，store/stream false
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["model"], "grok-4.5");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], false);
    assert_eq!(body["max_output_tokens"], 16);
}

#[tokio::test]
async fn forward_normalizes_model_and_preserves_reasoning_input() {
    // Go TestForwardResponseMatchesGrokBuildHeadersAndPreservesReasoning 的 body 断言
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(captured.clone(), |_| {
        (200, r#"{"id":"resp_1","object":"response"}"#)
    })
    .await;
    let adapter = BuildAdapter::new(test_config(base));

    let request = ForwardRequest {
        method: Method::POST,
        path: "/responses".into(),
        model: "grok-4.5".into(),
        access_token: "access-token".into(),
        user_id: Some("user-123".into()),
        prompt_cache_key: "official-key".into(),
        body: Some(json!({
            "model": "public",
            "prompt_cache_key": "official-key",
            "input": [{"type": "reasoning", "id": "rs_1", "encrypted_content": "cipher"}],
        })),
        streaming: false,
    };
    let response = adapter.forward(&request).await.expect("forward");
    assert!(response.is_success());

    let reqs = captured.lock().unwrap();
    let req = &reqs[0];
    assert_eq!(req.header("x-userid").as_deref(), Some("user-123"));
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["model"], "grok-4.5", "model overridden");
    assert_eq!(body["prompt_cache_key"], "official-key");
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(
        input[0]["encrypted_content"], "cipher",
        "reasoning replay preserved"
    );
}

#[tokio::test]
async fn forward_reports_upstream_non_success() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(captured.clone(), |_| {
        (429, r#"{"error":{"message":"rate limited"}}"#)
    })
    .await;
    let adapter = BuildAdapter::new(test_config(base));

    let request = ForwardRequest {
        method: Method::POST,
        path: "/responses".into(),
        model: "grok-4.5".into(),
        access_token: "token".into(),
        user_id: None,
        prompt_cache_key: String::new(),
        body: Some(json!({"input": "hello"})),
        streaming: false,
    };
    let response = adapter.forward(&request).await.expect("forward");
    assert!(!response.is_success());
    assert_eq!(response.status, 429);
    assert!(response.body.contains("rate limited"));
}

#[tokio::test]
async fn forward_supports_resource_methods_and_query() {
    // Go TestForwardResponseSupportsResourceMethodsAndQuery（GET/DELETE 资源方法）
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(captured.clone(), |_| (200, r#"{"id":"resp_1"}"#)).await;
    let adapter = BuildAdapter::new(test_config(base));

    for method in [Method::GET, Method::DELETE] {
        let request = ForwardRequest {
            method: method.clone(),
            path: "/responses/resp_1?include=reasoning.encrypted_content".into(),
            model: String::new(),
            access_token: "token".into(),
            user_id: None,
            prompt_cache_key: String::new(),
            body: None,
            streaming: false,
        };
        let response = adapter.forward(&request).await.expect("forward");
        assert!(response.is_success());
    }

    let reqs = captured.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(
        reqs[0].path,
        "/responses/resp_1?include=reasoning.encrypted_content"
    );
    assert_eq!(reqs[1].method, "DELETE");
    assert_eq!(
        reqs[1].path,
        "/responses/resp_1?include=reasoning.encrypted_content"
    );
    // 无 prompt_cache_key → 随机 32-hex conv id（对齐 Go `randomHex(16)`）
    assert_eq!(
        reqs[0].header("x-grok-conv-id").as_deref().map(str::len),
        Some(32)
    );
}
