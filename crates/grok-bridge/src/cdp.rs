//! CDP 客户端：tokio-tungstenite 直连 Chrome/Edge remote-debugging。
//!
//! 封装 JSON-RPC（`{id, method, params}` → `{id, result|error}`），并提供与
//! Python bridge 操作等价的原语：navigate / evaluate（awaitPromise）/ set_cookie /
//! set_user_agent / add_script / get_cookies。
//!
//! 测试注入：[`CdpClient`] trait + [`FakeCdpClient`]（预置结果，不发真实 WS）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Mutex as StdMutex;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::BridgeError;

/// CDP 原语抽象（可注入 fake 供端点逻辑单测）。
#[async_trait::async_trait]
pub trait CdpClient: Send + Sync {
    /// 导航到 URL。
    async fn navigate(&self, url: &str) -> Result<(), BridgeError>;
    /// 执行 JS 表达式；`await_promise=true` 时等待返回的 Promise 决议（returnByValue）。
    async fn evaluate(&self, expression: &str, await_promise: bool) -> Result<Value, BridgeError>;
    /// 设置 cookie（Network.setCookie；domain 形如 `.grok.com`）。
    async fn set_cookie(&self, name: &str, value: &str, domain: &str) -> Result<(), BridgeError>;
    /// 覆盖 User-Agent（Network.setUserAgentOverride）。
    async fn set_user_agent(&self, user_agent: &str) -> Result<(), BridgeError>;
    /// 注入 document 创建时执行的脚本（Page.addScriptToEvaluateOnNewDocument）。
    async fn add_script(&self, source: &str) -> Result<(), BridgeError>;
    /// 取当前页全部 cookie（Network.getAllCookies）。
    async fn get_cookies(&self) -> Result<Vec<CookieValue>, BridgeError>;
}

/// CDP cookie 记录。
#[derive(Debug, Clone, PartialEq)]
pub struct CookieValue {
    pub name: String,
    pub value: String,
}

/// 单条 JSON-RPC 超时。
const CDP_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 真实 Chrome CDP 客户端：单 WS 连接，接收循环投递响应，发送侧 Mutex 串行化。
pub struct ChromeCdpClient {
    next_id: AtomicU64,
    sender: Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
}

impl ChromeCdpClient {
    /// 从已建立的 WS 流构造客户端（由 [`crate::chrome::launch`] 调用）。
    pub fn new(stream: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, mut source) = stream.split();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_read = Arc::clone(&pending);
        // 接收循环：id 匹配的响应投递到对应 oneshot；事件消息（无 id）忽略。
        tokio::spawn(async move {
            while let Some(Ok(msg)) = source.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                let tx = pending_read.lock().await.remove(&id);
                if let Some(tx) = tx {
                    let _ = tx.send(value);
                }
            }
        });
        Self {
            next_id: AtomicU64::new(1),
            sender: Mutex::new(sink),
            pending,
        }
    }

    /// 发送 JSON-RPC 并等待匹配 id 的响应。
    async fn call(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let message =
            serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
                .map_err(|e| BridgeError::internal(format!("serialize cdp call: {e}")))?;
        self.sender
            .lock()
            .await
            .send(Message::Text(message.into()))
            .await
            .map_err(|e| BridgeError::upstream(format!("cdp send {method}: {e}")))?;
        tokio::time::timeout(CDP_CALL_TIMEOUT, rx)
            .await
            .map_err(|_| BridgeError::upstream(format!("cdp {method} timeout")))?
            .map_err(|_| BridgeError::upstream(format!("cdp {method} channel closed")))?
            .pipe_result()
    }
}

/// 响应解析：`{result}` 取 result；`{error}` 转上游错误。
trait PipeResult {
    fn pipe_result(self) -> Result<Value, BridgeError>;
}

