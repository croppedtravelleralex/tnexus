//! G5-A3 Console 流式往返集成测试（迁移 Go `console_test.go` 核心断言 + SSE 流）。
//!
//! mock 方式：本地 `TcpListener` 充当上游，捕获请求头/体后返回 SSE 块序列或错误 JSON
//! （对齐 grok-provider-build::tests::stored_response 的 mock 模式）。

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use grok_provider_console::{ChatDelta, Config, ConsoleAdapter, ProviderError};

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

/// 启动 mock 上游。`respond` 决定状态行/内容类型/响应体（支持 SSE 文本）。
async fn spawn_mock(
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    respond: fn(&CapturedRequest) -> (u16, &'static str, &'static str), // (status, content_type, body)
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let captured = captured.clone();
            tokio::spawn(async move {
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

                let (status, content_type, body) =
                    respond(captured.lock().unwrap().last().unwrap());
                let reason = if status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{addr}")
}

const SSE_BLOCKS: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n\
data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n";

#[tokio::test]
async fn streams_and_normalizes_sse_deltas() {
    // Go TestAdapterPreservesConversationRateLimitStatusAndProtocol 之外的流式核心：
    // 上游 SSE 块序列 → 归一化分片 [role, "Hel", "lo", stop]
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(captured.clone(), |_| (200, "text/event-stream", SSE_BLOCKS)).await;

    let adapter = ConsoleAdapter::new(Config {
        base_url: base,
        user_agent: "grok-console/0.1".into(),
        timeout: std::time::Duration::from_secs(10),
    });
    let deltas = adapter
        .forward_chat(
            "grok-4.3",
            &json!([{"role": "user", "content": "hello"}]),
            "test-sso",
        )
        .await
        .expect("stream ok");

    let expect: Vec<ChatDelta> = vec![
        ChatDelta { role: Some("assistant".into()), ..Default::default() },
        ChatDelta { content: Some("Hel".into()), ..Default::default() },
        ChatDelta { content: Some("lo".into()), ..Default::default() },
        ChatDelta { finish_reason: Some("stop".into()), ..Default::default() },
    ];
    assert_eq!(deltas, expect, "[DONE] 不产出分片");

    // 请求侧断言（对齐 Go applyHeaders + chat 请求形态）
    let reqs = captured.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/v1/chat/completions");
    assert_eq!(req.header("Authorization").as_deref(), Some("Bearer anonymous"));
    assert_eq!(req.header("Accept").as_deref(), Some("text/event-stream"));
    assert_eq!(req.header("Content-Type").as_deref(), Some("application/json"));
    let cookie = req.header("Cookie").unwrap_or_default();
    assert!(cookie.contains("sso=test-sso"), "cookie = {cookie}");
    assert!(cookie.contains("sso-rw=test-sso"), "cookie = {cookie}");
    assert_eq!(req.header("x-cluster").as_deref(), Some("https://us-east-1.api.x.ai"));
    assert_eq!(req.header("Origin").as_deref(), Some("https://console.x.ai"));
    let body: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(body["model"], "grok-4.3");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["content"], "hello");
}

#[tokio::test]
async fn parses_error_json_with_type() {
    // Go TestAdapterPreservesConversationRateLimitStatusAndProtocol：429 + 错误信封
    let captured = Arc::new(Mutex::new(Vec::new()));
    let base = spawn_mock(captured.clone(), |_| {
        (
            429,
            "application/json",
            r#"{"error":{"type":"rate_limit_error","message":"Rate limit reached. Resets in: 1h 2m 3s"}}"#,
        )
    })
    .await;
    let adapter = ConsoleAdapter::new(Config {
        base_url: base,
        user_agent: "grok-console/0.1".into(),
        timeout: std::time::Duration::from_secs(10),
    });
    let err = adapter
        .forward_chat("grok-4.3", &json!([]), "test-sso")
        .await
        .expect_err("rate limited");
    match err {
        ProviderError::Upstream(text) => {
            assert!(text.contains("429"), "message = {text}");
            assert!(text.contains("rate_limit_error"), "message = {text}");
            assert!(text.contains("Rate limit reached"), "message = {text}");
        }
        other => panic!("expected Upstream, got {other}"),
    }
}

#[tokio::test]
async fn reports_connection_error() {
    // 连接被拒 → Http（Go ForwardResponse 的 transport 错误分支）
    let adapter = ConsoleAdapter::new(Config {
        base_url: "http://127.0.0.1:1".into(),
        user_agent: "grok-console/0.1".into(),
        timeout: std::time::Duration::from_secs(5),
    });
    let err = adapter
        .forward_chat("grok-4.3", &json!([]), "test-sso")
        .await
        .expect_err("connection refused");
    assert!(
        matches!(err, ProviderError::Http(_)),
        "expected Http, got {err}"
    );
}

#[tokio::test]
async fn reports_timeout() {
    // 上游挂起超过 timeout → Timeout（Go context deadline 语义）
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // 读请求头后不回包（挂起）
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    });
    let adapter = ConsoleAdapter::new(Config {
        base_url: format!("http://{addr}"),
        user_agent: "grok-console/0.1".into(),
        timeout: std::time::Duration::from_millis(200),
    });
    let err = adapter
        .forward_chat("grok-4.3", &json!([]), "test-sso")
        .await
        .expect_err("timeout");
    assert!(
        matches!(err, ProviderError::Timeout(_)),
        "expected Timeout, got {err}"
    );
}
