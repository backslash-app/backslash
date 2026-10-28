//! The extension store: browsing the registry, installing, updating.

use backslash_core::catalog::{self, Entry, Locations};
use backslash_core::protocol::{Accessory, MetadataEntry, Tag};

use super::model::{tint_for, Op, Preview, Row, UiAction};
use super::State;
use crate::icons::Icon;

/// Registry entries and what is going on with them.
#[derive(Default)]
pub struct CatalogState {
    pub entries: Vec<Entry>,
    pub syncing: bool,
    /// Id of the extension being installed or updated.
    pub busy: Option<String>,
    pub error: Option<String>,
    /// Installed extensions with a newer reviewed version.
    pub updates: Vec<String>,
}

/// Ids of installed registry extensions whose entry moved to another commit.
pub fn pending_updates(loc: &Locations, entries: &[Entry]) -> Vec<String> {
    entries.iter().filter(|e| matches!(status(loc, e), Status::Update)).map(|e| e.id.clone()).collect()
}

pub enum Status {
    Available,
    Installed,
    Update,
    /// Approved once, no longer on disk: lost (e.g. by the 0.5.0 tarball
    /// update), as uninstalling forgets the approval.
    Previous,
}

/// Registry extensions that were installed and approved, then lost.
pub fn previous_ids(s: &State) -> Vec<String> {
    let loc = Locations::user(&s.config);
    let approved = backslash_core::approvals::approved_ids();
    s.catalog
        .entries
        .iter()
        .filter(|e| approved.contains(&e.id) && catalog::installed(&loc, &e.id).is_none())
        .map(|e| e.id.clone())
        .collect()
}

pub fn status(loc: &Locations, entry: &Entry) -> Status {
    match catalog::installed(loc, &entry.id) {
        Some(r) if r.commit == entry.commit => Status::Installed,
        Some(_) => Status::Update,
        None => Status::Available,
    }
}