impl PipeResult for Value {
    fn pipe_result(self) -> Result<Value, BridgeError> {
        if let Some(err) = self.get("error") {
            return Err(BridgeError::upstream(format!(
                "cdp error: {}",
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            )));
        }
        Ok(self.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[async_trait::async_trait]
impl CdpClient for ChromeCdpClient {
    async fn navigate(&self, url: &str) -> Result<(), BridgeError> {
        self.call("Page.navigate", json!({ "url": url }))
            .await
            .map(|_| ())
    }

    async fn evaluate(&self, expression: &str, await_promise: bool) -> Result<Value, BridgeError> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": await_promise,
                    "returnByValue": true,
                }),
            )
            .await?;
        // `result.value` 为 returnByValue 的求值结果；异常时 exceptionDetails 存在。
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(BridgeError::upstream(format!(
                "js exception: {}",
                exception
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            )));
        }
        Ok(result.get("value").cloned().unwrap_or(Value::Null))
    }

    async fn set_cookie(&self, name: &str, value: &str, domain: &str) -> Result<(), BridgeError> {
        self.call(
            "Network.setCookie",
            json!({
                "name": name,
                "value": value,
                "domain": domain,
                "path": "/",
                "secure": true,
            }),
        )
        .await
        .map(|_| ())
    }

    async fn set_user_agent(&self, user_agent: &str) -> Result<(), BridgeError> {
        self.call(
            "Network.setUserAgentOverride",
            json!({ "userAgent": user_agent }),
        )
        .await
        .map(|_| ())
    }

    async fn add_script(&self, source: &str) -> Result<(), BridgeError> {
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source }),
        )
        .await
        .map(|_| ())
    }

    async fn get_cookies(&self) -> Result<Vec<CookieValue>, BridgeError> {
        let result = self.call("Network.getAllCookies", json!({})).await?;
        let cookies = result
            .get("cookies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(cookies
            .iter()
            .filter_map(|c| {
                Some(CookieValue {
                    name: c.get("name")?.as_str()?.to_string(),
                    value: c.get("value")?.as_str()?.to_string(),
                })
            })
            .collect())
    }
}

// ── 测试 fake ────────────────────────────────────────────────────

/// 预置结果的内存 fake：按表达式精确匹配或 `fallback` 返回。
pub struct FakeCdpClient {
    /// expression → 返回 Value。
    pub eval: std::sync::Mutex<HashMap<String, Value>>,
    /// 未命中 eval 时的兜底值（默认 Null）。
    pub fallback: StdMutex<Value>,
    /// navigate 调用记录。
    pub navigations: std::sync::Mutex<Vec<String>>,
    /// get_cookies 返回。
    pub cookies: std::sync::Mutex<Vec<CookieValue>>,
    /// set_cookie 调用记录（name,value,domain）。
    pub set_cookies: std::sync::Mutex<Vec<(String, String, String)>>,
}

impl FakeCdpClient {
    pub fn new() -> Self {
        Self {
            eval: StdMutex::new(HashMap::new()),
            fallback: StdMutex::new(Value::Null),
            navigations: StdMutex::new(Vec::new()),
            cookies: StdMutex::new(Vec::new()),
            set_cookies: StdMutex::new(Vec::new()),
        }
    }

    /// 预置某表达式的返回值。
    pub fn stub(&self, expression: &str, value: Value) {
        self.eval
            .lock()
            .unwrap()
            .insert(expression.to_string(), value);
    }
}

impl Default for FakeCdpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CdpClient for FakeCdpClient {
    async fn navigate(&self, url: &str) -> Result<(), BridgeError> {
        self.navigations.lock().unwrap().push(url.to_string());
        Ok(())
    }

    async fn evaluate(&self, expression: &str, _await_promise: bool) -> Result<Value, BridgeError> {
        let hit = self.eval.lock().unwrap().get(expression).cloned();
        Ok(hit.unwrap_or_else(|| self.fallback.lock().unwrap().clone()))
    }

    async fn set_cookie(&self, name: &str, value: &str, domain: &str) -> Result<(), BridgeError> {
        self.set_cookies.lock().unwrap().push((
            name.to_string(),
            value.to_string(),
            domain.to_string(),
        ));
        Ok(())
    }

    async fn set_user_agent(&self, _user_agent: &str) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn add_script(&self, _source: &str) -> Result<(), BridgeError> {
        Ok(())
    }

    async fn get_cookies(&self) -> Result<Vec<CookieValue>, BridgeError> {
        Ok(self.cookies.lock().unwrap().clone())
    }
}
