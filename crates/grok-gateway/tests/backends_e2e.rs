//! G5-P3 真实后端 E2E：TcpListener mock 上游 → grok-provider-build / console
//! 适配器 → gateway /v1/responses 与 /v1/messages 全链路往返（无 fake）。
//!
//! - `/responses`（Build）：返回 stored response JSON，断言输出文本
//! - `/v1/chat/completions`（Console）：返回 SSE 流，断言拼接文本

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const BUILD_REPLY: &str = "mock-build-reply";
const CONSOLE_REPLY: &str = "mock-console-reply";

/// 单连接 mock：解析请求行路径 → 按路径返回固定响应。
fn handle_connection(mut stream: TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    // 读请求行 + 头（直到空行），捕获 Content-Length。
    let header_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
                    break pos + 4;
                }
            }
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let first_line = head.lines().next().unwrap_or_default().to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    // 读请求体（存在时）。
    while buf.len() < header_end + content_length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let path = first_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();

    let (status_line, body): (&str, String) = if path == "/responses" {
        (
            "HTTP/1.1 200 OK",
            json!({
                "id": "resp_mock",
                "model": "grok-4.5",
                "status": "completed",
                "output": [{ "type": "message", "content": [{ "type": "output_text", "text": BUILD_REPLY }] }],
            })
            .to_string(),
        )
    } else if path == "/v1/chat/completions" {
        // Console SSE：两条 delta + [DONE]。
        (
            "HTTP/1.1 200 OK",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"mock-\"}}]}\r\n\r\n\
             data: {\"choices\":[{\"delta\":{\"content\":\"console-reply\"}}]}\r\n\r\n\
             data: [DONE]\r\n\r\n"
            .to_string(),
        )
    } else {
        ("HTTP/1.1 404 Not Found", "not found".to_string())
    };

    let content_type = if path == "/v1/chat/completions" {
        "text/event-stream"
    } else {
        "application/json"
    };
    let _ = write!(
        stream,
        "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.flush();
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 启动 mock 上游，返回 base_url。线程处理完 `expected` 个连接后退出（可 join）。
fn spawn_mock(expected: usize) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut handled = 0usize;
        for stream in listener.incoming().take(expected) {
            match stream {
                Ok(stream) => {
                    handle_connection(stream);
                    handled += 1;
                }
                Err(_) => break,
            }
        }
        let _ = handled;
    });
    (format!("http://{addr}"), handle)
}

async fn post(app: &axum::Router, path: &str, body: Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// /v1/responses 经真实 Build adapter → mock /responses → stored response 文本。
#[tokio::test]
async fn responses_via_real_build_backend() {
    let (base, mock) = spawn_mock(1);
    let app = grok_gateway::build_app(grok_gateway::with_default_protocol_backends(
        Some(base.clone()),
        Some(base),
    ));
    let (status, body) = post(
        &app,
        "/v1/responses",
        json!({ "model": "grok-4.5", "input": [{
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "hello build"}],
        }] }),
    )
    .await;
    mock.join().unwrap();
    assert_eq!(status, StatusCode::OK, "body = {body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["object"], "response");
    assert_eq!(value["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(value["output"][0]["content"][0]["text"], BUILD_REPLY);
}

/// /v1/messages 经真实 Console adapter → mock SSE → 拼接文本。
#[tokio::test]
async fn messages_via_real_console_backend() {
    let (base, mock) = spawn_mock(1);
    let app = grok_gateway::build_app(grok_gateway::with_default_protocol_backends(
        Some(base.clone()),
        Some(base),
    ));
    let (status, body) = post(
        &app,
        "/v1/messages",
        json!({
            "model": "grok-4.5",
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "hi"}],
        }),
    )
    .await;
    mock.join().unwrap();
    assert_eq!(status, StatusCode::OK, "body = {body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["type"], "message");
    assert_eq!(value["content"][0]["type"], "text");
    assert_eq!(value["content"][0]["text"], CONSOLE_REPLY);
}

/// 独立注入：/v1/responses 用 Build 后端、/v1/messages 用 Console 后端，互不影响。
#[tokio::test]
async fn split_backends_responses_build_only() {
    let (base, mock) = spawn_mock(1); // 仅 /v1/responses 命中上游（messages 后端 None → 500 不请求）
                                      // 只注入 responses 后端 → messages 端点 500。
    let responses = Arc::new(grok_gateway::BuildResponsesBackend::new(
        Some(base.clone()),
        String::new(),
    ));
    let app = grok_gateway::build_app(grok_gateway::with_protocol_backends(Some(responses), None));
    let (status, body) = post(
        &app,
        "/v1/responses",
        json!({ "model": "grok-4.5", "input": "hi" }),
    )
    .await;
    mock.join().unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(BUILD_REPLY));

    let (status, body) = post(
        &app,
        "/v1/messages",
        json!({ "model": "grok-4.5", "messages": [{"role": "user", "content": "hi"}] }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("MessagesBackend"));
}

use std::sync::Arc;
