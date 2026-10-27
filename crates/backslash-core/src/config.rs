//! `~/.config/backslash/config.toml`, hand-editable and dotfiles friendly.
//!
//! ```toml
//! [extensions.github]
//! enabled = true
//! preferences = { username = "octocat" }
//! ```
//!
//! Secret preferences go in `secrets.toml` next to it (created with mode 0600).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::manifest::{Manifest, PreferenceKind};
use crate::paths;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Applications whose desktop id is listed here are hidden from results.
    pub hidden_apps: Vec<String>,
    pub extensions: BTreeMap<String, ExtensionConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ExtensionConfig {
    pub enabled: bool,
    pub preferences: toml::Table,
}

impl Default for ExtensionConfig {
    fn default() -> Self {
        Self { enabled: true, preferences: toml::Table::new() }
    }
}

fn config_path() -> PathBuf {
    paths::config_dir().join("config.toml")
}

fn secrets_path() -> PathBuf {
    paths::config_dir().join("secrets.toml")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|text| match toml::from_str(&text) {
                Ok(config) => Some(config),
                Err(err) => {
                    eprintln!("backslash: ignoring invalid config.toml: {err}");
                    None
                }
            })
            .unwrap_or_default()
    }

    pub fn is_enabled(&self, extension_id: &str) -> bool {
        self.extensions.get(extension_id).is_none_or(|e| e.enabled)
    }

    /// Preferences for one extension, with manifest defaults applied. Returns
    /// the names of required preferences that are still missing.
    pub fn preferences(&self, manifest: &Manifest) -> (Map<String, Value>, Vec<String>) {
        let id = &manifest.extension.id;
        let plain = self.extensions.get(id).map(|e| e.preferences.clone()).unwrap_or_default();
        let secrets: toml::Table = std::fs::read_to_string(secrets_path())
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
            .and_then(|mut t| t.remove(id))
            .and_then(|v| v.as_table().cloned())
            .unwrap_or_default();

        let mut out = Map::new();
        let mut missing = Vec::new();
        for pref in &manifest.preferences {
            let source = if pref.kind == PreferenceKind::Secret { &secrets } else { &plain };
            let value = source.get(&pref.name).or(pref.default.as_ref());
            match value {
                Some(v) => {
                    out.insert(pref.name.clone(), serde_json::to_value(v).unwrap_or(Value::Null));
                }
                None if pref.required => missing.push(pref.name.clone()),
                None => {}
            }
        }
        (out, missing)
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(paths::config_dir())?;
        std::fs::write(config_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
