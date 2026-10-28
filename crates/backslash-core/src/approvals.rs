//! Which extensions the user allowed to run, and with which permissions.
//!
//! The launcher asks before the first run of an extension, and again when an
//! update asks for more permissions than what was approved.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::manifest::{CommandMode, Manifest, Runtime};
use crate::{paths, store};

fn path() -> PathBuf {
    paths::state_dir().join("approved.json")
}

/// What the user agrees to, one readable line per permission, so an update
/// adding one host shows exactly that host.
pub fn requested(manifest: &Manifest) -> Vec<String> {
    let p = &manifest.permissions;
    let mut lines = Vec::new();
    // Bubblewrap confines files and the environment, not hosts or programs.
    let bubblewrap = !matches!(manifest.runtime, Runtime::Deno);
    if bubblewrap {
        lines.push("Run its own program, in a sandbox that lets it start any installed program".to_string());
        if !p.net.is_empty() {
            lines.push("Access any website".into());
        }
    } else {
        for host in &p.net {
            lines.push(if host == "*" { "Access any website".into() } else { format!("Access {host}") });
        }
        lines.extend(p.run.iter().map(|x| format!("Run {x}")));
    }
    lines.extend(p.read.iter().map(|x| format!("Read {x}")));
    lines.extend(p.write.iter().map(|x| format!("Write {x}")));
    lines.extend(p.env.iter().map(|x| format!("Read the environment variable {x}")));
    if p.clipboard_read {
        lines.push("Read the clipboard".into());
    }
    // Running without being asked is something to agree to as well.
    for command in &manifest.commands {
        let every = command.interval_text();
        match (command.mode, every) {
            (CommandMode::MenuBar, Some(every)) => {
                lines.push(format!("Show “{}” in the system tray, updated {every}", command.title))
            }
            (CommandMode::MenuBar, None) => lines.push(format!("Show “{}” in the system tray", command.title)),
            (_, Some(every)) => lines.push(format!("Run “{}” in the background {every}", command.title)),
            _ => {}
        }
    }
    lines
}

pub fn is_approved(manifest: &Manifest) -> bool {
    let approved: BTreeMap<String, Vec<String>> = store::read_json(&path());
    let Some(granted) = approved.get(&manifest.extension.id) else { return false };
    requested(manifest).iter().all(|line| granted.contains(line))
}

pub fn approve(manifest: &Manifest) -> anyhow::Result<()> {
    let mut approved: BTreeMap<String, Vec<String>> = store::read_json(&path());
    approved.insert(manifest.extension.id.clone(), requested(manifest));
    store::write_json(&path(), &approved)
}

/// Extensions the user approved at some point. An installed extension that
/// is missing from disk but still listed here was lost, not uninstalled.
pub fn approved_ids() -> Vec<String> {
    let approved: BTreeMap<String, Vec<String>> = store::read_json(&path());
    approved.into_keys().collect()
}

/// Forgets an extension's approval (when it is uninstalled on purpose).
pub fn forget(id: &str) -> anyhow::Result<()> {
    let mut approved: BTreeMap<String, Vec<String>> = store::read_json(&path());
    if approved.remove(id).is_some() {
        store::write_json(&path(), &approved)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(net: &[&str]) -> Manifest {
        let net = net.iter().map(|h| format!("\"{h}\"")).collect::<Vec<_>>().join(", ");
        toml::from_str(&format!("[extension]\nid = \"gh\"\ntitle = \"GitHub\"\n[permissions]\nnet = [{net}]\n"))
            .unwrap()
    }

    #[test]
    fn approval_is_invalidated_when_permissions_grow() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", dir.path());
        let v1 = manifest(&["api.github.com"]);
        assert!(!is_approved(&v1));
        approve(&v1).unwrap();
        assert!(is_approved(&v1));
        assert!(is_approved(&manifest(&[])), "fewer permissions stay approved");
        assert!(!is_approved(&manifest(&["api.github.com", "evil.example"])));
    }

    #[test]
    fn running_on_its_own_is_asked() {
        let m: Manifest = toml::from_str(
            "[extension]\nid = \"gh\"\ntitle = \"GitHub\"\n\
             [[commands]]\nname = \"inbox\"\ntitle = \"Inbox\"\nmode = \"menu-bar\"\ninterval = \"5m\"\n\
             [[commands]]\nname = \"sync\"\ntitle = \"Sync\"\nmode = \"no-view\"\ninterval = \"1h\"\n\
             [[commands]]\nname = \"tray\"\ntitle = \"Tray\"\nmode = \"menu-bar\"\n",
        )
        .unwrap();
        assert_eq!(
            requested(&m),
            [
                "Show “Inbox” in the system tray, updated every 5 minutes",
                "Run “Sync” in the background every hour",
                "Show “Tray” in the system tray",
            ]
        );
    }

    #[test]
    fn bubblewrap_lines_say_what_it_cannot_limit() {
        let m: Manifest = toml::from_str(
            "[extension]\nid = \"py\"\ntitle = \"Py\"\n[runtime]\nkind = \"exec\"\ncommand = [\"python3\"]\n\
             [permissions]\nnet = [\"a.example\", \"b.example\"]\nrun = [\"git\"]\nread = [\"~/Notes\"]\n",
        )
        .unwrap();
        assert_eq!(
            requested(&m),
            [
                "Run its own program, in a sandbox that lets it start any installed program",
                "Access any website",
                "Read ~/Notes",
            ]
        );
    }
}
