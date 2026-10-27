//! Small JSON files written atomically: extension storage and usage history.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Unique temporary name: concurrent writers (two processes starting the
    // same extension) must not rename each other's file away.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}-{n}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Key/value storage private to one extension, served over the protocol so
/// extensions need no filesystem permission to persist data.
pub struct ExtensionStorage {
    path: PathBuf,
}

/// Total size allowed per extension, to keep a buggy extension from filling the disk.
const MAX_BYTES: usize = 5 * 1024 * 1024;

impl ExtensionStorage {
    pub fn new(root: &Path, extension_id: &str) -> Self {
        Self { path: root.join(format!("{extension_id}.json")) }
    }

    pub fn get(&self, key: &str) -> Value {
        read_json::<Map<String, Value>>(&self.path).remove(key).unwrap_or(Value::Null)
    }

    pub fn set(&self, key: &str, value: Value) -> Result<()> {
        let mut map: Map<String, Value> = read_json(&self.path);
        if value.is_null() {
            map.remove(key);
        } else {
            map.insert(key.to_string(), value);
        }
        if serde_json::to_vec(&map)?.len() > MAX_BYTES {
            anyhow::bail!("storage quota exceeded (5 MB)");
        }
        write_json(&self.path, &map)
    }

    pub fn keys(&self) -> Vec<String> {
        read_json::<Map<String, Value>>(&self.path).keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn concurrent_writes_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || (0..50).for_each(|_| write_json(&path, &json!(i)).unwrap()))
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert!(read_json::<Value>(&path).is_number());
    }

    #[test]
    fn storage_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = ExtensionStorage::new(dir.path(), "npm");
        assert_eq!(s.get("recent"), Value::Null);
        s.set("recent", json!(["react"])).unwrap();
        assert_eq!(s.get("recent"), json!(["react"]));
        assert_eq!(s.keys(), vec!["recent"]);
        s.set("recent", Value::Null).unwrap();
        assert!(s.keys().is_empty());
    }
}