pub fn rows(s: &State) -> Vec<Row> {
    let loc = Locations::user(&s.config);
    let previous = previous_ids(s);
    let mut rows: Vec<(u8, Row)> = s
        .catalog
        .entries
        .iter()
        .map(|e| {
            let status = match status(&loc, e) {
                Status::Available if previous.contains(&e.id) => Status::Previous,
                other => other,
            };
            let busy = s.catalog.busy.as_deref() == Some(e.id.as_str());
            let (order, section, tag, color) = match status {
                Status::Update => (0, "Updates", "Update", "orange"),
                Status::Previous => (1, "Previously Installed", "Removed", "red"),
                Status::Installed => (2, "Installed", "Installed", "green"),
                Status::Available => (3, "Available", "", "gray"),
            };
            let mut actions = Vec::new();
            match status {
                Status::Available => {
                    actions.push(UiAction::new("Install…", Op::ReviewInstall(e.id.clone())).icon("bs-plus"))
                }
                Status::Update => {
                    actions.push(UiAction::new("Update…", Op::ReviewInstall(e.id.clone())).icon("bs-refresh"));
                    if s.catalog.updates.len() > 1 {
                        actions.push(UiAction::new("Update All", Op::UpdateAll).icon("bs-refresh"));
                    }
                }
                Status::Previous => {
                    // "Reinstall Previously Installed (N)" is in the action panel.
                    actions.push(UiAction::new("Reinstall…", Op::ReviewInstall(e.id.clone())).icon("bs-download"));
                }
                Status::Installed => {}
            }
            let page = repository_page(e);
            actions.push(UiAction::new("Open Source Code", Op::Open(page.clone())).icon("bs-external"));
            if !matches!(status, Status::Available | Status::Previous) {
                actions.push(
                    UiAction::new("Uninstall", Op::UninstallEntry(e.id.clone()))
                        .icon("bs-trash")
                        .destructive()
                        .confirm(&format!("Uninstall {}?", e.title)),
                );
            }
            let mut accessories = vec![Accessory { text: Some(format!("v{}", e.version)), ..Default::default() }];
            if busy {
                accessories.push(Accessory {
                    tag: Some("Installing…".into()),
                    color: Some("blue".into()),
                    ..Default::default()
                });
            } else if !tag.is_empty() {
                accessories.push(Accessory { tag: Some(tag.into()), color: Some(color.into()), ..Default::default() });
            }
            let icon = Icon::Spec(e.icon.clone().unwrap_or_else(|| "bs-package".into()));
            let row = Row {
                key: format!("store:{}", e.id),
                title: e.title.clone(),
                subtitle: Some(e.description.clone()).filter(|d| !d.is_empty()),
                icon: Some(icon.clone()),
                tint: Some(tint_for(&e.id)),
                accessories,
                section: Some(section.into()),
                actions,
                preview: Some(Preview {
                    icon: Some(icon),
                    tint: Some(tint_for(&e.id).into()),
                    title: e.title.clone(),
                    subtitle: Some(e.description.clone()).filter(|d| !d.is_empty()),
                    metadata: entry_metadata(e, &status),
                    ..Default::default()
                }),
                ..Default::default()
            };
            (order, row)
        })
        .collect();
    rows.sort_by_key(|(order, r)| (*order, r.title.to_lowercase()));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The side panel of a store entry: what it is, what it can do, where it comes from.
fn entry_metadata(e: &Entry, status: &Status) -> Vec<MetadataEntry> {
    let line = |label: &str, value: String| MetadataEntry { label: label.into(), value, ..Default::default() };
    let mut lines = vec![
        match status {
            Status::Installed => MetadataEntry {
                label: "Status".into(),
                value: "Installed".into(),
                icon: Some("bs-circle-check".into()),
                color: Some("green".into()),
                ..Default::default()
            },
            Status::Update => MetadataEntry {
                label: "Status".into(),
                value: "Update available".into(),
                icon: Some("bs-download".into()),
                color: Some("orange".into()),
                ..Default::default()
            },
            Status::Available => line("Status", "Not installed".into()),
            Status::Previous => MetadataEntry {
                label: "Status".into(),
                value: "Was installed, then removed: reinstall it".into(),
                icon: Some("bs-alert".into()),
                color: Some("red".into()),
                ..Default::default()
            },
        },
        line("Version", e.version.clone()),
        line("Author", if e.author.is_empty() { "Unknown".into() } else { e.author.clone() }),
        MetadataEntry {
            label: "Commands".into(),
            tags: e.commands.iter().map(|c| Tag { text: c.clone(), color: None }).collect(),
            ..Default::default()
        },
        MetadataEntry { separator: true, ..Default::default() },
    ];
    for (i, p) in permissions(e).into_iter().enumerate() {
        lines.push(MetadataEntry {
            label: if i == 0 { "Can".into() } else { String::new() },
            value: p,
            icon: Some(if e.permissions.is_empty() { "bs-shield" } else { "bs-check" }.into()),
            ..Default::default()
        });
    }
    lines.push(MetadataEntry { separator: true, ..Default::default() });
    lines.push(MetadataEntry {
        label: "Source".into(),
        value: short_source(e),
        link: Some(repository_page(e)),
        ..Default::default()
    });
    lines.push(line("Reviewed", e.commit.chars().take(12).collect()));
    lines
}

fn permissions(e: &Entry) -> Vec<String> {
    if e.permissions.is_empty() {
        vec!["None (fully sandboxed)".into()]
    } else {
        e.permissions.clone()
    }
}

/// `github.com/owner/repo/path`, for display.
fn short_source(e: &Entry) -> String {
    let repo = e.repository.trim_start_matches("https://").trim_end_matches(".git");
    if e.path.is_empty() {
        repo.to_string()
    } else {
        format!("{repo}/{}", e.path)
    }
}

/// A web page for the reviewed code, on the common forges.
fn repository_page(e: &Entry) -> String {
    let base = e.repository.trim_end_matches(".git");
    let forge =
        ["https://github.com/", "https://gitlab.com/", "https://codeberg.org/"].iter().any(|f| base.starts_with(f));
    if forge {
        let sep = if base.starts_with("https://gitlab.com/") { "/-/tree/" } else { "/tree/" };
        format!("{base}{sep}{}/{}", e.commit, e.path).trim_end_matches('/').to_string()
    } else {
        base.to_string()
    }
}

/// The install confirmation screen.
pub fn confirm_markdown(loc: &Locations, e: &Entry) -> String {
    let updating = catalog::installed(loc, &e.id).is_some();
    let mut md = format!(
        "# {} {}?\n\n{}\n\nby **{}** · version {}\n\n",
        if updating { "Update" } else { "Install" },
        e.title,
        e.description,
        if e.author.is_empty() { "unknown" } else { &e.author },
        e.version
    );
    if e.permissions.is_empty() {
        md.push_str("It needs **no permission**: it runs fully sandboxed.\n");
    } else {
        md.push_str("It will be allowed to:\n\n");
        for p in &e.permissions {
            md.push_str(&format!("- **{p}**\n"));
        }
        md.push_str("\nEverything else is blocked by the sandbox.\n");
    }
    if !e.commands.is_empty() {
        md.push_str("\nCommands:\n\n");
        for c in &e.commands {
            md.push_str(&format!("- {c}\n"));
        }
    }
    md.push_str(&format!(
        "\nThe code was reviewed at commit `{}`; Backslash refuses anything that differs.\n\n\
         Press **Enter** to {}, **Esc** to cancel.",
        e.commit.chars().take(12).collect::<String>(),
        if updating { "update" } else { "install" }
    ));
    md
}
