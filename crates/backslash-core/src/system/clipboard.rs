//! Clipboard history, stored in `~/.local/state/backslash/clipboard.json`
//! (mode 0600: it may contain sensitive text). Copied images are kept as PNG
//! files next to it, in `clipboard-images/`, named after their checksum.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use sha2::Digest;

use crate::store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub id: u64,
    pub text: String,
    /// Unix time of the last copy.
    pub time: u64,
    #[serde(default)]
    pub pinned: bool,
    /// A copied image: its file in the images folder (`text` is empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// `<sha256>.png`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// Larger copies (logs, files pasted as text) are not worth keeping.
const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Screenshots are big; whole photos at full size are not worth keeping.
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    entries: Vec<ClipEntry>,
    path: Option<PathBuf>,
    images: Option<PathBuf>,
    max: usize,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl ClipboardHistory {
    pub fn load(path: PathBuf, max: usize) -> Self {
        let entries: Vec<ClipEntry> = store::read_json(&path);
        let images = path.parent().map(|d| d.join("clipboard-images"));
        Self { entries, path: Some(path), images, max }
    }

    #[cfg(test)]
    fn in_memory(max: usize) -> Self {
        Self { entries: Vec::new(), path: None, images: None, max }
    }

    /// The file of an image entry.
    pub fn image_path(&self, entry: &ClipEntry) -> Option<PathBuf> {
        Some(self.images.as_ref()?.join(&entry.image.as_ref()?.file))
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
        self.push(text.to_string(), None)
    }

    /// Records a copied image (PNG data). Returns `false` when nothing
    /// changed (too big, not kept on disk, or already the latest entry).
    pub fn add_image(&mut self, png: &[u8], width: u32, height: u32) -> bool {
        if png.is_empty() || png.len() > MAX_IMAGE_BYTES {
            return false;
        }
        let digest = sha2::Sha256::digest(png);
        let file = format!("{}.png", digest.iter().map(|b| format!("{b:02x}")).collect::<String>());
        if let Some(dir) = &self.images {
            if let Err(e) = write_private(&dir.join(&file), png) {
                eprintln!("backslash: cannot save a copied image: {e}");
                return false;
            }
        }
        self.push(String::new(), Some(ImageInfo { file, width, height, bytes: png.len() as u64 }))
    }

    fn push(&mut self, text: String, image: Option<ImageInfo>) -> bool {
        let same = |e: &ClipEntry| match (&image, &e.image) {
            (Some(new), Some(old)) => new.file == old.file,
            (None, None) => e.text == text,
            _ => false,
        };
        let latest = self.entries.iter().filter(|e| !e.pinned).max_by_key(|e| (e.time, e.id));
        if latest.is_some_and(same) {
            return false;
        }
        let pinned = match self.entries.iter().position(same) {
            Some(i) => self.entries.remove(i).pinned,
            None => false,
        };
        let id = self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        self.entries.push(ClipEntry { id, text, time: now(), pinned, image });
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

    /// Deletes image files no entry refers to any more.
    fn forget_images(&self) {
        let Some(dir) = &self.images else { return };
        let Ok(files) = std::fs::read_dir(dir) else { return };
        let used: std::collections::HashSet<&str> =
            self.entries.iter().filter_map(|e| e.image.as_ref().map(|i| i.file.as_str())).collect();
        for file in files.flatten() {
            let name = file.file_name();
            if name.to_str().is_some_and(|n| n.ends_with(".png") && !used.contains(n)) {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }

    fn save(&self) {
        self.forget_images();
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

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if path.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("images folder"))?;
    let tmp = path.with_extension("part");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(data)?;
    std::fs::rename(&tmp, path)
}

/// Width and height of a PNG, from its header.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let be = |b: &[u8]| u32::from_be_bytes(b.try_into().expect("4 bytes"));
    Some((be(&png[16..20]), be(&png[20..24])))
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
    fn images_are_files_kept_while_used() {
        let dir = tempfile::tempdir().unwrap();
        let mut h = ClipboardHistory::load(dir.path().join("clipboard.json"), 1);
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x03\x20\0\0\x02\x58rest";
        assert_eq!(png_size(png), Some((800, 600)));
        assert!(h.add_image(png, 800, 600));
        assert!(!h.add_image(png, 800, 600), "same image as latest");
        let entry = h.entries()[0].clone();
        assert_eq!(entry.image.as_ref().map(|i| (i.width, i.height)), Some((800, 600)));
        let file = h.image_path(&entry).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), png);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        // A text pushes the image out (max 1): its file goes too.
        h.add("text");
        assert_eq!(h.entries().len(), 1);
        assert!(!file.exists());
        // Pinned images stay.
        h.add_image(png, 800, 600);
        let id = h.entries()[0].id;
        h.toggle_pin(id);
        h.clear();
        assert!(file.exists());
        assert_eq!(ClipboardHistory::load(dir.path().join("clipboard.json"), 1).entries()[0].image, entry.image);
    }

    #[test]
    fn summaries() {
        assert_eq!(summary("  hello\nworld"), "hello…");
        assert_eq!(summary("\n\nx"), "x");
        assert_eq!(summary(&"a".repeat(100)).chars().count(), 91);
    }
}
