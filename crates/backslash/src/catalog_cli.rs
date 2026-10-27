//! `backslash install | uninstall | update | outdated | search | publish`.

use std::io::IsTerminal;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use backslash_core::approvals;
use backslash_core::catalog::{self, Entry, Locations, Revision, Update};
use backslash_core::config::Config;
use backslash_core::manifest::Manifest;

/// The registry is refreshed when older than this.
const MAX_AGE: Duration = Duration::from_secs(3600);

fn locations() -> Locations {
    Locations::user(&Config::load())
}

fn ensure_synced(loc: &Locations, force: bool) -> Result<()> {
    if force || catalog::age(loc).is_none_or(|age| age > MAX_AGE) {
        eprintln!("Updating the registry from {}…", loc.registry);
        if let Err(e) = catalog::sync(loc) {
            if catalog::entries(loc).is_empty() {
                return Err(e);
            }
            eprintln!("warning: could not update the registry ({e:#}); using the cached copy");
        }
    }
    Ok(())
}

/// Shows what is asked and waits for consent (or `--yes`).
pub fn confirm(question: &str, lines: &[String], yes: bool) -> Result<()> {
    eprintln!("{question}");
    for line in lines {
        eprintln!("  - {line}");
    }
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!("confirmation needed: run again with --yes");
    }
    eprint!("Continue? [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes" | "o" | "oui") {
        bail!("cancelled");
    }
    Ok(())
}

fn is_git_source(target: &str) -> bool {
    target.contains("://") || target.starts_with("git@") || target.starts_with('/') || target.starts_with('.')
}

fn permission_lines(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        vec!["No permissions (fully sandboxed)".into()]
    } else {
        lines
    }
}

pub fn install(target: &str, rev: Option<String>, path: Option<String>, yes: bool) -> Result<()> {
    let loc = locations();
    if is_git_source(target) {
        let revision = match rev.clone() {
            Some(r) if r.len() == 40 && r.bytes().all(|b| b.is_ascii_hexdigit()) => Revision::Commit(r),
            other => Revision::Reference(other),
        };
        let path = path.unwrap_or_default();
        eprintln!("Fetching {target}…");
        let fetched = catalog::fetch(&loc, target, &path, &revision)?;
        let m = &fetched.manifest;
        confirm(
            &format!(
                "Install {} {} from {target}?\nIt is NOT reviewed by the registry. It will be allowed to:",
                m.extension.title, m.extension.version
            ),
            &permission_lines(approvals::requested(m)),
            yes,
        )?;
        let reference = match revision {
            Revision::Reference(r) => r,
            Revision::Commit(_) => None,
        };
        let record = catalog::record(target, &path, &fetched, reference, false);
        let dir = catalog::install(&loc, &fetched, record)?;
        approvals::approve(m)?;
        println!("Installed {} {} in {}", m.extension.id, m.extension.version, dir.display());
        return Ok(());
    }

    ensure_synced(&loc, false)?;
    let entry = catalog::find(&loc, target)
        .ok_or_else(|| anyhow!("no extension `{target}` in the registry (try `backslash search`)"))?;
    if catalog::installed(&loc, target).is_some_and(|r| r.commit == entry.commit) {
        println!("{} {} is already installed and up to date", entry.id, entry.version);
        return Ok(());
    }
    confirm(
        &format!("Install {} {} by {}? It will be allowed to:", entry.title, entry.version, author(&entry)),
        &permission_lines(entry.permissions.clone()),
        yes,
    )?;
    eprintln!("Fetching {}…", entry.repository);
    let (dir, manifest) = catalog::install_entry(&loc, &entry)?;
    approvals::approve(&manifest)?;
    println!("Installed {} {} in {}", entry.id, entry.version, dir.display());
    Ok(())
}

fn author(entry: &Entry) -> &str {
    if entry.author.is_empty() {
        "an unknown author"
    } else {
        &entry.author
    }
}

pub fn uninstall(id: &str) -> Result<()> {
    catalog::uninstall(&locations(), id)?;
    println!("Uninstalled {id}");
    Ok(())
}

