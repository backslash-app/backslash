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

/// What the user agrees to, as readable lines. Changes when permissions grow.
pub fn requested(manifest: &Manifest) -> Vec<String> {
    let mut lines = manifest.permissions.describe();
    if matches!(manifest.runtime, Runtime::Exec { .. }) {
        lines.insert(0, "Run as a regular program, without sandbox".into());
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
