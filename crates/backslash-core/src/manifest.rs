//! `backslash.toml`: what an extension is, what it may do, and which commands it offers.
//!
//! The launcher reads manifests without starting any process, so searching
//! commands stays instant no matter how many extensions are installed.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

pub const MANIFEST_FILE: &str = "backslash.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub extension: ExtensionMeta,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default, rename = "commands")]
    pub commands: Vec<CommandDef>,
    #[serde(default, rename = "preferences")]
    pub preferences: Vec<Preference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionMeta {
    /// Lowercase identifier, unique among installed extensions (`[a-z0-9-]+`).
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

fn default_version() -> String {
    "0.0.0".into()
}

/// How the extension process is started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Runtime {
    /// TypeScript/JavaScript run by Deno with the SDK preloaded and the
    /// declared permissions enforced by Deno itself.
    #[default]
    Deno,
    /// Any executable speaking the protocol on stdin/stdout. Not sandboxed:
    /// the install screen says so.
    Exec { command: Vec<String> },
}

/// Capabilities an extension asks for. Anything not listed is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Permissions {
    /// Hosts the extension may reach (`api.github.com`, `*` for any).
    pub net: Vec<String>,
    /// Programs the extension may spawn (`pactl`, `systemctl`).
    pub run: Vec<String>,
    /// Paths readable beyond the extension's own directory. `~` is expanded.
    pub read: Vec<String>,
    /// Paths writable. `~` is expanded.
    pub write: Vec<String>,
    /// Environment variables readable.
    pub env: Vec<String>,
    /// Read the clipboard (writing it is always allowed).
    #[serde(rename = "clipboard-read")]
    pub clipboard_read: bool,
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Human readable summary, shown before installing or enabling an extension.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        let join = |v: &[String]| v.join(", ");
        if !self.net.is_empty() {
            out.push(if self.net.iter().any(|h| h == "*") {
                "Access any website".to_string()
            } else {
                format!("Access {}", join(&self.net))
            });
        }
        if !self.run.is_empty() {
            out.push(format!("Run {}", join(&self.run)));
        }
        if !self.read.is_empty() {
            out.push(format!("Read {}", join(&self.read)));
        }
        if !self.write.is_empty() {
            out.push(format!("Write {}", join(&self.write)));
        }
        if !self.env.is_empty() {
            out.push(format!("Read environment variables {}", join(&self.env)));
        }
        if self.clipboard_read {
            out.push("Read the clipboard".into());
        }
        out
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandMode {
    /// Opens a view (list, detail or form).
    #[default]
    View,
    /// Runs, optionally shows a message, and closes the launcher.
    NoView,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub mode: CommandMode,
    /// Source file, relative to the extension directory. Defaults to
    /// `commands/<name>.ts`.
    #[serde(default)]
    pub entry: Option<PathBuf>,
    #[serde(default, rename = "arguments")]
    pub arguments: Vec<Argument>,
}

impl CommandDef {
    pub fn entry_path(&self) -> PathBuf {
        self.entry.clone().unwrap_or_else(|| PathBuf::from(format!("commands/{}.ts", self.name)))
    }
}

/// Typed inline argument, filled right in the search bar before running a command.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub name: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "type")]
    pub kind: ArgumentKind,
    /// Bounds for `number` arguments.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// Allowed values for `choice` arguments.
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgumentKind {
    #[default]
    Text,
    Number,
    Choice,
}

impl Argument {
    /// Checks a raw value typed by the user and converts it to JSON.
    pub fn parse(&self, raw: &str) -> Result<serde_json::Value, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return if self.required { Err(format!("{} is required", self.name)) } else { Ok(serde_json::Value::Null) };
        }
        match self.kind {
            ArgumentKind::Text => Ok(raw.into()),
            ArgumentKind::Number => {
                let n: f64 = raw.parse().map_err(|_| format!("{} must be a number", self.name))?;
                if self.min.is_some_and(|min| n < min) || self.max.is_some_and(|max| n > max) {
                    return Err(format!(
                        "{} must be between {} and {}",
                        self.name,
                        self.min.map_or("-∞".into(), |v| v.to_string()),
                        self.max.map_or("∞".into(), |v| v.to_string())
                    ));
                }
                Ok(serde_json::json!(n))
            }
            ArgumentKind::Choice => self
                .options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(raw))
                .map(|o| o.clone().into())
                .ok_or_else(|| format!("{} must be one of {}", self.name, self.options.join(", "))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default, rename = "type")]
    pub kind: PreferenceKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PreferenceKind {
    #[default]
    Text,
    /// Stored outside the plain config file.
    Secret,
    Boolean,
    Number,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let manifest: Manifest = toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        validate_id("extension id", &self.extension.id)?;
        let mut seen = std::collections::HashSet::new();
        for command in &self.commands {
            validate_id("command name", &command.name)?;
            if !seen.insert(&command.name) {
                bail!("duplicate command `{}`", command.name);
            }
            if command.entry_path().is_absolute()
                || command.entry_path().components().any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!("command `{}`: entry must stay inside the extension", command.name);
            }
        }
        Ok(())
    }

    pub fn command(&self, name: &str) -> Option<&CommandDef> {
        self.commands.iter().find(|c| c.name == name)
    }
}

fn validate_id(what: &str, id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-');
    if !ok {
        bail!("{what} `{id}` must match [a-z0-9-]+");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPM: &str = r#"
        [extension]
        id = "npm"
        title = "npm"
        description = "Search the npm registry"

        [permissions]
        net = ["registry.npmjs.org"]

        [[commands]]
        name = "search"
        title = "Search npm Packages"
        keywords = ["node", "package"]

        [[commands]]
        name = "set-volume"
        title = "Set Volume"
        mode = "no-view"
        [[commands.arguments]]
        name = "level"
        type = "number"
        max = 100
        required = true

        [[preferences]]
        name = "token"
        title = "Token"
        type = "secret"
    "#;

    #[test]
    fn parses_full_manifest() {
        let m: Manifest = toml::from_str(NPM).unwrap();
        m.validate().unwrap();
        assert_eq!(m.runtime, Runtime::Deno);
        assert_eq!(m.commands[0].entry_path(), PathBuf::from("commands/search.ts"));
        assert_eq!(m.commands[1].mode, CommandMode::NoView);
        let level = &m.commands[1].arguments[0];
        assert_eq!(level.kind, ArgumentKind::Number);
        assert_eq!(level.parse("40").unwrap(), serde_json::json!(40.0));
        assert!(level.parse("140").is_err());
        assert!(level.parse("loud").is_err());
        assert!(level.parse("").is_err());
        assert_eq!(m.preferences[0].kind, PreferenceKind::Secret);
        assert_eq!(m.permissions.describe(), vec!["Access registry.npmjs.org"]);
    }

    #[test]
    fn rejects_bad_ids_and_escaping_entries() {
        let bad = NPM.replace("id = \"npm\"", "id = \"Npm Tools\"");
        assert!(toml::from_str::<Manifest>(&bad).unwrap().validate().is_err());

        let escape = NPM.replace("name = \"search\"", "name = \"search\"\nentry = \"../../etc/passwd\"");
        assert!(toml::from_str::<Manifest>(&escape).unwrap().validate().is_err());
    }

    #[test]
    fn rejects_unknown_permission() {
        let typo = NPM.replace("net = [", "network = [");
        assert!(toml::from_str::<Manifest>(&typo).is_err());
    }
}
