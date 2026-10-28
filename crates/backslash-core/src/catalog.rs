//! The extension catalog: finding, installing, verifying and updating
//! extensions from the community registry or any git repository.
//!
//! The registry is a git repository holding one small file per extension,
//! `registry/<id>.toml`. Each entry points to the extension's own repository
//! (anywhere), pinned to a commit, with a checksum of the extension's files
//! and the permissions it asks for. Installing fetches that exact commit and
//! refuses anything that does not match: the registry is a curated, reviewed
//! index, not a host, so authors keep their code and reviews stay small.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, MANIFEST_FILE};
use crate::{approvals, paths, store};

pub const DEFAULT_REGISTRY: &str = "https://github.com/backslash-app/plugins";
const INSTALL_RECORD: &str = ".backslash-install.json";

/// One extension listed in the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub version: String,
    /// Git URL of the repository holding the extension.
    pub repository: String,
    /// Folder of the extension inside the repository ("" for the root).
    #[serde(default)]
    pub path: String,
    /// Full commit hash the entry was reviewed at.
    pub commit: String,
    /// `sha256:<hex>` of the extension's files (see [`checksum`]).
    pub checksum: String,
    /// What the extension asks for, as shown to users before installing.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Command titles, for the store.
    #[serde(default)]
    pub commands: Vec<String>,
}

/// Where things live; explicit so tests (and packagers) can redirect it.
#[derive(Debug, Clone)]
pub struct Locations {
    /// User extensions (`~/.local/share/backslash/extensions`).
    pub extensions: PathBuf,
    /// Local clone of the registry and download area.
    pub cache: PathBuf,
    /// Git URL of the registry.
    pub registry: String,
}

impl Locations {
    pub fn user(config: &crate::config::Config) -> Self {
        Self {
            extensions: paths::data_dir().join("extensions"),
            cache: paths::cache_dir(),
            registry: std::env::var("BACKSLASH_REGISTRY").unwrap_or_else(|_| config.registry.clone()),
        }
    }

    fn registry_dir(&self) -> PathBuf {
        self.cache.join("registry")
    }
}

/// How an installed extension was obtained, saved next to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub repository: String,
    pub path: String,
    pub commit: String,
    pub checksum: String,
    pub version: String,
    /// Branch or tag followed by updates for direct git installs.
    #[serde(default)]
    pub reference: Option<String>,
    /// Installed from the registry (verified) or from a git URL.
    pub from_registry: bool,
    pub installed: u64,
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "true");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().context("git is needed to install extensions")?;
    if !out.status.success() {
        bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Registry

/// Entries of the local registry clone (empty before the first [`sync`]).
pub fn entries(loc: &Locations) -> Vec<Entry> {
    let dir = loc.registry_dir().join("registry");
    let Ok(files) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut entries: Vec<Entry> = files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            match toml::from_str::<Entry>(&text) {
                Ok(e) if p.file_stem().is_some_and(|s| s == e.id.as_str()) => Some(e),
                Ok(e) => {
                    eprintln!("backslash: registry entry {} does not match its id `{}`", p.display(), e.id);
                    None
                }
                Err(err) => {
                    eprintln!("backslash: invalid registry entry {}: {err}", p.display());
                    None
                }
            }
        })
        .collect();
    entries.sort_by_key(|e| e.title.to_lowercase());
    entries
}

pub fn find(loc: &Locations, id: &str) -> Option<Entry> {
    entries(loc).into_iter().find(|e| e.id == id)
}

/// Time since the last successful sync.
pub fn age(loc: &Locations) -> Option<Duration> {
    let stamp = std::fs::metadata(loc.registry_dir().join(".git/backslash-synced"))
        .or_else(|_| std::fs::metadata(loc.registry_dir().join(".git/HEAD")))
        .ok()?;
    SystemTime::now().duration_since(stamp.modified().ok()?).ok()
}

/// What a [`sync`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Extensions in the registry now.
    pub total: usize,
    /// Ids that were not in the previous copy.
    pub added: Vec<String>,
    /// Ids whose entry changed (a new version or commit).
    pub changed: Vec<String>,
}

