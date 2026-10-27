//! Discovers installed extensions. A broken extension is reported, never fatal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::manifest::{CommandDef, Manifest, MANIFEST_FILE};
use crate::search::Searchable;

#[derive(Debug, Clone)]
pub struct Extension {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

#[derive(Debug, Default)]
pub struct Registry {
    pub extensions: Vec<Extension>,
    /// `(directory, error)` for extensions that failed to load.
    pub errors: Vec<(PathBuf, String)>,
}

/// A command as it appears in the launcher's root search.
#[derive(Debug, Clone)]
pub struct CommandEntry {
    /// `extension/command`, stable across versions; used for usage history and the CLI.
    pub id: String,
    pub extension: usize,
    pub title: String,
    pub subtitle: String,
    pub icon: Option<String>,
    pub keywords: Vec<String>,
}

impl Searchable for CommandEntry {
    fn title(&self) -> &str {
        &self.title
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
    fn subtitle(&self) -> Option<&str> {
        Some(&self.subtitle)
    }
}

impl Registry {
    /// Scans `dirs` in order; the first extension with a given id wins, so a
    /// local checkout shadows the installed copy.
    pub fn scan(dirs: &[PathBuf]) -> Self {
        let mut registry = Registry::default();
        let mut seen = HashSet::new();
        for root in dirs {
            let Ok(entries) = std::fs::read_dir(root) else { continue };
            let mut candidates: Vec<PathBuf> =
                entries.flatten().map(|e| e.path()).filter(|p| p.join(MANIFEST_FILE).is_file()).collect();
            candidates.sort();
            for dir in candidates {
                registry.add(dir, &mut seen);
            }
        }
        registry
    }

    /// Loads a single extension directory (used by `backslash dev <dir>`).
    pub fn add_dir(&mut self, dir: &Path) {
        let mut seen: HashSet<String> = self.extensions.iter().map(|e| e.manifest.extension.id.clone()).collect();
        // A dev directory takes precedence over an installed copy.
        if let Ok(m) = Manifest::load(dir) {
            self.extensions.retain(|e| e.manifest.extension.id != m.extension.id);
            seen.remove(&m.extension.id);
        }
        self.add(dir.to_path_buf(), &mut seen);
    }

    fn add(&mut self, dir: PathBuf, seen: &mut HashSet<String>) {
        match Manifest::load(&dir) {
            Ok(manifest) => {
                if seen.insert(manifest.extension.id.clone()) {
                    self.extensions.push(Extension { dir, manifest });
                }
            }
            Err(err) => self.errors.push((dir, format!("{err:#}"))),
        }
    }

    pub fn find(&self, id: &str) -> Option<&Extension> {
        self.extensions.iter().find(|e| e.manifest.extension.id == id)
    }

    /// Resolves `extension/command`.
    pub fn resolve(&self, id: &str) -> Option<(&Extension, &CommandDef)> {
        let (ext, cmd) = id.split_once('/')?;
        let ext = self.find(ext)?;
        Some((ext, ext.manifest.command(cmd)?))
    }

    pub fn commands(&self, enabled: impl Fn(&str) -> bool) -> Vec<CommandEntry> {
        self.extensions
            .iter()
            .enumerate()
            .filter(|(_, e)| enabled(&e.manifest.extension.id))
            .flat_map(|(i, ext)| {
                let meta = &ext.manifest.extension;
                ext.manifest.commands.iter().map(move |c| {
                    let mut keywords = c.keywords.clone();
                    keywords.push(meta.title.clone());
                    CommandEntry {
                        id: format!("{}/{}", meta.id, c.name),
                        extension: i,
                        title: c.title.clone(),
                        subtitle: meta.title.clone(),
                        icon: c.icon.clone().or_else(|| meta.icon.clone()),
                        keywords,
                    }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, id: &str, title: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "[extension]\nid = \"{id}\"\ntitle = \"{title}\"\n\n[[commands]]\nname = \"run\"\ntitle = \"Run {title}\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn first_directory_wins_and_errors_are_collected() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(&a.path().join("x"), "x", "Local X");
        write(&b.path().join("x"), "x", "Installed X");
        write(&b.path().join("y"), "y", "Y");
        std::fs::create_dir_all(b.path().join("broken")).unwrap();
        std::fs::write(b.path().join("broken").join(MANIFEST_FILE), "nope").unwrap();

        let r = Registry::scan(&[a.path().into(), b.path().into()]);
        assert_eq!(r.extensions.len(), 2);
        assert_eq!(r.find("x").unwrap().manifest.extension.title, "Local X");
        assert_eq!(r.errors.len(), 1);
        assert!(r.resolve("y/run").is_some());
        assert!(r.resolve("y/missing").is_none());

        let commands = r.commands(|id| id != "y");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].id, "x/run");
    }
}
