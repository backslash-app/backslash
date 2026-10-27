//! File search in the home directory, and recently used files.
//!
//! Searching uses the best tool available: `fd` (live and fast), then
//! `plocate`/`locate` (indexed, but the index is usually refreshed only once a
//! day), then a bounded walk of the home directory.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use super::which as tool;
use crate::paths;

pub const MAX_RESULTS: usize = 60;

/// Recently used files from `~/.local/share/recently-used.xbel` (GTK, KDE and
/// most apps write it), newest first, existing files only.
pub fn recent(limit: usize) -> Vec<PathBuf> {
    let path = dirs::data_dir().unwrap_or_else(|| paths::home().join(".local/share")).join("recently-used.xbel");
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut entries = parse_xbel(&text);
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.into_iter().map(|(p, _)| p).filter(|p| p.is_file()).take(limit).collect()
}

/// `(path, modified)` pairs; `modified` is an ISO 8601 string, which sorts as text.
pub fn parse_xbel(text: &str) -> Vec<(PathBuf, String)> {
    let attr = |tag: &str, name: &str| -> Option<String> {
        let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
        let end = tag[start..].find('"')? + start;
        Some(tag[start..end].to_string())
    };
    let mut out = Vec::new();
    for chunk in text.split("<bookmark ").skip(1) {
        let tag = &chunk[..chunk.find('>').unwrap_or(chunk.len())];
        let tag = format!(" {tag}");
        let Some(href) = attr(&tag, "href") else { continue };
        let Some(path) = href.strip_prefix("file://") else { continue };
        let modified = attr(&tag, "modified").or_else(|| attr(&tag, "visited")).unwrap_or_default();
        out.push((PathBuf::from(percent_decode(&unescape_xml(path))), modified));
    }
    out
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&").replace("&apos;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Which backend [`search`] will use, for `backslash doctor`.
pub fn backend() -> &'static str {
    if tool(&["fd", "fdfind"]).is_some() {
        "fd"
    } else if tool(&["plocate", "locate"]).is_some() && locate_db_exists() {
        "locate"
    } else {
        "built-in walk"
    }
}

fn locate_db_exists() -> bool {
    ["/var/lib/plocate/plocate.db", "/var/lib/mlocate/mlocate.db", "/var/lib/locate/locatedb"]
        .iter()
        .any(|p| Path::new(p).exists())
}

fn hidden_or_noise(path: &Path, home: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(home) else { return true };
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || s == "node_modules" || s == "target" || s == "__pycache__"
    })
}

/// Finds files and folders under the home directory whose name contains `query`
/// (case-insensitive). `cancel` aborts a search made obsolete by more typing.
pub fn search(query: &str, cancel: &AtomicBool) -> Vec<PathBuf> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Vec::new();
    }
    let home = paths::home();
    let mut hits = match (backend(), tool(&["fd", "fdfind"])) {
        ("fd", Some(fd)) => fd_search(&fd, query, &home),
        ("locate", _) => locate(query, &home),
        _ => walk(query, &home, cancel),
    };
    hits.retain(|p| !hidden_or_noise(p, &home));
    rank(query, &mut hits);
    hits.truncate(MAX_RESULTS);
    hits
}

fn locate(query: &str, home: &Path) -> Vec<PathBuf> {
    let Some(bin) = tool(&["plocate", "locate"]) else { return Vec::new() };
    let out = Command::new(&bin).args(["-i", "-b", "-l", "2000", "--", query]).output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).filter(|p| p.starts_with(home)).collect()
}

fn fd_search(bin: &str, query: &str, home: &Path) -> Vec<PathBuf> {
    let out = Command::new(bin)
        .args(["--ignore-case", "--fixed-strings", "--absolute-path", "--max-results", "500", "--", query])
        .arg(home)
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).lines().map(|l| PathBuf::from(l.trim_end_matches('/'))).collect()
}

/// Bounded breadth-first walk: depth 6, 50k entries, 1.5 s.
fn walk(query: &str, home: &Path, cancel: &AtomicBool) -> Vec<PathBuf> {
    let needle = query.to_lowercase();
    let started = SystemTime::now();
    let mut queue = std::collections::VecDeque::from([(home.to_path_buf(), 0usize)]);
    let (mut seen, mut hits) = (0usize, Vec::new());
    while let Some((dir, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed)
            || seen > 50_000
            || started.elapsed().unwrap_or_default() > Duration::from_millis(1500)
        {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            let path = entry.path();
            if name.contains(&needle) {
                hits.push(path.clone());
            }
            if depth < 6 && entry.file_type().is_ok_and(|t| t.is_dir()) {
                queue.push_back((path, depth + 1));
            }
        }
    }
    hits
}

/// Exact name, then prefix, then substring; shallower paths first.
fn rank(query: &str, hits: &mut [PathBuf]) {
    let q = query.to_lowercase();
    hits.sort_by_cached_key(|p| {
        let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let stem = p.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let tier = if name == q || stem == q {
            0
        } else if name.starts_with(&q) {
            1
        } else {
            2
        };
        (tier, p.components().count(), name)
    });
}

/// `~/Documents/x` instead of `/home/me/Documents/x`.
pub fn tilde(path: &Path) -> String {
    let home = paths::home();
    match path.strip_prefix(&home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".into(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xbel() {
        let xbel = r#"<?xml version="1.0"?>
<xbel version="1.0">
  <bookmark href="file:///home/me/Doc%20s/report%C3%A9.pdf" added="2026-10-01T10:00:00Z" modified="2026-10-20T10:00:00Z" visited="2026-10-20T10:00:00Z">
    <info/>
  </bookmark>
  <bookmark href="https://example.com" modified="2026-10-21T10:00:00Z"/>
  <bookmark href="file:///tmp/a&amp;b.txt" added="x" modified="2026-10-22T10:00:00Z"></bookmark>
</xbel>"#;
        let parsed = parse_xbel(xbel);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, PathBuf::from("/home/me/Doc s/reportÃ©.pdf".replace("Ã©", "é")));
        assert_eq!(parsed[1].0, PathBuf::from("/tmp/a&b.txt"));
        assert_eq!(parsed[1].1, "2026-10-22T10:00:00Z");
    }

    #[test]
    fn ranking_prefers_exact_then_prefix() {
        let mut hits = vec![
            PathBuf::from("/h/projects/my-notes.md"),
            PathBuf::from("/h/a/b/notes"),
            PathBuf::from("/h/notes.txt"),
            PathBuf::from("/h/notes-old.txt"),
        ];
        rank("notes", &mut hits);
        let names: Vec<_> = hits.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["notes.txt", "notes", "notes-old.txt", "my-notes.md"]);
    }

    #[test]
    fn walk_finds_files_and_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Docs/sub")).unwrap();
        std::fs::create_dir_all(dir.path().join(".cache")).unwrap();
        std::fs::write(dir.path().join("Docs/sub/Budget-2026.ods"), "").unwrap();
        std::fs::write(dir.path().join(".cache/budget.tmp"), "").unwrap();
        let hits = walk("budget", dir.path(), &AtomicBool::new(false));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].ends_with("Docs/sub/Budget-2026.ods"));
    }
}