impl SyncReport {
    /// "143 extensions, 45 new", for a toast or the terminal.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} extension{}", self.total, if self.total == 1 { "" } else { "s" })];
        if !self.added.is_empty() {
            parts.push(format!("{} new", self.added.len()));
        }
        if !self.changed.is_empty() {
            parts.push(format!("{} updated", self.changed.len()));
        }
        if parts.len() == 1 {
            parts.push("up to date".into());
        }
        parts.join(", ")
    }
}

/// Brings the local registry copy to the latest commit of the registry (a
/// shallow clone: fetch, then reset to what was fetched). When the copy can't
/// be updated in place, it is cloned again. Errors are returned, never
/// swallowed: the caller decides whether the old copy is still worth showing.
pub fn sync(loc: &Locations) -> Result<SyncReport> {
    let before: BTreeMap<String, String> = entries(loc).into_iter().map(|e| (e.id, e.commit)).collect();
    let dir = loc.registry_dir();
    let updated = dir.join(".git").is_dir()
        && git(&["remote", "get-url", "origin"], Some(&dir)).is_ok_and(|remote| remote == loc.registry)
        // When it fails, a fresh clone below either fixes the copy or reports the error.
        && update_in_place(&dir).is_ok();
    if !updated {
        clone_fresh(loc)?;
    }
    stamp(&dir);
    let after = entries(loc);
    Ok(SyncReport {
        total: after.len(),
        added: after.iter().filter(|e| !before.contains_key(&e.id)).map(|e| e.id.clone()).collect(),
        changed: after
            .iter()
            .filter(|e| before.get(&e.id).is_some_and(|commit| commit != &e.commit))
            .map(|e| e.id.clone())
            .collect(),
    })
}

/// Fetches the remote's latest commit and moves the copy onto it.
fn update_in_place(dir: &Path) -> Result<()> {
    git(&["fetch", "--quiet", "--depth", "1", "origin", "HEAD"], Some(dir))?;
    let fetched = git(&["rev-parse", "FETCH_HEAD"], Some(dir))?;
    git(&["reset", "--quiet", "--hard", &fetched], Some(dir))?;
    git(&["clean", "--quiet", "-fd"], Some(dir))?;
    if git(&["rev-parse", "HEAD"], Some(dir))? != fetched {
        bail!("the registry copy did not move to the fetched commit");
    }
    Ok(())
}

/// Clones the registry next to the copy, then swaps them, so a failed clone
/// leaves the old copy in place.
fn clone_fresh(loc: &Locations) -> Result<()> {
    std::fs::create_dir_all(&loc.cache)?;
    let tmp = tempfile::Builder::new().prefix("registry-").tempdir_in(&loc.cache)?;
    let target = tmp.path().join("clone");
    git(&["clone", "--quiet", "--depth", "1", &loc.registry, &target.to_string_lossy()], None)?;
    let dir = loc.registry_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::rename(&target, &dir)?;
    Ok(())
}

fn stamp(dir: &Path) {
    let _ = std::fs::write(dir.join(".git/backslash-synced"), "");
}

// ---------------------------------------------------------------------------
// Fetching and verifying

