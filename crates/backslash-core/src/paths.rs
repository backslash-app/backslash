//! XDG locations. Everything is plain files so it can live in dotfiles.

use std::path::{Path, PathBuf};

fn base(dir: Option<PathBuf>, fallback: &str) -> PathBuf {
    dir.unwrap_or_else(|| home().join(fallback)).join("backslash")
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/.config/backslash`: `config.toml`, `style.css`.
pub fn config_dir() -> PathBuf {
    base(dirs::config_dir(), ".config")
}

/// `~/.local/share/backslash`: installed extensions and their storage.
pub fn data_dir() -> PathBuf {
    base(dirs::data_dir(), ".local/share")
}

/// `~/.cache/backslash`: the extension catalog, downloads.
pub fn cache_dir() -> PathBuf {
    base(dirs::cache_dir(), ".cache")
}

/// `~/.local/state/backslash`: usage history, generated import maps.
pub fn state_dir() -> PathBuf {
    base(dirs::state_dir(), ".local/state")
}

/// Directories scanned for extensions, highest priority first.
pub fn extension_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("BACKSLASH_EXTENSIONS_PATH").map(|v| std::env::split_paths(&v).collect()).unwrap_or_default();
    dirs.push(data_dir().join("extensions"));
    let system = std::env::var_os("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]);
    dirs.extend(system.into_iter().map(|d| d.join("backslash/extensions")));
    // Bundled with the binary, wherever it was unpacked.
    if let Some(bundled) = installed_share().map(|d| d.join("extensions")) {
        if !dirs.contains(&bundled) {
            dirs.push(bundled);
        }
    }
    dirs
}

/// `<prefix>/share/backslash` for a binary in `<prefix>/bin`.
fn installed_share() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("share/backslash"))
}

/// The TypeScript SDK shipped with this exact launcher version, so extensions
/// can never run against a mismatched API.
pub fn sdk_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BACKSLASH_SDK") {
        return dir.into();
    }
    // Installed: <prefix>/bin/backslash → <prefix>/share/backslash/sdk.
    let installed = installed_share().map(|d| d.join("sdk"));
    if let Some(dir) = installed.filter(|d| d.join("mod.ts").exists()) {
        return dir;
    }
    // Running from a source checkout (`cargo run`).
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk");
    if dev.join("mod.ts").exists() {
        return dev.canonicalize().unwrap_or(dev);
    }
    PathBuf::from("/usr/share/backslash/sdk")
}

/// Expands a leading `~` and `$VAR`/`${VAR}` references.
pub fn expand(path: &str) -> PathBuf {
    let mut out = String::new();
    let mut rest = path;
    if let Some(stripped) = rest.strip_prefix('~') {
        if stripped.is_empty() || stripped.starts_with('/') {
            out.push_str(&home().to_string_lossy());
            rest = stripped;
        }
    }
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_alphanumeric() || n == '_' {
                name.push(n);
                chars.next();
            } else {
                break;
            }
        }
        if braced && chars.peek() == Some(&'}') {
            chars.next();
        }
        out.push_str(&std::env::var(&name).unwrap_or_default());
    }
    PathBuf::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_and_vars() {
        std::env::set_var("BS_TEST_DIR", "/srv/x");
        assert_eq!(expand("~/notes"), home().join("notes"));
        assert_eq!(expand("$BS_TEST_DIR/a"), PathBuf::from("/srv/x/a"));
        assert_eq!(expand("${BS_TEST_DIR}b"), PathBuf::from("/srv/xb"));
        assert_eq!(expand("~user/x"), PathBuf::from("~user/x"));
    }
}