/// User-installed extensions that Backslash can update.
fn managed(loc: &Locations) -> Vec<(String, Manifest)> {
    let Ok(dirs) = std::fs::read_dir(&loc.extensions) else { return Vec::new() };
    let mut out: Vec<(String, Manifest)> = dirs
        .flatten()
        .filter_map(|d| {
            let id = d.file_name().to_string_lossy().to_string();
            catalog::installed(loc, &id)?;
            Some((id, Manifest::load(&d.path()).ok()?))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn outdated() -> Result<()> {
    let loc = locations();
    ensure_synced(&loc, true)?;
    let mut any = false;
    for (id, manifest) in managed(&loc) {
        match catalog::check_update(&loc, &id, &manifest) {
            Ok(Update::UpToDate) => {}
            Ok(Update::Registry { entry, new_permissions }) => {
                any = true;
                let more = if new_permissions.is_empty() { String::new() } else { "  (asks for more)".into() };
                println!("{id} {} → {}{more}", manifest.extension.version, entry.version);
            }
            Ok(Update::Git { commit }) => {
                any = true;
                println!("{id} {} → {}", manifest.extension.version, &commit[..12.min(commit.len())]);
            }
            Err(e) => eprintln!("{id}: {e:#}"),
        }
    }
    if !any {
        println!("Everything is up to date");
    }
    Ok(())
}

pub fn update(only: Option<&str>, yes: bool) -> Result<()> {
    let loc = locations();
    ensure_synced(&loc, true)?;
    let mut updated = 0;
    for (id, manifest) in managed(&loc) {
        if only.is_some_and(|o| o != id) {
            continue;
        }
        match catalog::check_update(&loc, &id, &manifest)? {
            Update::UpToDate => {}
            Update::Registry { entry, new_permissions } => {
                if !new_permissions.is_empty() {
                    confirm(&format!("{} {} also asks to:", entry.title, entry.version), &new_permissions, yes)?;
                }
                let (_, manifest) = catalog::install_entry(&loc, &entry)?;
                approvals::approve(&manifest)?;
                println!("Updated {id} to {}", entry.version);
                updated += 1;
            }
            Update::Git { commit } => {
                let record = catalog::installed(&loc, &id).expect("checked by check_update");
                let fetched = catalog::fetch(&loc, &record.repository, &record.path, &Revision::Commit(commit))?;
                if !approvals::is_approved(&fetched.manifest) {
                    let had = approvals::requested(&manifest);
                    let new: Vec<String> =
                        approvals::requested(&fetched.manifest).into_iter().filter(|p| !had.contains(p)).collect();
                    confirm(&format!("{} also asks to:", fetched.manifest.extension.title), &new, yes)?;
                }
                let rec = catalog::record(&record.repository, &record.path, &fetched, record.reference.clone(), false);
                catalog::install(&loc, &fetched, rec)?;
                approvals::approve(&fetched.manifest)?;
                println!("Updated {id} to {}", fetched.manifest.extension.version);
                updated += 1;
            }
        }
    }
    if updated == 0 {
        println!("Everything is up to date");
    }
    Ok(())
}

pub fn search(query: Option<&str>) -> Result<()> {
    let loc = locations();
    ensure_synced(&loc, false)?;
    let q = query.unwrap_or_default().to_lowercase();
    let entries: Vec<Entry> = catalog::entries(&loc)
        .into_iter()
        .filter(|e| {
            q.is_empty()
                || [&e.id, &e.title, &e.description].iter().any(|t| t.to_lowercase().contains(&q))
                || e.commands.iter().any(|c| c.to_lowercase().contains(&q))
        })
        .collect();
    if entries.is_empty() {
        println!("No extension found");
    }
    for e in entries {
        let state = match catalog::installed(&loc, &e.id) {
            Some(r) if r.commit == e.commit => "  [installed]",
            Some(_) => "  [update available]",
            None => "",
        };
        println!("{} {} — {}: {}{state}", e.id, e.version, e.title, e.description);
    }
    Ok(())
}

pub fn publish(dir: Option<&Path>) -> Result<()> {
    let dir = dir.unwrap_or(Path::new("."));
    let loc = locations();
    let entry = catalog::publish_entry(&loc, dir)?;
    if !catalog::is_pushed(dir, &entry.commit) {
        eprintln!("warning: commit {} is not on any remote branch yet: push it first", &entry.commit[..12]);
    }
    if entry.repository.starts_with('/') {
        eprintln!("warning: this repository has no `origin` remote; the registry needs a public URL");
    }
    let file = format!("registry/{}.toml", entry.id);
    println!("# {file}\n{}", toml_entry(&entry)?);
    eprintln!();
    eprintln!("To publish, open a pull request adding {file} to {}", backslash_core::catalog::DEFAULT_REGISTRY);
    eprintln!("(the text above is the whole file). Reviewers check the code at this commit,");
    eprintln!("and Backslash refuses to install anything that differs from it.");
    Ok(())
}

fn toml_entry(entry: &Entry) -> Result<String> {
    Ok(backslash_core::toml::to_string(entry)?)
}