/// `sha256:` of every regular file under `dir` (sorted relative paths and
/// contents), ignoring `.git`, `node_modules` and the install record.
/// Symbolic links are refused: they could point outside the extension.
pub fn checksum(dir: &Path) -> Result<String> {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".git" || name == "node_modules" || name == INSTALL_RECORD {
                continue;
            }
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_symlink() {
                bail!("{} is a symbolic link; extensions may not contain links", path.display());
            } else if kind.is_dir() {
                walk(root, &path, files)?;
            } else if kind.is_file() {
                files.push(path.strip_prefix(root)?.to_path_buf());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let mut names: Vec<String> = files.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    names.sort();
    let mut hasher = Sha256::new();
    for name in names {
        let content = std::fs::read(dir.join(&name))?;
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(content.len().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(&content);
    }
    let digest = hasher.finalize();
    Ok(format!("sha256:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

/// A downloaded extension, not installed yet.
pub struct Fetched {
    _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub commit: String,
    pub manifest: Manifest,
    pub checksum: String,
}

/// What to fetch: a pinned commit, or a branch/tag (`None` = default branch).
#[derive(Debug, Clone)]
pub enum Revision {
    Commit(String),
    Reference(Option<String>),
}

fn is_commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Clones `repository` at `revision` and reads the extension in `path`.
pub fn fetch(loc: &Locations, repository: &str, path: &str, revision: &Revision) -> Result<Fetched> {
    std::fs::create_dir_all(&loc.cache)?;
    let tmp = tempfile::Builder::new().prefix("fetch-").tempdir_in(&loc.cache)?;
    let clone = tmp.path().join("repo");
    let clone_s = clone.to_string_lossy().to_string();
    match revision {
        Revision::Reference(reference) => {
            let mut args = vec!["clone", "--quiet", "--depth", "1"];
            if let Some(r) = reference {
                args.extend(["--branch", r.as_str()]);
            }
            args.extend([repository, clone_s.as_str()]);
            git(&args, None)?;
        }
        Revision::Commit(commit) => {
            if !is_commit(commit) {
                bail!("`{commit}` is not a full commit hash");
            }
            // Fetching one commit is fast where servers allow it (GitHub,
            // GitLab…); otherwise fall back to a full clone.
            std::fs::create_dir_all(&clone)?;
            git(&["init", "--quiet"], Some(&clone))?;
            git(&["remote", "add", "origin", repository], Some(&clone))?;
            if git(&["fetch", "--quiet", "--depth", "1", "origin", commit], Some(&clone)).is_err() {
                git(&["fetch", "--quiet", "origin"], Some(&clone))?;
            }
            git(&["-c", "advice.detachedHead=false", "checkout", "--quiet", commit], Some(&clone))
                .with_context(|| format!("commit {commit} not found in {repository}"))?;
        }
    }
    let commit = git(&["rev-parse", "HEAD"], Some(&clone))?;
    let relative = Path::new(path.trim_matches('/'));
    if relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        bail!("invalid extension path `{path}`");
    }
    let dir = clone.join(relative);
    if !dir.join(MANIFEST_FILE).is_file() {
        bail!(
            "no {MANIFEST_FILE} in {repository}{}",
            if path.is_empty() { String::new() } else { format!(" ({path})") }
        );
    }
    let manifest = Manifest::load(&dir)?;
    let checksum = checksum(&dir)?;
    Ok(Fetched { _tmp: tmp, dir, commit, manifest, checksum })
}

/// Checks a download against its registry entry: same commit, files,
/// identity, version and permissions.
pub fn verify(entry: &Entry, fetched: &Fetched) -> Result<()> {
    let m = &fetched.manifest.extension;
    if fetched.commit != entry.commit {
        bail!("fetched commit {} instead of {}", fetched.commit, entry.commit);
    }
    if fetched.checksum != entry.checksum {
        bail!("checksum mismatch: the files differ from what the registry reviewed");
    }
    if m.id != entry.id || m.version != entry.version {
        bail!("the manifest says {} {}, the registry {} {}", m.id, m.version, entry.id, entry.version);
    }
    let asked: BTreeSet<String> = approvals::requested(&fetched.manifest).into_iter().collect();
    let listed: BTreeSet<String> = entry.permissions.iter().cloned().collect();
    if asked != listed {
        bail!("the extension asks for permissions the registry does not list");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Installing

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &to.join(&name))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(&name))?;
        }
    }
    Ok(())
}

/// Installs (or replaces) a fetched extension in the user's extensions folder.
pub fn install(loc: &Locations, fetched: &Fetched, record: InstallRecord) -> Result<PathBuf> {
    let id = &fetched.manifest.extension.id;
    std::fs::create_dir_all(&loc.extensions)?;
    let target = loc.extensions.join(id);
    let staging = loc.extensions.join(format!(".{id}.installing"));
    let backup = loc.extensions.join(format!(".{id}.previous"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&backup);
    copy_dir(&fetched.dir, &staging)?;
    store::write_json(&staging.join(INSTALL_RECORD), &record)?;
    // Swap atomically enough: a crash leaves either version in place.
    if target.exists() {
        std::fs::rename(&target, &backup)?;
    }
    if let Err(e) = std::fs::rename(&staging, &target) {
        let _ = std::fs::rename(&backup, &target);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(target)
}

pub fn record(
    source: &str,
    path: &str,
    fetched: &Fetched,
    reference: Option<String>,
    from_registry: bool,
) -> InstallRecord {
    InstallRecord {
        repository: source.to_string(),
        path: path.to_string(),
        commit: fetched.commit.clone(),
        checksum: fetched.checksum.clone(),
        version: fetched.manifest.extension.version.clone(),
        reference,
        from_registry,
        installed: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
    }
}

/// Fetches, verifies and installs a registry entry.
pub fn install_entry(loc: &Locations, entry: &Entry) -> Result<(PathBuf, Manifest)> {
    let fetched = fetch(loc, &entry.repository, &entry.path, &Revision::Commit(entry.commit.clone()))?;
    verify(entry, &fetched).with_context(|| format!("refusing to install {}", entry.id))?;
    let dir = install(loc, &fetched, record(&entry.repository, &entry.path, &fetched, None, true))?;
    Ok((dir, fetched.manifest))
}

/// The install record of a user-installed extension, if any.
pub fn installed(loc: &Locations, id: &str) -> Option<InstallRecord> {
    let path = loc.extensions.join(id).join(INSTALL_RECORD);
    path.is_file().then(|| store::read_json::<Option<InstallRecord>>(&path)).flatten()
}

/// Removes an extension from the user's folder (never system-wide ones).
pub fn uninstall(loc: &Locations, id: &str) -> Result<()> {
    if id.is_empty() || id.contains('/') || id.starts_with('.') {
        bail!("invalid extension id `{id}`");
    }
    let dir = loc.extensions.join(id);
    if !dir.join(MANIFEST_FILE).is_file() {
        bail!("{id} is not installed in {}", loc.extensions.display());
    }
    std::fs::remove_dir_all(&dir)?;
    // Uninstalled on purpose: it isn't offered back under "Previously Installed".
    crate::approvals::forget(id)?;
    Ok(())
}

/// What an update would bring.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    UpToDate,
    /// From the registry: the new entry, and the permissions it adds.
    Registry {
        entry: Box<Entry>,
        new_permissions: Vec<String>,
    },
    /// From a git install: the new commit of the followed branch.
    Git {
        commit: String,
    },
}

pub fn check_update(loc: &Locations, id: &str, current: &Manifest) -> Result<Update> {
    let record = installed(loc, id).ok_or_else(|| anyhow!("{id} was not installed with Backslash"))?;
    if record.from_registry {
        let Some(entry) = find(loc, id) else { return Ok(Update::UpToDate) };
        if entry.commit == record.commit {
            return Ok(Update::UpToDate);
        }
        let had: BTreeSet<String> = approvals::requested(current).into_iter().collect();
        let new_permissions = entry.permissions.iter().filter(|p| !had.contains(*p)).cloned().collect();
        return Ok(Update::Registry { entry: Box::new(entry), new_permissions });
    }
    let reference = record.reference.clone().unwrap_or_else(|| "HEAD".into());
    let remote = git(&["ls-remote", &record.repository, &reference], None)?;
    let commit = remote.split_whitespace().next().unwrap_or_default().to_string();
    if commit.is_empty() || commit == record.commit {
        Ok(Update::UpToDate)
    } else {
        Ok(Update::Git { commit })
    }
}

// ---------------------------------------------------------------------------
// Publishing

fn https_url(remote: &str) -> String {
    // git@github.com:owner/repo.git → https://github.com/owner/repo
    let url = match remote.strip_prefix("git@") {
        Some(rest) => format!("https://{}", rest.replacen(':', "/", 1)),
        None => remote.to_string(),
    };
    url.trim_end_matches(".git").to_string()
}

/// Builds the registry entry for the extension in `dir`, at the committed
/// `HEAD` of its repository. Fails on uncommitted changes.
pub fn publish_entry(loc: &Locations, dir: &Path) -> Result<Entry> {
    let dir = dir.canonicalize()?;
    let root =
        PathBuf::from(git(&["rev-parse", "--show-toplevel"], Some(&dir)).context("not inside a git repository")?);
    let path = dir.strip_prefix(&root).unwrap_or(Path::new("")).to_string_lossy().to_string();
    let dirty = git(&["status", "--porcelain", "--", "."], Some(&dir))?;
    if !dirty.is_empty() {
        bail!("commit your changes first:\n{dirty}");
    }
    let commit = git(&["rev-parse", "HEAD"], Some(&dir))?;
    let remote = git(&["remote", "get-url", "origin"], Some(&dir)).map(|r| https_url(&r)).unwrap_or_default();
    // Compute the checksum on a clean checkout, exactly as users will get it.
    let fetched = fetch(loc, &root.to_string_lossy(), &path, &Revision::Commit(commit.clone()))?;
    let m = &fetched.manifest;
    Ok(Entry {
        id: m.extension.id.clone(),
        title: m.extension.title.clone(),
        description: m.extension.description.clone(),
        author: m.extension.author.clone(),
        icon: m.extension.icon.clone(),
        version: m.extension.version.clone(),
        repository: if remote.is_empty() { root.to_string_lossy().to_string() } else { remote },
        path,
        commit,
        checksum: fetched.checksum.clone(),
        permissions: approvals::requested(m),
        commands: m.commands.iter().map(|c| c.title.clone()).collect(),
    })
}

/// Whether `commit` is on a branch of the `origin` remote (published).
pub fn is_pushed(dir: &Path, commit: &str) -> bool {
    git(&["branch", "-r", "--contains", commit], Some(dir)).is_ok_and(|out| !out.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    fn write_ext(dir: &Path, version: &str, net: &str) {
        std::fs::create_dir_all(dir.join("commands")).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "[extension]\nid = \"weather\"\ntitle = \"Weather\"\nversion = \"{version}\"\n\
                 [permissions]\nnet = [\"{net}\"]\n[[commands]]\nname = \"now\"\ntitle = \"Weather Now\"\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("commands/now.ts"), format!("// {version}\n")).unwrap();
    }

    struct World {
        _root: tempfile::TempDir,
        loc: Locations,
        author: PathBuf,
        registry: PathBuf,
    }

    /// An author repository with an extension in `ext/`, and a registry repo.
    fn world() -> World {
        let root = tempfile::tempdir().unwrap();
        let author = root.path().join("author");
        std::fs::create_dir_all(&author).unwrap();
        sh(&author, &["init", "-q", "-b", "main"]);
        write_ext(&author.join("ext"), "1.0.0", "api.weather.example");
        sh(&author, &["add", "."]);
        sh(&author, &["commit", "-qm", "v1"]);
        let registry = root.path().join("registry");
        std::fs::create_dir_all(registry.join("registry")).unwrap();
        sh(&registry, &["init", "-q", "-b", "main"]);
        std::fs::write(registry.join("README.md"), "registry").unwrap();
        sh(&registry, &["add", "."]);
        sh(&registry, &["commit", "-qm", "init"]);
        let loc = Locations {
            extensions: root.path().join("extensions"),
            cache: root.path().join("cache"),
            registry: registry.to_string_lossy().to_string(),
        };
        World { _root: root, loc, author, registry }
    }

    fn add_entry(w: &World, entry: &Entry) {
        std::fs::write(w.registry.join(format!("registry/{}.toml", entry.id)), toml::to_string(entry).unwrap())
            .unwrap();
        sh(&w.registry, &["add", "."]);
        sh(&w.registry, &["commit", "-qm", "entry"]);
    }

    #[test]
    fn publish_sync_install_update_uninstall() {
        let w = world();
        let entry = publish_entry(&w.loc, &w.author.join("ext")).unwrap();
        assert_eq!(entry.id, "weather");
        assert_eq!(entry.path, "ext");
        assert_eq!(entry.permissions, vec!["Access api.weather.example"]);
        assert_eq!(entry.commands, vec!["Weather Now"]);
        add_entry(&w, &entry);

        let report = sync(&w.loc).unwrap();
        assert_eq!(report.added, vec!["weather"]);
        assert_eq!(report.summary(), "1 extension, 1 new");
        assert_eq!(entries(&w.loc), vec![entry.clone()]);
        assert!(age(&w.loc).is_some());

        let (dir, manifest) = install_entry(&w.loc, &entry).unwrap();
        assert!(dir.join("commands/now.ts").is_file());
        let record = installed(&w.loc, "weather").unwrap();
        assert!(record.from_registry);
        assert_eq!(record.commit, entry.commit);
        // The install record does not change the checksum.
        assert_eq!(checksum(&dir).unwrap(), entry.checksum);
        assert_eq!(check_update(&w.loc, "weather", &manifest).unwrap(), Update::UpToDate);

        // v2 asks for one more host: the update says so.
        write_ext(&w.author.join("ext"), "2.0.0", "api.weather.example\", \"tracker.example");
        sh(&w.author, &["commit", "-qam", "v2"]);
        let v2 = publish_entry(&w.loc, &w.author.join("ext")).unwrap();
        add_entry(&w, &v2);
        let report = sync(&w.loc).unwrap();
        assert_eq!((report.added.len(), report.changed.clone()), (0, vec!["weather".to_string()]));
        assert_eq!(report.summary(), "1 extension, 1 updated");
        match check_update(&w.loc, "weather", &manifest).unwrap() {
            Update::Registry { entry, new_permissions } => {
                assert_eq!(entry.version, "2.0.0");
                assert_eq!(new_permissions, vec!["Access tracker.example"]);
            }
            other => panic!("expected an update, got {other:?}"),
        }
        install_entry(&w.loc, &v2).unwrap();
        assert_eq!(installed(&w.loc, "weather").unwrap().version, "2.0.0");

        uninstall(&w.loc, "weather").unwrap();
        assert!(installed(&w.loc, "weather").is_none());
        assert!(uninstall(&w.loc, "../etc").is_err());
    }

    #[test]
    fn sync_recovers_or_reports() {
        let w = world();
        let entry = publish_entry(&w.loc, &w.author.join("ext")).unwrap();
        add_entry(&w, &entry);
        sync(&w.loc).unwrap();
        assert_eq!(sync(&w.loc).unwrap().summary(), "1 extension, up to date");

        // A broken copy is cloned again rather than kept as it was.
        let copy = w.loc.registry_dir();
        std::fs::write(copy.join(".git/HEAD"), "garbage").unwrap();
        std::fs::write(w.registry.join("registry/extra.toml"), "not an entry").unwrap();
        sh(&w.registry, &["add", "."]);
        sh(&w.registry, &["commit", "-qm", "extra"]);
        sync(&w.loc).unwrap();
        assert!(copy.join("registry/extra.toml").is_file());

        // An unreachable registry is an error, and the copy stays usable.
        let gone = Locations { registry: w.loc.registry.clone() + "-missing", ..w.loc.clone() };
        assert!(sync(&gone).is_err());
        assert_eq!(entries(&w.loc).len(), 1);
    }

    #[test]
    fn refuses_tampering() {
        let w = world();
        let good = publish_entry(&w.loc, &w.author.join("ext")).unwrap();

        let mut wrong_sum = good.clone();
        wrong_sum.checksum = "sha256:00".into();
        assert!(install_entry(&w.loc, &wrong_sum).unwrap_err().to_string().contains("refusing"));

        let mut hidden_permission = good.clone();
        hidden_permission.permissions = vec![];
        assert!(install_entry(&w.loc, &hidden_permission).is_err());

        let mut bad_path = good.clone();
        bad_path.path = "../..".into();
        assert!(install_entry(&w.loc, &bad_path).is_err());

        assert!(!w.loc.extensions.join("weather").exists(), "nothing installed");
    }

    #[test]
    fn direct_git_install_and_dirty_publish() {
        let w = world();
        let fetched = fetch(&w.loc, &w.author.to_string_lossy(), "ext", &Revision::Reference(None)).unwrap();
        let rec = record(&w.author.to_string_lossy(), "ext", &fetched, None, false);
        install(&w.loc, &fetched, rec).unwrap();
        let manifest = Manifest::load(&w.loc.extensions.join("weather")).unwrap();
        assert_eq!(check_update(&w.loc, "weather", &manifest).unwrap(), Update::UpToDate);
        std::fs::write(w.author.join("ext/commands/now.ts"), "// changed\n").unwrap();
        assert!(publish_entry(&w.loc, &w.author.join("ext")).is_err(), "uncommitted changes");
        sh(&w.author, &["commit", "-qam", "change"]);
        assert!(matches!(check_update(&w.loc, "weather", &manifest).unwrap(), Update::Git { .. }));
    }

    #[test]
    fn urls() {
        assert_eq!(https_url("git@github.com:me/ext.git"), "https://github.com/me/ext");
        assert_eq!(https_url("https://gitlab.com/me/ext.git"), "https://gitlab.com/me/ext");
    }
}
