//! The extension store: browsing the registry, installing, updating.

use backslash_core::catalog::{self, Entry, Locations};
use backslash_core::protocol::Accessory;

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
}

pub enum Status {
    Available,
    Installed,
    Update,
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
    let mut rows: Vec<(u8, Row)> = s
        .catalog
        .entries
        .iter()
        .map(|e| {
            let status = status(&loc, e);
            let busy = s.catalog.busy.as_deref() == Some(e.id.as_str());
            let (order, section, tag, color) = match status {
                Status::Update => (0, "Updates", "Update", "orange"),
                Status::Installed => (1, "Installed", "Installed", "green"),
                Status::Available => (2, "Available", "", "gray"),
            };
            let mut actions = Vec::new();
            match status {
                Status::Available => {
                    actions.push(UiAction::new("Install…", Op::ReviewInstall(e.id.clone())).icon("list-add"))
                }
                Status::Update => {
                    actions.push(UiAction::new("Update…", Op::ReviewInstall(e.id.clone())).icon("view-refresh"))
                }
                Status::Installed => {}
            }
            let page = repository_page(e);
            actions.push(UiAction::new("Open Source Code", Op::Open(page.clone())).icon("web-browser"));
            if !matches!(status, Status::Available) {
                actions.push(
                    UiAction::new("Uninstall", Op::UninstallEntry(e.id.clone()))
                        .icon("edit-delete")
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
            let icon = Icon::Spec(e.icon.clone().unwrap_or_else(|| "application-x-addon".into()));
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
                    title: e.title.clone(),
                    subtitle: Some(e.description.clone()).filter(|d| !d.is_empty()),
                    facts: vec![
                        ("Author".into(), if e.author.is_empty() { "Unknown".into() } else { e.author.clone() }),
                        ("Version".into(), e.version.clone()),
                        ("Commands".into(), e.commands.join("\n")),
                        ("Permissions".into(), permissions(e).join("\n")),
                        ("Source".into(), short_source(e)),
                        ("Reviewed at".into(), e.commit.chars().take(12).collect()),
                    ],
                    body: None,
                }),
                ..Default::default()
            };
            (order, row)
        })
        .collect();
    rows.sort_by_key(|(order, r)| (*order, r.title.to_lowercase()));
    rows.into_iter().map(|(_, r)| r).collect()
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
