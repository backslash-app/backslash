//! `~/.config/backslash/config.toml`, hand-editable and dotfiles friendly.
//!
//! ```toml
//! theme = "system"            # "light", "dark", "system", "nord", "dracula", "catppuccin-mocha",
//!                             # "catppuccin-latte", "gruvbox", "solarized-light", "tokyo-night"
//! accent = "indigo"           # theme (the theme's own), indigo, blue, green, orange, pink, graphite
//! density = "comfortable"     # or "compact"
//! window_size = "medium"      # "small" or "large"
//! reopen = "restore"          # where I was; "minute": for a minute; "root": always the search
//! hotkey = "Super+Space"      # asked to the desktop through the GlobalShortcuts portal
//!
//! [extensions.github]
//! enabled = true
//! preferences = { username = "octocat" }
//!
//! [[quicklinks]]
//! name = "Search GitHub"
//! url = "https://github.com/search?q={query}"
//! keyword = "gh"
//!
//! [[snippets]]
//! name = "Email signature"
//! text = "Best,\nSteven"
//! keyword = "sig"
//!
//! # Anything in the root search can get an alias and a global shortcut.
//! [aliases]
//! "app:org.mozilla.firefox.desktop" = "ff"
//! "cmd:github/pull-requests" = "pr"
//!
//! [hotkeys]
//! "builtin:clipboard" = "Super+V"
//! ```
//!
//! Secret preferences go to the system keyring (see [`crate::secrets`]).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::manifest::{Manifest, PreferenceKind};
use crate::paths;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
    /// Color of the selection, keys and links: `indigo`, `blue`, `green`,
    /// `orange`, `pink` or `graphite`.
    pub accent: Accent,
    /// `compact` fits more rows.
    pub density: Density,
    /// `small`, `medium` or `large`.
    pub window_size: WindowSize,
    /// What the launcher shows when it opens again after hiding (after an
    /// action, Escape, or losing the focus).
    pub reopen: Reopen,
    /// Shortcut requested from the desktop, e.g. `Super+Space`. Empty disables it.
    pub hotkey: String,
    /// Show as an overlay on compositors supporting wlr-layer-shell (Sway, Hyprland, KDE…).
    pub layer_shell: bool,
    /// Applications whose desktop id is listed here are hidden from results.
    pub hidden_apps: Vec<String>,
    /// Git URL of the extension registry.
    pub registry: String,
    /// Keys the registry must be signed with, as `ssh-ed25519 …` lines. Unset:
    /// Backslash's own keys. `[]` turns the check off (a registry of your own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_signers: Option<Vec<String>>,
    pub clipboard: ClipboardConfig,
    pub extensions: BTreeMap<String, ExtensionConfig>,
    pub quicklinks: Vec<Quicklink>,
    pub snippets: Vec<Snippet>,
    /// Root search item key (`app:…`, `cmd:…`, `builtin:…`…) → alias typed to find it first.
    pub aliases: BTreeMap<String, String>,
    /// Root search item key → global shortcut that opens it.
    pub hotkeys: BTreeMap<String, String>,
    /// Look for new versions of Backslash on GitHub (at start, then every few hours).
    pub check_updates: bool,
    /// Show an icon in the system tray while Backslash runs.
    pub tray: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: Accent::default(),
            density: Density::default(),
            window_size: WindowSize::default(),
            reopen: Reopen::default(),
            hotkey: "Super+Space".into(),
            layer_shell: true,
            hidden_apps: Vec::new(),
            registry: crate::catalog::DEFAULT_REGISTRY.into(),
            registry_signers: None,
            clipboard: ClipboardConfig::default(),
            extensions: BTreeMap::new(),
            quicklinks: Vec::new(),
            snippets: Vec::new(),
            aliases: BTreeMap::new(),
            hotkeys: BTreeMap::new(),
            check_updates: true,
            tray: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the desktop's light/dark preference.
    #[default]
    System,
    Light,
    Dark,
    Nord,
    Dracula,
    #[serde(rename = "catppuccin-mocha")]
    CatppuccinMocha,
    #[serde(rename = "catppuccin-latte")]
    CatppuccinLatte,
    Gruvbox,
    #[serde(rename = "solarized-light")]
    SolarizedLight,
    #[serde(rename = "tokyo-night")]
    TokyoNight,
}

impl Theme {
    pub const ALL: [Theme; 10] = [
        Self::System,
        Self::Light,
        Self::Dark,
        Self::Nord,
        Self::Dracula,
        Self::CatppuccinMocha,
        Self::CatppuccinLatte,
        Self::Gruvbox,
        Self::SolarizedLight,
        Self::TokyoNight,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Nord => "Nord",
            Self::Dracula => "Dracula",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::Gruvbox => "Gruvbox",
            Self::SolarizedLight => "Solarized Light",
            Self::TokyoNight => "Tokyo Night",
        }
    }

    /// Whether it is dark, light, or follows the desktop (`None`).
    pub fn is_dark(self) -> Option<bool> {
        match self {
            Self::System => None,
            Self::Light | Self::CatppuccinLatte | Self::SolarizedLight => Some(false),
            _ => Some(true),
        }
    }
}

