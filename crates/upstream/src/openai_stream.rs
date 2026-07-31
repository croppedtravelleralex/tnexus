//! Map upstream ChatGPT SSE into OpenAI `chat.completion.chunk` events.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::Error;
use bytes::Bytes;
use futures_util::Stream;
use http_body_util::BodyExt;
use serde_json::json;
use uuid::Uuid;

use crate::sse::{split_sse_data_lines, SseParser};

fn chrono_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_delta_chunk(chunk_id: &str, model: &str, delta: &str) -> Bytes {
    let payload = json!({
        "id": chunk_id,
        "object": "chat.completion.chunk",
        "created": chrono_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "content": delta },
            "finish_reason": null
        }]
    });
    Bytes::from(format!("data: {payload}\n\n"))
}

fn format_finish_chunk(chunk_id: &str, model: &str) -> Bytes {
    let payload = json!({
        "id": chunk_id,
        "object": "chat.completion.chunk",
        "created": chrono_secs(),
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    });
    Bytes::from(format!("data: {payload}\n\n"))
}

/// Stream OpenAI-compatible SSE chunks from an upstream conversation response.
pub struct OpenAiSseStream {
    body: Pin<Box<dyn Stream<Item = std::result::Result<Bytes, wreq::Error>> + Send>>,
    parser: SseParser,
    pending: Vec<u8>,
    model: String,
    chunk_id: String,
    out: VecDeque<Bytes>,
    body_done: bool,
    finished: bool,
}

impl OpenAiSseStream {
    pub fn from_upstream_sse(resp: wreq::Response, model: String) -> Self {
        Self {
            body: Box::pin(resp.into_data_stream()),
            parser: SseParser::new(),
            pending: Vec::new(),
            model,
            chunk_id: format!("chatcmpl-{}", Uuid::new_v4()),
            out: VecDeque::new(),
            body_done: false,
            finished: false,
        }
    }

    fn enqueue_finish(&mut self) {
        if self.finished {
            return;
        }
        self.out
            .push_back(format_finish_chunk(&self.chunk_id, &self.model));
        self.out
            .push_back(Bytes::from_static(b"data: [DONE]\n\n"));
        self.finished = true;
    }

    fn feed_payload(&mut self, payload: &str) {
        let Some(event) = self.parser.feed_line(payload) else {
            return;
        };
        if event.event_type == "conversation.delta" && !event.delta.is_empty() {
            self.out
                .push_back(format_delta_chunk(&self.chunk_id, &self.model, &event.delta));
        }
        if event.done {
            self.body_done = true;
        }
    }
}

impl Stream for OpenAiSseStream {
    type Item = std::result::Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(chunk) = self.out.pop_front() {
                return Poll::Ready(Some(Ok(chunk)));
            }
            if self.finished {
                return Poll::Ready(None);
            }
            if self.body_done {
                self.enqueue_finish();
                continue;
            }

            match self.body.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    for payload in split_sse_data_lines(&bytes, &mut self.pending) {
                        self.feed_payload(&payload);
                        if self.body_done {
                            break;
                        }
                    }
                }
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err.into()))),
                Poll::Ready(None) => {
                    self.body_done = true;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_chunk_is_openai_compatible() {
        let chunk = format_delta_chunk("chatcmpl-test", "gpt-4o-mini", "hi");
        let text = String::from_utf8_lossy(&chunk);
        assert!(text.starts_with("data: "));
        assert!(text.ends_with("\n\n"));
        let payload = text.trim_start_matches("data: ").trim();
        let v: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(v["object"], "chat.completion.chunk");
        assert_eq!(v["choices"][0]["delta"]["content"], "hi");
    }
}
