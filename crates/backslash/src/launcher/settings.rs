//! The Settings view and the forms built by the launcher (preferences,
//! quicklinks, snippets, shortcut), plus the extension approval screen.

use backslash_core::config::{Config, Quicklink, Snippet, Theme};
use backslash_core::manifest::{Manifest, PreferenceKind};
use backslash_core::protocol::{Accessory, Field, FieldKind, FormView};
use backslash_core::{approvals, secrets};
use serde_json::Value;

use super::model::{tint_for, Builtin, Op, Preview, Row, UiAction};
use super::State;
use crate::icons::Icon;

fn setting(key: &str, title: &str, value: &str, icon: &str, tint: &'static str, actions: Vec<UiAction>) -> Row {
    Row {
        key: format!("setting:{key}"),
        title: title.into(),
        subtitle: Some(value.into()),
        icon: Some(Icon::Spec(icon.into())),
        tint: Some(tint),
        section: Some("General".into()),
        actions,
        ..Default::default()
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "On"
    } else {
        "Off"
    }
}

pub fn rows(s: &State) -> Vec<Row> {
    let c = &s.config;
    let mut rows = vec![
        setting(
            "theme",
            "Theme",
            match c.theme {
                Theme::System => "Follow the system",
                Theme::Light => "Light",
                Theme::Dark => "Dark",
            },
            "preferences-desktop-appearance",
            "yellow",
            vec![UiAction::new("Change Theme", Op::OpenBuiltin(Builtin::Theme))],
        ),
        setting(
            "hotkey",
            "Global Shortcut",
            if c.hotkey.is_empty() { "Disabled" } else { &c.hotkey },
            "preferences-desktop-keyboard-shortcuts",
            "blue",
            vec![UiAction::new("Change Shortcut", Op::EditHotkey)],
        ),
        setting(
            "autostart",
            "Launch at Login",
            on_off(s.autostart),
            "system-run",
            "green",
            vec![UiAction::new(if s.autostart { "Turn Off" } else { "Turn On" }, Op::SetAutostart(!s.autostart))],
        ),
        setting(
            "clipboard",
            "Clipboard History",
            on_off(c.clipboard.enabled),
            "edit-paste",
            "purple",
            vec![UiAction::new(
                if c.clipboard.enabled { "Turn Off" } else { "Turn On" },
                Op::SetClipboard(!c.clipboard.enabled),
            )],
        ),
        setting(
            "config",
            "Config File",
            "Everything here is saved in config.toml",
            "text-x-generic",
            "gray",
            vec![UiAction::new("Open config.toml", Op::Open(Config::path().display().to_string()))],
        ),
    ];

    for ext in &s.registry.extensions {
        let m = &ext.manifest;
        let id = m.extension.id.clone();
        let enabled = c.is_enabled(&id);
        let commands = m.commands.len();
        let mut actions = Vec::new();
        if !m.preferences.is_empty() {
            actions.push(UiAction::new("Configure", Op::ConfigureExtension(id.clone())).icon("preferences-system"));
        }
        actions.push(
            UiAction::new(if enabled { "Disable" } else { "Enable" }, Op::ToggleExtension(id.clone()))
                .icon(if enabled { "list-remove" } else { "list-add" }),
        );
        actions.push(UiAction::new("Open Folder", Op::Open(ext.dir.display().to_string())).icon("folder-open"));
        let permissions = approvals::requested(m);
        let mut facts = vec![
            ("Version".to_string(), m.extension.version.clone()),
            ("Author".into(), if m.extension.author.is_empty() { "—".into() } else { m.extension.author.clone() }),
            ("Commands".into(), m.commands.iter().map(|c| c.title.clone()).collect::<Vec<_>>().join("\n")),
            ("Permissions".into(), if permissions.is_empty() { "None".into() } else { permissions.join("\n") }),
            ("Folder".into(), backslash_core::system::files::tilde(&ext.dir)),
        ];
        if !m.preferences.is_empty() {
            facts.push((
                "Preferences".into(),
                m.preferences.iter().map(|p| p.title.clone()).collect::<Vec<_>>().join(", "),
            ));
        }
        rows.push(Row {
            key: format!("ext:{id}"),
            title: m.extension.title.clone(),
            subtitle: Some(format!(
                "{} · {commands} command{}",
                if enabled { "Enabled" } else { "Disabled" },
                if commands == 1 { "" } else { "s" }
            )),
            icon: Some(Icon::Spec(m.extension.icon.clone().unwrap_or_else(|| "application-x-addon".into()))),
            tint: Some(if enabled { tint_for(&id) } else { "gray" }),
            accessories: vec![Accessory { tag: Some(format!("v{}", m.extension.version)), ..Default::default() }],
            section: Some("Extensions".into()),
            actions,
            preview: Some(Preview {
                icon: Some(Icon::Spec(m.extension.icon.clone().unwrap_or_else(|| "application-x-addon".into()))),
                title: m.extension.title.clone(),
                subtitle: Some(m.extension.description.clone()).filter(|d| !d.is_empty()),
                facts,
                body: None,
            }),
            ..Default::default()
        });
    }

    for (i, link) in c.quicklinks.iter().enumerate() {
        rows.push(Row {
            key: format!("ql:{i}"),
            title: link.name.clone(),
            subtitle: Some(link.url.clone()),
            icon: Some(Icon::Spec(link.icon.clone().unwrap_or_else(|| "web-browser".into()))),
            tint: Some("blue"),
            accessories: link
                .keyword
                .iter()
                .map(|k| Accessory { tag: Some(k.clone()), ..Default::default() })
                .collect(),
            section: Some("Quicklinks".into()),
            actions: vec![
                UiAction::new("Edit", Op::EditQuicklink(Some(i))).icon("document-edit"),
                UiAction::new("Delete", Op::DeleteQuicklink(i))
                    .icon("edit-delete")
                    .destructive()
                    .shortcut("<Ctrl>d")
                    .confirm(&format!("Delete “{}”?", link.name)),
            ],
            ..Default::default()
        });
    }
    rows.push(Row {
        key: "ql:new".into(),
        title: "Add Quicklink".into(),
        subtitle: Some("A URL, optionally with {query} and a keyword".into()),
        icon: Some(Icon::Spec("list-add".into())),
        tint: Some("blue"),
        section: Some("Quicklinks".into()),
        actions: vec![UiAction::new("Add Quicklink", Op::EditQuicklink(None))],
        ..Default::default()
    });

    for (i, snippet) in c.snippets.iter().enumerate() {
        rows.push(Row {
            key: format!("sn:{i}"),
            title: snippet.name.clone(),
            subtitle: Some(backslash_core::system::clipboard::summary(&snippet.text)),
            icon: Some(Icon::Spec("insert-text".into())),
            tint: Some("green"),
            accessories: snippet
                .keyword
                .iter()
                .map(|k| Accessory { tag: Some(k.clone()), ..Default::default() })
                .collect(),
            section: Some("Snippets".into()),
            actions: vec![
                UiAction::new("Edit", Op::EditSnippet(Some(i))).icon("document-edit"),
                UiAction::new("Delete", Op::DeleteSnippet(i))
                    .icon("edit-delete")
                    .destructive()
                    .shortcut("<Ctrl>d")
                    .confirm(&format!("Delete “{}”?", snippet.name)),
            ],
            preview: Some(Preview {
                icon: Some(Icon::Spec("insert-text".into())),
                title: snippet.name.clone(),
                subtitle: Some("Snippet".into()),
                body: Some(snippet.text.clone()),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    rows.push(Row {
        key: "sn:new".into(),
        title: "Add Snippet".into(),
        subtitle: Some("Text to copy, with {date}, {time} or {clipboard}".into()),
        icon: Some(Icon::Spec("list-add".into())),
        tint: Some("green"),
        section: Some("Snippets".into()),
        actions: vec![UiAction::new("Add Snippet", Op::EditSnippet(None))],
        ..Default::default()
    });

    for id in &c.hidden_apps {
        let name = gtk::gio::DesktopAppInfo::new(id)
            .map(|a| gtk::prelude::AppInfoExt::display_name(&a).to_string())
            .unwrap_or_else(|| id.clone());
        rows.push(Row {
            key: format!("hidden:{id}"),
            title: name,
            subtitle: Some(id.clone()),
            icon: Some(Icon::Spec("view-conceal".into())),
            tint: Some("gray"),
            section: Some("Hidden Applications".into()),
            actions: vec![UiAction::new("Show in Results", Op::UnhideApp(id.clone())).icon("view-reveal")],
            ..Default::default()
        });
    }
    rows
}

fn text(id: &str, title: &str, value: &str, placeholder: &str, required: bool) -> Field {
    Field {
        id: id.into(),
        title: title.into(),
        placeholder: Some(placeholder.into()).filter(|p: &String| !p.is_empty()),
        required,
        value: Some(Value::String(value.into())),
        kind: FieldKind::Text,
    }
}

pub fn quicklink_form(link: Option<&Quicklink>) -> FormView {
    FormView {
        id: None,
        title: Some(if link.is_some() { "Edit Quicklink" } else { "Add Quicklink" }.into()),
        submit_title: Some("Save".into()),
        fields: vec![
            text("name", "Name", link.map_or("", |l| &l.name), "Search GitHub", true),
            text("url", "URL", link.map_or("", |l| &l.url), "https://github.com/search?q={query}", true),
            text(
                "keyword",
                "Keyword",
                link.and_then(|l| l.keyword.as_deref()).unwrap_or_default(),
                "gh  (then type “gh tokio”)",
                false,
            ),
        ],
    }
}

pub fn snippet_form(snippet: Option<&Snippet>) -> FormView {
    let mut body = text("text", "Text", snippet.map_or("", |s| &s.text), "", true);
    body.kind = FieldKind::TextArea;
    FormView {
        id: None,
        title: Some(if snippet.is_some() { "Edit Snippet" } else { "Add Snippet" }.into()),
        submit_title: Some("Save".into()),
        fields: vec![
            text("name", "Name", snippet.map_or("", |s| &s.name), "Email signature", true),
            body,
            text("keyword", "Keyword", snippet.and_then(|s| s.keyword.as_deref()).unwrap_or_default(), "sig", false),
        ],
    }
}

pub fn hotkey_form(current: &str) -> FormView {
    FormView {
        id: None,
        title: Some("Global Shortcut".into()),
        submit_title: Some("Save".into()),
        fields: vec![text(
            "hotkey",
            "Shortcut (empty to disable; applies at the next start)",
            current,
            "Super+Space",
            false,
        )],
    }
}

/// The extension's preferences, prefilled. Saved secrets are not shown: an
/// empty password field keeps the current value.
pub fn preferences_form(manifest: &Manifest, config: &Config) -> FormView {
    let id = &manifest.extension.id;
    let plain = config.extensions.get(id).map(|e| e.preferences.clone()).unwrap_or_default();
    let fields = manifest
        .preferences
        .iter()
        .map(|p| {
            let current = plain.get(&p.name).or(p.default.as_ref());
            let (kind, value, required, placeholder) = match p.kind {
                PreferenceKind::Secret => {
                    let saved = secrets::get(id, &p.name).is_some();
                    let hint = if saved { "Saved — leave empty to keep it" } else { "" };
                    (FieldKind::Password, None, p.required && !saved, hint.to_string())
                }
                PreferenceKind::Boolean => (
                    FieldKind::Checkbox,
                    Some(Value::Bool(current.and_then(|v| v.as_bool()).unwrap_or(false))),
                    false,
                    String::new(),
                ),
                PreferenceKind::Text | PreferenceKind::Number => {
                    let text =
                        current.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).unwrap_or_default();
                    (FieldKind::Text, Some(Value::String(text)), p.required, p.description.clone())
                }
            };
            Field {
                id: p.name.clone(),
                title: p.title.clone(),
                placeholder: Some(placeholder).filter(|s| !s.is_empty()),
                required,
                value,
                kind,
            }
        })
        .collect();
    FormView {
        id: None,
        title: Some(format!("{} Preferences", manifest.extension.title)),
        submit_title: Some("Save".into()),
        fields,
    }
}

/// Markdown for the approval screen.
pub fn approval_markdown(manifest: &Manifest) -> String {
    let mut md = format!("# Allow {}?\n\n", manifest.extension.title);
    if !manifest.extension.description.is_empty() {
        md.push_str(&format!("{}\n\n", manifest.extension.description));
    }
    md.push_str("This extension asks to:\n\n");
    for line in approvals::requested(manifest) {
        md.push_str(&format!("- **{line}**\n"));
    }
    md.push_str(
        "\nEverything else is blocked by the sandbox. You will be asked again if an update wants more.\n\n\
         Press **Enter** to allow and run, **Esc** to cancel.",
    );
    md
}