/// Size of the launcher window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl WindowSize {
    pub const ALL: [WindowSize; 3] = [Self::Small, Self::Medium, Self::Large];

    /// Width and height in pixels, and the width of the list next to the preview.
    pub fn pixels(self) -> (i32, i32, i32) {
        match self {
            Self::Small => (740, 460, 390),
            Self::Medium => (860, 540, 450),
            Self::Large => (1020, 660, 520),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Medium => "Medium",
            Self::Large => "Large",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    Indigo,
    /// The named theme's own accent (indigo for System, Light and Dark).
    #[default]
    Theme,
    Blue,
    Green,
    Orange,
    Pink,
    Graphite,
}

impl Accent {
    pub const ALL: [Accent; 7] =
        [Self::Theme, Self::Indigo, Self::Blue, Self::Green, Self::Orange, Self::Pink, Self::Graphite];

    pub fn name(self) -> &'static str {
        match self {
            Self::Indigo => "Indigo",
            Self::Theme => "Theme",
            Self::Blue => "Blue",
            Self::Green => "Green",
            Self::Orange => "Orange",
            Self::Pink => "Pink",
            Self::Graphite => "Graphite",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reopen {
    /// Where the user was: the open command, its search, filter and selection.
    #[default]
    Restore,
    /// Where the user was if it hid less than a minute ago, else the root search.
    Minute,
    /// Always the root search.
    Root,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// Record copied text in the clipboard history.
    pub enabled: bool,
    /// Unpinned entries kept.
    pub max_items: usize,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self { enabled: true, max_items: 300 }
    }
}

/// A URL opened from the launcher. `{query}` is replaced by what follows the
/// keyword: with `keyword = "gh"`, typing `gh tokio` opens the search for "tokio".
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Quicklink {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Text copied by the launcher. `{date}`, `{time}` and `{clipboard}` are expanded.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Snippet {
    pub name: String,
    pub text: String,
    #[serde(default)]
    pub keyword: Option<String>,
}

impl Quicklink {
    pub fn takes_query(&self) -> bool {
        self.url.contains("{query}")
    }

    /// The URL with `{query}` replaced (percent-encoded).
    pub fn url_for(&self, query: &str) -> String {
        self.url.replace("{query}", &percent_encode(query))
    }
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

        let mut out = Map::new();
        let mut missing = Vec::new();
        for pref in &manifest.preferences {
            let value = if pref.kind == PreferenceKind::Secret {
                crate::secrets::get(id, &pref.name).map(toml::Value::String)
            } else {
                plain.get(&pref.name).cloned()
            };
            match value.as_ref().or(pref.default.as_ref()) {
                Some(v) => {
                    out.insert(pref.name.clone(), serde_json::to_value(v).unwrap_or(Value::Null));
                }
                None if pref.required => missing.push(pref.name.clone()),
                None => {}
            }
        }
        (out, missing)
    }

    /// The item whose alias is exactly `query` (case-insensitive).
    pub fn alias_target(&self, query: &str) -> Option<&str> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        self.aliases.iter().find(|(_, alias)| alias.eq_ignore_ascii_case(q)).map(|(key, _)| key.as_str())
    }

    /// Path of the config file, for "Open Config" style actions.
    pub fn path() -> PathBuf {
        config_path()
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(paths::config_dir())?;
        std::fs::write(config_path(), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases() {
        let c: Config = toml::from_str("[aliases]\n\"app:firefox.desktop\" = \"ff\"").unwrap();
        assert_eq!(c.alias_target(" FF "), Some("app:firefox.desktop"));
        assert_eq!(c.alias_target("f"), None);
        assert_eq!(c.alias_target(""), None);
    }

    #[test]
    fn defaults_and_round_trip() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.theme, Theme::System);
        assert_eq!(c.hotkey, "Super+Space");
        assert!(c.clipboard.enabled);

        let text = r#"
            theme = "dark"
            [[quicklinks]]
            name = "GitHub"
            url = "https://github.com/search?q={query}"
            keyword = "gh"
            [[snippets]]
            name = "Mail"
            text = "me@example.com"
        "#;
        let c: Config = toml::from_str(text).unwrap();
        assert_eq!(c.theme, Theme::Dark);
        assert_eq!(c.quicklinks[0].url_for("tokio rs"), "https://github.com/search?q=tokio%20rs");
        assert!(c.quicklinks[0].takes_query());
        // Saving must produce valid TOML (values before tables).
        let saved = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&saved).unwrap();
        assert_eq!(back.snippets[0].text, "me@example.com");
    }
}
