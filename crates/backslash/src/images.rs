//! Pictures from the web (avatars, favicons), downloaded once into the cache
//! folder. The first request starts the download and returns nothing; when
//! it lands, the registered callback redraws whatever asked.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use backslash_core::paths;
use gtk::{gdk, glib};
use sha2::{Digest, Sha256};

/// Bigger files are not pictures we want in a launcher.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

thread_local! {
    static TEXTURES: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::default();
    static PENDING: RefCell<HashSet<String>> = RefCell::default();
    static ON_READY: RefCell<Option<Box<dyn Fn()>>> = RefCell::default();
}

pub fn is_url(spec: &str) -> bool {
    spec.starts_with("https://") || spec.starts_with("http://")
}

/// The host of a URL, for permission checks.
pub fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    Some(authority.rsplit_once('@').map_or(authority, |(_, h)| h).split(':').next().unwrap_or(authority))
}

/// Whether `url` may be fetched for an extension with these `net` hosts.
pub fn allowed(url: &str, hosts: &[String]) -> bool {
    let Some(host) = host(url) else { return false };
    hosts.iter().any(|h| h == "*" || h == host || h.split(':').next() == Some(host))
}

/// Called (on the main thread) each time a picture finished downloading.
pub fn on_ready(f: impl Fn() + 'static) {
    ON_READY.with(|r| r.replace(Some(Box::new(f))));
}

fn file_for(url: &str) -> PathBuf {
    let hash = Sha256::digest(url.as_bytes());
    let name: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    paths::cache_dir().join("images").join(name)
}

/// The picture at `url` if it is already here; otherwise starts fetching it.
pub fn texture(url: &str) -> Option<gdk::Texture> {
    if let Some(cached) = TEXTURES.with(|t| t.borrow().get(url).cloned()) {
        return cached;
    }
    let file = file_for(url);
    if file.is_file() {
        let texture = gdk::Texture::from_filename(&file).ok();
        TEXTURES.with(|t| t.borrow_mut().insert(url.to_string(), texture.clone()));
        return texture;
    }
    if PENDING.with(|p| p.borrow_mut().insert(url.to_string())) {
        let url = url.to_string();
        std::thread::spawn(move || {
            let ok = download(&url, &file).is_ok();
            glib::MainContext::default().invoke(move || {
                PENDING.with(|p| p.borrow_mut().remove(&url));
                if !ok {
                    // Don't try again during this session.
                    TEXTURES.with(|t| t.borrow_mut().insert(url, None));
                    return;
                }
                ON_READY.with(|r| {
                    if let Some(f) = r.borrow().as_ref() {
                        f();
                    }
                });
            });
        });
    }
    None
}

fn download(url: &str, file: &PathBuf) -> anyhow::Result<()> {
    let agent = backslash_core::http::agent(Duration::from_secs(10));
    let mut response = agent.get(url).call()?;
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(MAX_BYTES).read_to_end(&mut bytes)?;
    // Only keep what GDK can show.
    gdk::Texture::from_bytes(&glib::Bytes::from(&bytes))?;
    std::fs::create_dir_all(file.parent().expect("cache folder"))?;
    std::fs::write(file, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosts() {
        assert_eq!(super::host("https://avatars.githubusercontent.com/u/1?v=4"), Some("avatars.githubusercontent.com"));
        let hosts = vec!["avatars.githubusercontent.com".to_string()];
        assert!(super::allowed("https://avatars.githubusercontent.com/u/1", &hosts));
        assert!(!super::allowed("https://evil.example/x.png", &hosts));
        assert!(super::allowed("https://a.b/c", &["*".to_string()]));
    }
}
