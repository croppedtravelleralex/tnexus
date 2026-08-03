//! Recent prompt dedup — returns 429 when same account repeats identical prompt too fast.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(120);

pub struct DuplicatePromptGate {
    inner: Mutex<HashMap<String, (String, Instant)>>,
}

impl DuplicatePromptGate {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, email: &str, prompt: &str) -> bool {
        let key = email.trim().to_lowercase();
        let prompt = prompt.trim();
        if key.is_empty() || prompt.is_empty() {
            return false;
        }
        let now = Instant::now();
        let mut map = self.inner.lock().expect("duplicate prompt lock");
        map.retain(|_, (_, t)| now.duration_since(*t) < TTL);
        if let Some((prev, t)) = map.get(&key) {
            if prev == prompt && now.duration_since(*t) < TTL {
                return true;
            }
        }
        map.insert(key, (prompt.to_string(), now));
        false
    }
}

impl Default for DuplicatePromptGate {
    fn default() -> Self {
        Self::new()
    }
}
