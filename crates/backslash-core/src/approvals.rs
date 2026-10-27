//! Which extensions the user allowed to run, and with which permissions.
//!
//! The launcher asks before the first run of an extension, and again when an
//! update asks for more permissions than what was approved.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::manifest::{Manifest, Runtime};
use crate::{paths, store};

fn path() -> PathBuf {
    paths::state_dir().join("approved.json")
}

/// What the user agrees to, one readable line per permission, so an update
/// adding one host shows exactly that host.
pub fn requested(manifest: &Manifest) -> Vec<String> {
    let p = &manifest.permissions;
    let mut lines = Vec::new();
    if matches!(manifest.runtime, Runtime::Exec { .. }) {
        lines.push("Run as a regular program, without sandbox".to_string());
    }
    for host in &p.net {
        lines.push(if host == "*" { "Access any website".into() } else { format!("Access {host}") });
    }
    lines.extend(p.run.iter().map(|x| format!("Run {x}")));
    lines.extend(p.read.iter().map(|x| format!("Read {x}")));
    lines.extend(p.write.iter().map(|x| format!("Write {x}")));
    lines.extend(p.env.iter().map(|x| format!("Read the environment variable {x}")));
    if p.clipboard_read {
        lines.push("Read the clipboard".into());
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
}
