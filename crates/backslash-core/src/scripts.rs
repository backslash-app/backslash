//! Script commands: any executable in `~/.config/backslash/scripts` whose
//! first comment lines describe it becomes a command. No SDK, any language.
//!
//! ```sh
//! #!/bin/bash
//! # title: Restart Wi-Fi
//! # description: Turns Wi-Fi off and on again
//! # icon: 📶
//! # mode: silent
//! # keywords: network, wlan
//! nmcli radio wifi off && nmcli radio wifi on
//! ```
//!
//! Keys, in the first block of comments (`#`, `//` or `--`):
//! `title` (required), `description`, `icon`, `keywords`, `confirm`,
//! `mode` (`silent`, `output`, `list` or `paste`) and `argument`
//! (repeatable; `name` or `name?` when optional, then a placeholder).
//! Arguments are passed to the script in order.

use std::path::{Path, PathBuf};

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Runs in the background; you hear about it only if it fails.
    #[default]
    Silent,
    /// Shows what it prints.
    Output,
    /// Each line it prints becomes an item to copy or paste.
    List,
    /// Pastes what it prints into the app you were in.
    Paste,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptArgument {
    pub name: String,
    pub placeholder: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// The file name: stable, used for aliases and usage history.
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub keywords: Vec<String>,
    pub mode: Mode,
    pub arguments: Vec<ScriptArgument>,
    /// Asked before running.
    pub confirm: Option<String>,
}

/// The default folder for script commands.
pub fn dir() -> PathBuf {
    paths::config_dir().join("scripts")
}

/// Reads the metadata of a script; `None` when it has no `title`.
pub fn parse(id: &str, path: &Path, text: &str) -> Option<Script> {
    let mut script = Script {
        id: id.to_string(),
        path: path.to_path_buf(),
        title: String::new(),
        description: None,
        icon: None,
        keywords: Vec::new(),
        mode: Mode::default(),
        arguments: Vec::new(),
        confirm: None,
    };
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if i == 0 && line.starts_with("#!") {
            continue;
        }
        let Some(comment) = ["#", "//", "--"].iter().find_map(|m| line.strip_prefix(m)) else {
            if line.is_empty() {
                continue;
            }
            break; // metadata stops at the first line of code
        };
        let Some((key, value)) = comment.split_once(':') else { continue };
        let (key, value) = (key.trim().to_lowercase(), value.trim());
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "title" => script.title = value.to_string(),
            "description" => script.description = Some(value.to_string()),
            "icon" => script.icon = Some(value.to_string()),
            "keywords" => script.keywords = value.split(',').map(|k| k.trim().to_string()).collect(),
            "confirm" => script.confirm = Some(value.to_string()),
            "mode" => {
                script.mode = match value.to_lowercase().as_str() {
                    "output" => Mode::Output,
                    "list" => Mode::List,
                    "paste" => Mode::Paste,
                    _ => Mode::Silent,
                }
            }
            "argument" => {
                let (name, placeholder) = match value.split_once(char::is_whitespace) {
                    Some((n, p)) => (n, Some(p.trim().to_string())),
                    None => (value, None),
                };
                let required = !name.ends_with('?');
                script.arguments.push(ScriptArgument {
                    name: name.trim_end_matches('?').to_string(),
                    placeholder,
                    required,
                });
            }
            _ => {}
        }
    }
    (!script.title.is_empty()).then_some(script)
}

/// Script commands found in `dirs`, sorted by title. Files without a title
/// are not commands (helpers, notes) and are skipped.
pub fn discover(dirs: &[PathBuf]) -> Vec<Script> {
    let mut scripts: Vec<Script> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file() || t.is_symlink()))
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            // Only the head is needed, and scripts can be large.
            let text = read_head(&path)?;
            parse(&name, &path, &text)
        })
        .collect();
    scripts.sort_by_key(|s| s.title.to_lowercase());
    scripts
}

fn read_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0; 8192];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

/// Whether the file can run as is.
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

/// A new script from a template, made executable. Returns its path.
pub fn create(dir: &Path, title: &str, mode: Mode) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let mut path = dir.join(format!("{}.sh", if slug.is_empty() { "script" } else { &slug }));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{slug}-{n}.sh"));
        n += 1;
    }
    let (mode_name, body) = match mode {
        Mode::Silent => ("silent", "notify-send \"Hello from Backslash\""),
        Mode::Output => ("output", "uptime\ndf -h /"),
        Mode::List => ("list", "ls ~/Downloads"),
        Mode::Paste => ("paste", "date +%F"),
    };
    let text = format!(
        "#!/bin/bash\n# title: {title}\n# description: What it does\n# icon: 💻\n# mode: {mode_name}\n# keywords: \n#\n# Modes: silent (runs quietly), output (shows what it prints),\n# list (one item per line), paste (pastes what it prints).\n# Arguments: \"# argument: name placeholder\", or \"name?\" when optional;\n# they arrive as $1, $2 and so on.\n\n{body}\n"
    );
    std::fs::write(&path, text)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata() {
        let s = parse(
            "wifi.sh",
            Path::new("/s/wifi.sh"),
            "#!/bin/bash\n# title: Restart Wi-Fi\n# icon: 📶\n# mode: output\n# keywords: net, wlan\n# argument: delay? seconds to wait\n# argument: name\n\nnmcli radio wifi off\n# title: ignored after code",
        )
        .unwrap();
        assert_eq!(s.title, "Restart Wi-Fi");
        assert_eq!(s.mode, Mode::Output);
        assert_eq!(s.keywords, ["net", "wlan"]);
        assert_eq!(
            s.arguments,
            [
                ScriptArgument { name: "delay".into(), placeholder: Some("seconds to wait".into()), required: false },
                ScriptArgument { name: "name".into(), placeholder: None, required: true },
            ]
        );
    }

    #[test]
    fn other_comment_styles_and_no_title() {
        let js = parse("a.js", Path::new("a.js"), "#!/usr/bin/env node\n// title: Hello\n// mode: paste\n").unwrap();
        assert_eq!((js.title.as_str(), js.mode), ("Hello", Mode::Paste));
        assert!(parse("helper.sh", Path::new("h"), "#!/bin/sh\necho hi\n").is_none());
    }

    #[test]
    fn create_and_discover() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), "Say Hello!", Mode::Output).unwrap();
        assert_eq!(path.file_name().unwrap(), "say-hello.sh");
        assert!(is_executable(&path));
        assert_eq!(create(dir.path(), "Say Hello!", Mode::Output).unwrap().file_name().unwrap(), "say-hello-2.sh");
        std::fs::write(dir.path().join("notes.txt"), "just notes").unwrap();
        let found = discover(&[dir.path().to_path_buf()]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].mode, Mode::Output);
    }
}
