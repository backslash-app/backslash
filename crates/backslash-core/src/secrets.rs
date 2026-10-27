//! Secret preferences (API tokens…). Stored in the desktop keyring through
//! `secret-tool` (libsecret: GNOME Keyring, KWallet, KeePassXC…) when it
//! works, otherwise in `~/.config/backslash/secrets.toml` with mode 0600.
//!
//! `BACKSLASH_SECRETS=file` forces the file backend.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keyring,
    File,
}

impl Backend {
    pub fn detect() -> Self {
        if std::env::var("BACKSLASH_SECRETS").as_deref() == Ok("file") {
            return Self::File;
        }
        if crate::system::which(&["secret-tool"]).is_some() && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
            Self::Keyring
        } else {
            Self::File
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Keyring => "system keyring (libsecret)",
            Self::File => "~/.config/backslash/secrets.toml (mode 0600)",
        }
    }
}

fn file_path() -> PathBuf {
    paths::config_dir().join("secrets.toml")
}

fn read_file() -> toml::Table {
    std::fs::read_to_string(file_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

fn write_file(table: &toml::Table) -> Result<()> {
    let path = file_path();
    std::fs::create_dir_all(paths::config_dir())?;
    std::fs::write(&path, toml::to_string_pretty(table)?)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn attributes<'a>(extension: &'a str, key: &'a str) -> [&'a str; 6] {
    ["application", "backslash", "extension", extension, "key", key]
}

pub fn get(extension: &str, key: &str) -> Option<String> {
    if Backend::detect() == Backend::Keyring {
        let out = Command::new("secret-tool").arg("lookup").args(attributes(extension, key)).output().ok();
        if let Some(out) = out.filter(|o| o.status.success() && !o.stdout.is_empty()) {
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    // Also read the file with the keyring available, for secrets saved before.
    read_file().get(extension)?.get(key)?.as_str().map(str::to_owned)
}

pub fn set(extension: &str, key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return delete(extension, key);
    }
    if Backend::detect() == Backend::Keyring {
        let mut child = Command::new("secret-tool")
            .args(["store", &format!("--label=Backslash: {extension} {key}")])
            .args(attributes(extension, key))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot run secret-tool")?;
        child.stdin.take().expect("piped").write_all(value.as_bytes())?;
        let out = child.wait_with_output()?;
        if out.status.success() {
            // Don't leave an older copy in the file.
            remove_from_file(extension, key)?;
            return Ok(());
        }
        eprintln!(
            "backslash: keyring unavailable ({}), using secrets.toml",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut table = read_file();
    let entry = table.entry(extension.to_string()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(ext) = entry.as_table_mut() else { bail!("secrets.toml: [{extension}] is not a table") };
    ext.insert(key.to_string(), toml::Value::String(value.to_string()));
    write_file(&table)
}

pub fn delete(extension: &str, key: &str) -> Result<()> {
    if Backend::detect() == Backend::Keyring {
        let _ = Command::new("secret-tool").arg("clear").args(attributes(extension, key)).status();
    }
    remove_from_file(extension, key)
}

fn remove_from_file(extension: &str, key: &str) -> Result<()> {
    let mut table = read_file();
    let removed = table.get_mut(extension).and_then(|t| t.as_table_mut()).and_then(|t| t.remove(key)).is_some();
    if removed {
        write_file(&table)?;
    }
    Ok(())
}

/// Tests that point `XDG_CONFIG_HOME` at a temporary directory take this.
#[cfg(test)]
pub(crate) static TEST_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backend_round_trip() {
        let _env = TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", dir.path());
        std::env::set_var("BACKSLASH_SECRETS", "file");
        set("github", "token", "ghp_123").unwrap();
        assert_eq!(get("github", "token").as_deref(), Some("ghp_123"));
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("backslash/secrets.toml")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        set("github", "token", "").unwrap();
        assert_eq!(get("github", "token"), None);
    }
}
