//! Cross-process locked JSON file read/write with atomic rename.
//!
//! `fs2` advisory locks coordinate readers and writers across processes; writes go to a
//! same-volume temp file and are renamed atomically so a concurrent reader never observes a
//! torn write. Used by both `tnexus-api` and `gateway` to guard `scheduling_state.json`.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// Serialize `value` to pretty JSON, write it to `path` via a same-volume temp file
/// renamed atomically, all under an exclusive advisory lock.
///
/// Lock is released when the lock file handle closes (end of function).
pub fn write_json<D: serde::Serialize>(path: &Path, value: &D) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create dir {:?}", parent))?;
    }
    let lock = File::open(path).with_context(|| format!("open lock {:?}", path))?;
    lock.lock_exclusive()
        .with_context(|| format!("lock exclusive {:?}", path))?;
    let _guard = LockGuard { file: lock };

    let raw = serde_json::to_string_pretty(value).context("serialize json")?;
    let tmp = temp_path(path);
    fs::write(&tmp, raw).with_context(|| format!("write temp {:?}", tmp))?;
    fs::rename(&tmp, path).with_context(|| format!("rename {:?} -> {:?}", tmp, path))?;
    Ok(())
}

/// Read and deserialize `path` under a shared advisory lock. Missing or empty file yields `None`.
pub fn read_json<D: for<'de> serde::Deserialize<'de>>(path: &Path) -> Result<Option<D>> {
    if !path.exists() {
        return Ok(None);
    }
    let lock = File::open(path).with_context(|| format!("open lock {:?}", path))?;
    lock.lock_shared()
        .with_context(|| format!("lock shared {:?}", path))?;
    let _guard = LockGuard { file: lock };

    let raw = fs::read_to_string(path).with_context(|| format!("read {:?}", path))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let value: D = serde_json::from_str(&raw).context("parse json")?;
    Ok(Some(value))
}

fn temp_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

/// Holds an fs2 file lock; releases it on drop (when the file handle closes).
struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
