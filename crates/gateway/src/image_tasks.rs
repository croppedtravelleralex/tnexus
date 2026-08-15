//! Async image task queue — gptimage `image_task_service` subset.

use parking_lot::RwLock;
use protocol::ImageGenerationRequest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageTaskState {
    Queued,
    Running,
    TimeoutPending,
    Done,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageTaskRecord {
    pub id: String,
    pub state: ImageTaskState,
    pub request: ImageGenerationRequest,
    pub created_at: i64,
    pub updated_at: i64,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub email: Option<String>,
    pub trace: Option<Value>,
}

#[derive(Clone)]
pub struct ImageTaskStore {
    inner: Arc<RwLock<HashMap<String, ImageTaskRecord>>>,
    path: PathBuf,
}

impl ImageTaskStore {
    pub fn from_env() -> Self {
        let path = std::env::var("IMAGE_TASKS_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/pool/image_tasks.json"));
        let store = Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            path,
        };
        store.load_from_disk();
        store
    }

    fn load_from_disk(&self) {
        if !self.path.exists() {
            return;
        }
        if let Ok(raw) = fs::read_to_string(&self.path) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, ImageTaskRecord>>(&raw) {
                *self.inner.write() = map;
            }
        }
    }

    fn persist(&self) {
        let map = self.inner.read().clone();
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_string_pretty(&map) {
            let _ = fs::write(&self.path, raw);
        }
    }

    pub fn insert(&self, record: ImageTaskRecord) {
        self.inner.write().insert(record.id.clone(), record);
        self.persist();
    }

    pub fn get(&self, id: &str) -> Option<ImageTaskRecord> {
        self.inner.read().get(id).cloned()
    }

    pub fn update_state(
        &self,
        id: &str,
        state: ImageTaskState,
        result: Option<Value>,
        error: Option<String>,
        email: Option<String>,
        trace: Option<Value>,
    ) {
        let mut map = self.inner.write();
        if let Some(rec) = map.get_mut(id) {
            rec.state = state;
            rec.updated_at = protocol::chrono_secs() as i64;
            if result.is_some() {
                rec.result = result;
            }
            if error.is_some() {
                rec.error = error;
            }
            if email.is_some() {
                rec.email = email;
            }
            if trace.is_some() {
                rec.trace = trace;
            }
            self.persist();
        }
    }

    pub fn snapshot_len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn count_states(&self) -> (usize, usize) {
        let map = self.inner.read();
        let mut queued = 0usize;
        let mut running = 0usize;
        for rec in map.values() {
            match rec.state {
                ImageTaskState::Queued | ImageTaskState::TimeoutPending => queued += 1,
                ImageTaskState::Running => running += 1,
                ImageTaskState::Done | ImageTaskState::Failed => {}
            }
        }
        (queued, running)
    }
}

pub struct ImageTaskService {
    pub store: Arc<ImageTaskStore>,
    queue_tx: mpsc::Sender<String>,
}

impl ImageTaskService {
    pub fn spawn<F, Fut>(worker_fn: F, workers: usize) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let workers = workers.max(1).min(64);
        let store = Arc::new(ImageTaskStore::from_env());
        let (queue_tx, mut queue_rx) = mpsc::channel::<String>(512);
        let worker_sem = Arc::new(Semaphore::new(workers));
        let worker_fn = Arc::new(worker_fn);

        tokio::spawn(async move {
            while let Some(task_id) = queue_rx.recv().await {
                let permit = worker_sem.clone().acquire_owned().await;
                let worker_fn = worker_fn.clone();
                tokio::spawn(async move {
                    worker_fn(task_id).await;
                    if let Ok(p) = permit {
                        drop(p);
                    }
                });
            }
        });

        Self {
            store,
            queue_tx,
        }
    }

    pub fn enqueue(&self, request: ImageGenerationRequest) -> String {
        let id = format!("imgtask-{}", Uuid::new_v4());
        let now = protocol::chrono_secs() as i64;
        let record = ImageTaskRecord {
            id: id.clone(),
            state: ImageTaskState::Queued,
            request,
            created_at: now,
            updated_at: now,
            result: None,
            error: None,
            email: None,
            trace: None,
        };
        self.store.insert(record);
        if let Err(e) = self.queue_tx.try_send(id.clone()) {
            warn!(error = %e, task_id = %id, "image task queue full");
        }
        id
    }

    pub fn try_enqueue(&self, request: ImageGenerationRequest) -> Result<String, String> {
        let id = format!("imgtask-{}", Uuid::new_v4());
        let now = protocol::chrono_secs() as i64;
        let record = ImageTaskRecord {
            id: id.clone(),
            state: ImageTaskState::Queued,
            request,
            created_at: now,
            updated_at: now,
            result: None,
            error: None,
            email: None,
            trace: None,
        };
        self.store.insert(record);
        self.queue_tx
            .try_send(id.clone())
            .map_err(|e| format!("image_task_queue_full: {e}"))?;
        Ok(id)
    }
}

pub fn append_task_trace_ndjson(task_id: &str, trace: &Value) {
    let path = std::env::var("IMAGE_TASK_TRACE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/pool/image_task_trace.ndjson"));
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let line = serde_json::json!({
        "task_id": task_id,
        "trace": trace,
        "ts": protocol::chrono_secs(),
    });
    if let Ok(raw) = serde_json::to_string(&line) {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{raw}");
        }
    }
}

pub fn task_state_str(state: &ImageTaskState) -> &'static str {
    match state {
        ImageTaskState::Queued => "queued",
        ImageTaskState::Running => "running",
        ImageTaskState::TimeoutPending => "timeout_pending",
        ImageTaskState::Done => "done",
        ImageTaskState::Failed => "failed",
    }
}

pub fn log_task_fail(store: &ImageTaskStore, task_id: &str, err: &str) {
    error!(task_id = %task_id, error = %err, "image task failed");
    store.update_state(
        task_id,
        ImageTaskState::Failed,
        None,
        Some(err.to_string()),
        None,
        None,
    );
}

pub fn log_task_done(store: &ImageTaskStore, task_id: &str, result: Value, email: &str, trace: Option<Value>) {
    info!(task_id = %task_id, email = %email, "image task done");
    store.update_state(
        task_id,
        ImageTaskState::Done,
        Some(result),
        None,
        Some(email.to_string()),
        trace,
    );
}
