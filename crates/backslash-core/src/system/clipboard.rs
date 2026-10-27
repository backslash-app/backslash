//! Clipboard history, stored in `~/.local/state/backslash/clipboard.json`
//! (mode 0600: it may contain sensitive text).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub id: u64,
    pub text: String,
    /// Unix time of the last copy.
    pub time: u64,
    #[serde(default)]
    pub pinned: bool,
}

/// Larger copies (logs, files pasted as text) are not worth keeping.
const MAX_ENTRY_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    entries: Vec<ClipEntry>,
    path: Option<PathBuf>,
    max: usize,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl ClipboardHistory {
    pub fn load(path: PathBuf, max: usize) -> Self {
        let entries: Vec<ClipEntry> = store::read_json(&path);
        Self { entries, path: Some(path), max }
    }

    #[cfg(test)]
    fn in_memory(max: usize) -> Self {
        Self { entries: Vec::new(), path: None, max }
    }

    /// Pinned first, then most recent first.
    pub fn entries(&self) -> Vec<&ClipEntry> {
        let mut out: Vec<&ClipEntry> = self.entries.iter().collect();
        out.sort_by_key(|e| (std::cmp::Reverse(e.pinned), std::cmp::Reverse(e.time), std::cmp::Reverse(e.id)));
        out
    }

    pub fn get(&self, id: u64) -> Option<&ClipEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Records a copy. Returns `false` when nothing changed (empty, too big,
    /// or already the latest entry).
    pub fn add(&mut self, text: &str) -> bool {
        if text.trim().is_empty() || text.len() > MAX_ENTRY_BYTES {
            return false;
        }
        let latest = self.entries.iter().filter(|e| !e.pinned).max_by_key(|e| (e.time, e.id));
        if latest.is_some_and(|e| e.text == text) {
            return false;
        }
        let pinned = match self.entries.iter().position(|e| e.text == text) {
            Some(i) => self.entries.remove(i).pinned,
            None => false,
        };
        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.entries.push(ClipEntry { id, text: text.to_string(), time: now(), pinned });
        // Drop the oldest unpinned entries beyond the limit.
        let unpinned = self.entries.iter().filter(|e| !e.pinned).count();
        if unpinned > self.max {
            let mut old: Vec<(u64, u64)> = self.entries.iter().filter(|e| !e.pinned).map(|e| (e.time, e.id)).collect();
            old.sort();
            let drop: Vec<u64> = old.iter().take(unpinned - self.max).map(|(_, id)| *id).collect();
            self.entries.retain(|e| !drop.contains(&e.id));
        }
        self.save();
        true
    }

    pub fn delete(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
        self.save();
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = !e.pinned;
        }
        self.save();
    }

    /// Removes everything but pinned entries.
    pub fn clear(&mut self) {
        self.entries.retain(|e| e.pinned);
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Err(e) = store::write_json(path, &self.entries) {
            eprintln!("backslash: cannot save clipboard history: {e}");
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// First line, trimmed and shortened, for list rows.
pub fn summary(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let mut out: String = line.chars().take(90).collect();
    if line.chars().count() > 90 || text.trim().lines().count() > 1 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_limits_and_keeps_pins() {
        let mut h = ClipboardHistory::in_memory(2);
        assert!(h.add("one"));
        assert!(!h.add("one"), "same as latest");
        assert!(!h.add("   "));
        h.add("two");
        let one = h.entries().iter().find(|e| e.text == "one").unwrap().id;
        h.toggle_pin(one);
        h.add("three");
        h.add("four");
        let texts: Vec<&str> = h.entries().iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[0], "one", "pinned first");
        assert_eq!(texts.len(), 3, "2 unpinned + 1 pinned: {texts:?}");
        assert!(!texts.contains(&"two"));
        // Copying an old text again moves it to the top.
        h.add("three");
        assert_eq!(h.entries()[1].text, "three");
        h.clear();
        assert_eq!(h.entries().len(), 1);
    }

    #[test]
    fn persists_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard.json");
        let mut h = ClipboardHistory::load(path.clone(), 10);
        h.add("secret");
        let again = ClipboardHistory::load(path.clone(), 10);
        assert_eq!(again.entries()[0].text, "secret");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn summaries() {
        assert_eq!(summary("  hello\nworld"), "hello…");
        assert_eq!(summary("\n\nx"), "x");
        assert_eq!(summary(&"a".repeat(100)).chars().count(), 91);
    }
}
