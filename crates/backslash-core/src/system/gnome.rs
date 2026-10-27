//! The GNOME Shell extension that lets Backslash list and switch windows on
//! GNOME, where apps have no other way to. Shipped inside the binary.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

pub const UUID: &str = "backslash@backslash.app";
const METADATA: &str = include_str!("../../../../data/gnome-shell/backslash@backslash.app/metadata.json");
const EXTENSION: &str = include_str!("../../../../data/gnome-shell/backslash@backslash.app/extension.js");

fn dir() -> PathBuf {
    let data = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    data.join("gnome-shell/extensions").join(UUID)
}

pub fn is_installed() -> bool {
    dir().join("extension.js").exists()
}

/// Installs and enables the extension. GNOME on Wayland loads new
/// extensions at the next login, so it works after logging out and in once.
pub fn install() -> Result<()> {
    let dir = dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    std::fs::write(dir.join("metadata.json"), METADATA)?;
    std::fs::write(dir.join("extension.js"), EXTENSION)?;
    let enabled = Command::new("gnome-extensions").args(["enable", UUID]).output().is_ok_and(|o| o.status.success());
    if !enabled {
        // Not discovered yet (Wayland): add it to the list GNOME enables at login.
        let current = Command::new("gsettings")
            .args(["get", "org.gnome.shell", "enabled-extensions"])
            .output()
            .context("gsettings is not available")?;
        let list = add_to_list(String::from_utf8_lossy(&current.stdout).trim(), UUID);
        let status =
            Command::new("gsettings").args(["set", "org.gnome.shell", "enabled-extensions", &list]).status()?;
        anyhow::ensure!(status.success(), "could not enable the extension with gsettings");
    }
    Ok(())
}

/// Adds `uuid` to a GVariant string list such as `['a@b', 'c@d']` or `@as []`.
pub fn add_to_list(list: &str, uuid: &str) -> String {
    let items: Vec<String> = list
        .trim_start_matches("@as")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut items = items;
    if !items.iter().any(|i| i == uuid) {
        items.push(uuid.to_string());
    }
    format!("[{}]", items.iter().map(|i| format!("'{i}'")).collect::<Vec<_>>().join(", "))
}

#[cfg(test)]
mod tests {
    use super::add_to_list;

    #[test]
    fn enabled_extensions_list() {
        assert_eq!(add_to_list("@as []", "x@y"), "['x@y']");
        assert_eq!(add_to_list("['a@b', 'c@d']", "x@y"), "['a@b', 'c@d', 'x@y']");
        assert_eq!(add_to_list("['x@y']", "x@y"), "['x@y']");
    }
}
