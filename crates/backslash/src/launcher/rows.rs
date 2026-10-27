//! Turning state into rows: the root search, extension lists, built-in views.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use backslash_core::config::{Quicklink, Snippet, Theme};
use backslash_core::protocol::{self as proto, Accessory, Filtering};
use backslash_core::registry::CommandEntry;
use backslash_core::search::{Ranker, Searchable};
use backslash_core::system::{clipboard, files};
use backslash_core::{calc, units};
use gtk::gio;

use super::model::{tint_for, Builtin, Filter, ItemRef, Op, Preview, Row, RowStyle, UiAction};
use super::State;
use crate::apps::App;
use crate::currency::RatesStatus;
use crate::icons::Icon;

const MAX_ROWS: usize = 60;

/// Searchable stand-in for things without their own type.
struct Entry {
    /// Usage-history id, so frequently used entries rank higher.
    key: String,
    title: String,
    keywords: Vec<String>,
    subtitle: Option<String>,
}

impl Searchable for Entry {
    fn title(&self) -> &str {
        &self.title
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
    fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }
}

fn entry(title: &str, keywords: &[&str]) -> Entry {
    Entry {
        key: String::new(),
        title: title.into(),
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
        subtitle: None,
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// "just now", "5 min ago", "3 h ago", "2 days ago", or a date.
pub fn ago(time: u64) -> String {
    let age = now().saturating_sub(time);
    match age {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", age / 60),
        3600..=86_399 => format!("{} h ago", age / 3600),
        86_400..=172_799 => "yesterday".into(),
        172_800..=2_591_999 => format!("{} days ago", age / 86_400),
        _ => gtk::glib::DateTime::from_unix_local(time as i64)
            .ok()
            .and_then(|d| d.format("%x").ok())
            .map_or_else(|| "long ago".into(), |s| s.to_string()),
    }
}

fn copy_actions(value: &str, full: &str) -> Vec<UiAction> {
    vec![
        UiAction::new("Copy Result", Op::Copy(value.to_string())).icon("edit-copy"),
        UiAction::new("Copy Full Answer", Op::Copy(full.to_string())).icon("edit-copy"),
    ]
}

fn answer(key: &str, value: String, detail: String, kind: &str, facts: Vec<(String, String)>) -> Row {
    Row {
        key: key.into(),
        title: value.clone(),
        subtitle: Some(detail.clone()),
        icon: Some(Icon::Spec("accessories-calculator".into())),
        tint: Some("orange"),
        style: RowStyle::Answer,
        actions: copy_actions(&value, &detail),
        preview: Some(Preview {
            icon: Some(Icon::Spec("accessories-calculator".into())),
            title: value,
            subtitle: Some(kind.into()),
            facts,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Calculator, unit and currency conversion, quicklink keyword.
fn answers(s: &mut State, query: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let q = query.trim();
    if let Some(value) = calc::evaluate(q) {
        let result = calc::format(value);
        rows.push(answer("calc", result.clone(), q.to_string(), "Calculator", vec![("Expression".into(), q.into())]));
    }
    match units::parse(q) {
        Some(units::Request::Unit { value, from, to }) => {
            let result = format!("{} {}", units::format(units::convert(value, from, to)), to.symbol);
            let detail = format!("{} {} = {result}", units::format(value), from.symbol);
            rows.push(answer("convert", result, detail, "Conversion", Vec::new()));
        }
        Some(units::Request::Currency { amount, from, to }) => match s.currency.status() {
            RatesStatus::Ready(rates) => {
                if let Some(v) = rates.convert(amount, &from, &to) {
                    let result = format!("{v:.2} {to}");
                    let detail = format!("{amount} {from} = {result}");
                    let facts = vec![
                        ("Source".into(), "European Central Bank".into()),
                        ("Rates of".into(), rates.date.clone()),
                    ];
                    rows.push(answer("currency", result, detail, "Currency", facts));
                }
            }
            RatesStatus::Loading => rows.push(Row {
                key: "currency".into(),
                title: "Fetching exchange rates…".into(),
                subtitle: Some(format!("{amount} {from} → {to}")),
                icon: Some(Icon::Spec("network-transmit-receive".into())),
                tint: Some("orange"),
                ..Default::default()
            }),
            RatesStatus::Failed(e) => rows.push(Row {
                key: "currency".into(),
                title: "Exchange rates unavailable".into(),
                subtitle: Some(e),
                icon: Some(Icon::Spec("network-offline".into())),
                tint: Some("red"),
                ..Default::default()
            }),
        },
        None => {}
    }
    // `gh tokio` with a quicklink whose keyword is `gh`.
    if let Some((keyword, rest)) = q.split_once(' ') {
        let rest = rest.trim();
        for link in s.config.quicklinks.iter().filter(|l| l.takes_query()) {
            if link.keyword.as_deref() == Some(keyword) && !rest.is_empty() {
                let url = link.url_for(rest);
                rows.push(Row {
                    key: format!("quicklink-run:{}", link.name),
                    title: format!("{} “{rest}”", link.name),
                    subtitle: Some(url.clone()),
                    icon: Some(Icon::Spec(link.icon.clone().unwrap_or_else(|| "web-browser".into()))),
                    tint: Some("blue"),
                    actions: vec![
                        UiAction::new("Open", Op::Open(url.clone())).icon("web-browser"),
                        UiAction::new("Copy URL", Op::Copy(url)).icon("edit-copy"),
                    ],
                    ..Default::default()
                });
            }
        }
    }
    rows
}

/// Built-in commands shown in the root search.
fn builtin_commands(s: &State) -> Vec<(Entry, Row)> {
    let mut out = Vec::new();
    let mut add = |id: &str, title: &str, keywords: &[&str], icon: &str, tint: &'static str, op: Op, about: &str| {
        let row = Row {
            key: format!("builtin:{id}"),
            title: title.into(),
            subtitle: Some("Command · Backslash".into()),
            icon: Some(Icon::Spec(icon.into())),
            tint: Some(tint),
            section: Some("Commands".into()),
            actions: vec![UiAction::new("Open Command", op).icon("system-run")],
            preview: Some(Preview {
                icon: Some(Icon::Spec(icon.into())),
                title: title.into(),
                subtitle: Some("Built into Backslash".into()),
                body: Some(about.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut e = entry(title, keywords);
        e.key = row.key.clone();
        out.push((e, row));
    };
    if s.config.clipboard.enabled {
        add(
            "clipboard",
            "Clipboard History",
            &["paste", "copy", "history", "presse-papier"],
            "edit-paste",
            "purple",
            Op::OpenBuiltin(Builtin::Clipboard),
            "Everything you copied, searchable. Pin what you reuse.",
        );
    }
    add(
        "files",
        "Search Files",
        &["find", "document", "fichier", "recent"],
        "system-search",
        "blue",
        Op::OpenBuiltin(Builtin::Files),
        "Find files in your home folder. Recent files appear first.",
    );
    if s.window_backend.is_some() {
        add(
            "windows",
            "Switch Windows",
            &["window", "focus", "fenêtre", "alt tab"],
            "preferences-system-windows",
            "teal",
            Op::OpenBuiltin(Builtin::Windows),
            "Jump to any open window.",
        );
    }
    add(
        "theme",
        "Change Theme",
        &["dark", "light", "appearance", "mode", "thème", "sombre", "clair"],
        "preferences-desktop-appearance",
        "yellow",
        Op::OpenBuiltin(Builtin::Theme),
        "Light, dark, or follow the system.",
    );
    let (title, op, about) = if s.autostart {
        ("Disable Launch at Login", Op::SetAutostart(false), "Stop starting Backslash with your session.")
    } else {
        (
            "Enable Launch at Login",
            Op::SetAutostart(true),
            "Start Backslash in the background with your session, so it opens instantly.",
        )
    };
    add("autostart", title, &["startup", "autostart", "login", "démarrage"], "system-run", "green", op, about);
    add(
        "store",
        "Browse Extensions",
        &["store", "install", "extensions", "plugins", "registry", "update"],
        "system-software-install",
        "blue",
        Op::OpenBuiltin(Builtin::Store),
        "Find, install and update extensions. You see what each one is allowed to do before installing.",
    );
    add(
        "settings",
        "Settings",
        &["preferences", "extensions", "quicklinks", "snippets", "réglages", "configure"],
        "preferences-system",
        "gray",
        Op::OpenBuiltin(Builtin::Settings),
        "Theme, shortcut, extensions and their preferences, quicklinks, snippets.",
    );
    add(
        "config",
        "Open Config File",
        &["settings", "preferences", "quicklinks", "snippets", "réglages"],
        "preferences-system",
        "gray",
        Op::Open(backslash_core::config::Config::path().display().to_string()),
        "Theme, hotkey, quicklinks, snippets, hidden apps and extension preferences live in config.toml.",
    );
    out
}

fn snippet_row(snippet: &Snippet) -> Row {
    Row {
        key: format!("snippet:{}", snippet.name),
        title: snippet.name.clone(),
        subtitle: Some(match &snippet.keyword {
            Some(kw) => format!("Snippet · {kw}"),
            None => "Snippet".into(),
        }),
        icon: Some(Icon::Spec("insert-text".into())),
        tint: Some("green"),
        section: Some("Commands".into()),
        actions: vec![UiAction::new("Copy Snippet", Op::Snippet(snippet.text.clone())).icon("edit-copy")],
        preview: Some(Preview {
            icon: Some(Icon::Spec("insert-text".into())),
            title: snippet.name.clone(),
            subtitle: Some("Snippet".into()),
            body: Some(snippet.text.clone()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The root search row with this key, wherever it comes from.
pub fn row_for_key(s: &State, key: &str) -> Option<Row> {
    let (kind, id) = key.split_once(':')?;
    match kind {
        "app" => s.apps.iter().find(|a| a.id == id).map(|a| app_row(a, s.usage.stats(key))),
        "cmd" => s.commands.iter().find(|c| c.id == id).map(|c| command_row(s, c)),
        "builtin" => builtin_commands(s).into_iter().map(|(_, r)| r).find(|r| r.key == key),
        "quicklink" => s.config.quicklinks.iter().find(|l| l.name == id).map(quicklink_row),
        "snippet" => s.config.snippets.iter().find(|n| n.name == id).map(snippet_row),
        _ => None,
    }
}

/// Whether a row can get an alias and a hotkey.
pub fn is_assignable(key: &str) -> bool {
    ["app:", "cmd:", "builtin:", "quicklink:", "snippet:"].iter().any(|p| key.starts_with(p))
}

/// Shows the alias and hotkey of a root row, and actions to change them.
fn decorate(s: &State, row: &mut Row) {
    if !is_assignable(&row.key) {
        return;
    }
    let alias = s.config.aliases.get(&row.key);
    let hotkey = s.config.hotkeys.get(&row.key);
    if let Some(alias) = alias {
        row.accessories
            .insert(0, Accessory { tag: Some(alias.clone()), color: Some("gray".into()), ..Default::default() });
    }
    if let Some(p) = row.preview.as_mut() {
        if let Some(alias) = alias {
            p.facts.push(("Alias".into(), alias.clone()));
        }
        if let Some(hotkey) = hotkey {
            p.facts.push(("Hotkey".into(), hotkey.clone()));
        }
    }
    row.actions.push(
        UiAction::new(if alias.is_some() { "Change Alias…" } else { "Set Alias…" }, Op::EditAlias(row.key.clone()))
            .icon("insert-text"),
    );
    row.actions.push(
        UiAction::new(
            if hotkey.is_some() { "Change Hotkey…" } else { "Set Hotkey…" },
            Op::EditKeyHotkey(row.key.clone()),
        )
        .icon("input-keyboard"),
    );
}

fn quicklink_row(link: &Quicklink) -> Row {
    let (title, op) = match (&link.keyword, link.takes_query()) {
        (Some(kw), true) => ("Type a search…", Op::FillQuery(format!("{kw} "))),
        (None, true) => ("Open", Op::Open(link.url_for(""))),
        _ => ("Open", Op::Open(link.url.clone())),
    };
    let icon = link.icon.clone().unwrap_or_else(|| "web-browser".into());
    Row {
        key: format!("quicklink:{}", link.name),
        title: link.name.clone(),
        subtitle: Some(match &link.keyword {
            Some(kw) => format!("Quicklink · {kw}"),
            None => "Quicklink".into(),
        }),
        icon: Some(Icon::Spec(icon.clone())),
        tint: Some("blue"),
        section: Some("Commands".into()),
        actions: vec![
            UiAction::new(title, op).icon("web-browser"),
            UiAction::new("Copy URL", Op::Copy(link.url.clone())),
        ],
        preview: Some(Preview {
            icon: Some(Icon::Spec(icon)),
            title: link.name.clone(),
            subtitle: Some("Quicklink".into()),
            facts: vec![
                ("URL".into(), link.url.clone()),
                ("Keyword".into(), link.keyword.clone().unwrap_or_else(|| "—".into())),
            ],
            body: None,
        }),
        ..Default::default()
    }
}

fn app_row(a: &App, usage: Option<(u32, u64)>) -> Row {
    let mut actions = vec![UiAction::new("Open Application", Op::LaunchApp(a.id.clone(), None)).icon("document-open")];
    actions.extend(
        a.actions.iter().map(|(id, name)| UiAction::new(name.clone(), Op::LaunchApp(a.id.clone(), Some(id.clone())))),
    );
    actions.push(UiAction::new("Hide from Results", Op::HideApp(a.id.clone())).icon("view-conceal"));
    let mut facts = vec![("Source".to_string(), a.source.to_string())];
    if let Some((count, last)) = usage {
        facts.push(("Last opened".into(), ago(last)));
        facts.push(("Opened".into(), if count == 1 { "once".into() } else { format!("{count} times") }));
    }
    Row {
        key: format!("app:{}", a.id),
        title: a.name.clone(),
        subtitle: Some("Application".into()),
        icon: a.icon.clone(),
        section: Some("Applications".into()),
        actions,
        preview: Some(Preview {
            icon: a.icon.clone(),
            title: a.name.clone(),
            subtitle: a.description.clone().or_else(|| a.generic_name.clone()),
            facts,
            body: None,
        }),
        ..Default::default()
    }
}

fn command_row(s: &State, c: &CommandEntry) -> Row {
    let ext = &s.registry.extensions[c.extension];
    let manifest = &ext.manifest;
    let command = manifest.commands.iter().find(|cmd| format!("{}/{}", manifest.extension.id, cmd.name) == c.id);
    let mut facts = vec![("Extension".to_string(), manifest.extension.title.clone())];
    if !manifest.extension.author.is_empty() {
        facts.push(("Author".into(), manifest.extension.author.clone()));
    }
    let permissions = manifest.permissions.describe();
    facts.push(("Permissions".into(), if permissions.is_empty() { "None".into() } else { permissions.join("\n") }));
    Row {
        key: format!("cmd:{}", c.id),
        title: c.title.clone(),
        subtitle: Some(format!("Command · {}", c.subtitle)),
        icon: Some(Icon::Spec(c.icon.clone().unwrap_or_else(|| "system-run".into()))),
        tint: Some(tint_for(&manifest.extension.id)),
        section: Some("Commands".into()),
        actions: vec![UiAction::new("Open Command", Op::StartCommand(c.id.clone())).icon("system-run")],
        preview: Some(Preview {
            icon: Some(Icon::Spec(c.icon.clone().unwrap_or_else(|| "system-run".into()))),
            title: c.title.clone(),
            subtitle: command.map(|c| c.description.clone()).filter(|d| !d.is_empty()),
            facts,
            body: None,
        }),
        ..Default::default()
    }
}

pub fn root_rows(s: &mut State, query: &str) -> Vec<Row> {
    let mut rows = if s.filter == Filter::All { answers(s, query) } else { Vec::new() };
    let mut scored: Vec<(u32, Row)> = Vec::new();

    if s.filter != Filter::Commands {
        let app_id = |a: &App| format!("app:{}", a.id);
        for (i, score) in s.ranker.rank(query, &s.apps, Some((&s.usage, &app_id))) {
            let a = &s.apps[i];
            scored.push((score, app_row(a, s.usage.stats(&app_id(a)))));
        }
    }
    if s.filter != Filter::Apps {
        let cmd_id = |c: &CommandEntry| format!("cmd:{}", c.id);
        let ranked = s.ranker.rank(query, &s.commands, Some((&s.usage, &cmd_id)));
        for (i, score) in ranked {
            scored.push((score, command_row(s, &s.commands[i])));
        }

        let mut extra: Vec<(Entry, Row)> = builtin_commands(s);
        for link in &s.config.quicklinks {
            let mut kw = vec![link.url.clone()];
            kw.extend(link.keyword.clone());
            let row = quicklink_row(link);
            extra.push((Entry { key: row.key.clone(), title: link.name.clone(), keywords: kw, subtitle: None }, row));
        }
        for snippet in &s.config.snippets {
            let row = snippet_row(snippet);
            let keywords = snippet.keyword.iter().cloned().collect();
            extra.push((Entry { key: row.key.clone(), title: snippet.name.clone(), keywords, subtitle: None }, row));
        }
        let entries: Vec<&Entry> = extra.iter().map(|(e, _)| e).collect();
        let key_of = |e: &&Entry| e.key.clone();
        for (i, score) in s.ranker.rank(query, &entries, Some((&s.usage, &key_of))) {
            scored.push((score, extra[i].1.clone()));
        }
    }
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    if let Some(row) = s.config.alias_target(query).and_then(|key| row_for_key(s, key)) {
        scored.retain(|(_, r)| r.key != row.key);
        scored.insert(0, (u32::MAX, row));
    }

    if query.trim().is_empty() {
        // Suggestions first (what you use), then everything grouped by kind.
        let mut suggestions = Vec::new();
        let mut rest = Vec::new();
        for (score, mut row) in scored {
            if score > 0 && suggestions.len() < 6 {
                row.section = Some("Suggestions".into());
                suggestions.push(row);
            } else {
                rest.push(row);
            }
        }
        // Commands (few, and Backslash's own features) before the long app list.
        rest.sort_by_key(|r| (r.section.as_deref() != Some("Commands"), r.title.to_lowercase()));
        rows.extend(suggestions);
        rows.extend(rest);
    } else {
        rows.extend(scored.into_iter().map(|(_, mut r)| {
            r.section = Some("Results".into());
            r
        }));
    }
    rows.truncate(MAX_ROWS);
    for row in &mut rows {
        decorate(s, row);
    }
    rows
}

impl Searchable for &Entry {
    fn title(&self) -> &str {
        &self.title
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
    fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }
}

pub fn list_rows(ranker: &mut Ranker, list: &proto::ListView, query: &str, extension_id: &str) -> Vec<Row> {
    let ranked = list.filtering == Filtering::Host && !query.trim().is_empty();
    let order: Vec<usize> = if ranked {
        let refs: Vec<ItemRef> = list.items.iter().map(ItemRef).collect();
        ranker.rank(query, &refs, None).into_iter().map(|(i, _)| i).collect()
    } else {
        (0..list.items.len()).collect()
    };
    let tint = tint_for(extension_id);
    order
        .into_iter()
        .take(500)
        .map(|i| {
            let item = &list.items[i];
            Row {
                key: item.id.clone(),
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                icon: item.icon.clone().map(Icon::Spec),
                tint: Some(tint),
                accessories: item.accessories.clone(),
                // Sections only make sense in the extension's own order.
                section: if ranked { None } else { item.section.clone() },
                actions: item.actions.iter().map(UiAction::from_proto).collect(),
                ..Default::default()
            }
        })
        .collect()
}

fn ranked_rows(ranker: &mut Ranker, query: &str, entries: Vec<(Entry, Row)>) -> Vec<Row> {
    if query.trim().is_empty() {
        return entries.into_iter().map(|(_, r)| r).collect();
    }
    let refs: Vec<&Entry> = entries.iter().map(|(e, _)| e).collect();
    let order: Vec<usize> = ranker.rank(query, &refs, None).into_iter().map(|(i, _)| i).collect();
    order.into_iter().map(|i| entries[i].1.clone()).collect()
}

pub fn builtin_rows(s: &mut State, builtin: Builtin, query: &str) -> Vec<Row> {
    match builtin {
        Builtin::Clipboard => {
            let entries: Vec<(Entry, Row)> = s
                .clipboard
                .entries()
                .into_iter()
                .map(|e| {
                    let lines = e.text.lines().count().max(1);
                    let chars = e.text.chars().count();
                    let pin_title = if e.pinned { "Unpin" } else { "Pin" };
                    let row = Row {
                        key: format!("clip:{}", e.id),
                        title: clipboard::summary(&e.text),
                        subtitle: Some(if e.pinned { format!("Pinned · {}", ago(e.time)) } else { ago(e.time) }),
                        icon: Some(Icon::Spec(if e.pinned { "view-pin" } else { "edit-paste" }.into())),
                        tint: Some(if e.pinned { "orange" } else { "purple" }),
                        section: Some(if e.pinned { "Pinned" } else { "History" }.into()),
                        actions: vec![
                            UiAction::new("Copy to Clipboard", Op::ClipCopy(e.id)).icon("edit-copy"),
                            UiAction::new(pin_title, Op::ClipPin(e.id)).icon("view-pin").shortcut("<Ctrl><Shift>p"),
                            UiAction::new("Delete", Op::ClipDelete(e.id))
                                .icon("edit-delete")
                                .shortcut("<Ctrl>d")
                                .destructive(),
                            UiAction::new("Clear History", Op::ClipClear)
                                .icon("edit-clear-all")
                                .destructive()
                                .confirm("Delete all unpinned entries?"),
                        ],
                        preview: Some(Preview {
                            icon: Some(Icon::Spec("edit-paste".into())),
                            title: "Text".into(),
                            subtitle: Some(format!(
                                "{chars} characters · {lines} line{}",
                                if lines > 1 { "s" } else { "" }
                            )),
                            facts: vec![("Copied".into(), ago(e.time))],
                            body: Some(e.text.chars().take(4000).collect()),
                        }),
                        ..Default::default()
                    };
                    let entry = Entry {
                        key: row.key.clone(),
                        title: row.title.clone(),
                        keywords: Vec::new(),
                        subtitle: Some(e.text.clone()),
                    };
                    (entry, row)
                })
                .collect();
            let mut rows = ranked_rows(&mut s.ranker, query, entries);
            if !query.trim().is_empty() {
                rows.iter_mut().for_each(|r| r.section = None);
            }
            rows
        }
        Builtin::Files => {
            let results = if query.trim().chars().count() < 2 { &s.files.recent } else { &s.files.results };
            let section = if query.trim().chars().count() < 2 { "Recent Files" } else { "Files" };
            results.iter().map(|p| file_row(p, section)).collect()
        }
        Builtin::Windows => {
            let entries: Vec<(Entry, Row)> = s
                .windows
                .iter()
                .map(|w| {
                    let app = find_app(&s.apps, &w.app_id);
                    let app_name = app.map_or_else(|| w.app_id.clone(), |a| a.name.clone());
                    let icon = app.and_then(|a| a.icon.clone()).or(Some(Icon::Spec("application-x-executable".into())));
                    let mut facts = vec![("Application".into(), app_name.clone())];
                    if let Some(ws) = &w.workspace {
                        facts.push(("Workspace".into(), ws.clone()));
                    }
                    let row = Row {
                        key: format!("win:{}", w.id),
                        title: if w.title.is_empty() { app_name.clone() } else { w.title.clone() },
                        subtitle: Some(match &w.workspace {
                            Some(ws) => format!("{app_name} · workspace {ws}"),
                            None => app_name.clone(),
                        }),
                        icon: icon.clone(),
                        tint: Some("teal"),
                        actions: vec![UiAction::new("Switch to Window", Op::FocusWindow(w.id.clone())).icon("go-jump")],
                        preview: Some(Preview {
                            icon,
                            title: w.title.clone(),
                            subtitle: Some(app_name.clone()),
                            facts,
                            body: None,
                        }),
                        ..Default::default()
                    };
                    let keywords = vec![app_name, w.app_id.clone()];
                    (Entry { key: row.key.clone(), title: row.title.clone(), keywords, subtitle: None }, row)
                })
                .collect();
            ranked_rows(&mut s.ranker, query, entries)
        }
        Builtin::Store => {
            let entries = super::store::rows(s)
                .into_iter()
                .map(|row| {
                    let keywords = [row.subtitle.clone()].into_iter().flatten().collect();
                    (Entry { key: row.key.clone(), title: row.title.clone(), keywords, subtitle: None }, row)
                })
                .collect();
            let mut rows = ranked_rows(&mut s.ranker, query, entries);
            if !query.trim().is_empty() {
                rows.iter_mut().for_each(|r| r.section = None);
            }
            rows
        }
        Builtin::Settings => {
            let entries = super::settings::rows(s)
                .into_iter()
                .map(|row| {
                    let keywords = [row.section.clone(), row.subtitle.clone()].into_iter().flatten().collect();
                    (Entry { key: row.key.clone(), title: row.title.clone(), keywords, subtitle: None }, row)
                })
                .collect();
            let mut rows = ranked_rows(&mut s.ranker, query, entries);
            if !query.trim().is_empty() {
                rows.iter_mut().for_each(|r| r.section = None);
            }
            rows
        }
        Builtin::Theme => {
            let current = s.config.theme;
            let entries = [
                (Theme::System, "System", "Follow the desktop's light or dark preference", "emblem-system"),
                (Theme::Light, "Light", "Bright and calm", "weather-clear"),
                (Theme::Dark, "Dark", "Easy on the eyes at night", "weather-clear-night"),
            ]
            .into_iter()
            .map(|(theme, title, about, icon)| {
                let row = Row {
                    key: format!("theme:{title}"),
                    title: title.into(),
                    subtitle: Some(about.into()),
                    icon: Some(Icon::Spec(icon.into())),
                    tint: Some("yellow"),
                    accessories: if theme == current {
                        vec![Accessory { tag: Some("Current".into()), ..Default::default() }]
                    } else {
                        Vec::new()
                    },
                    actions: vec![UiAction::new("Use Theme", Op::SetTheme(theme)).icon("object-select")],
                    ..Default::default()
                };
                (entry(title, &[]), row)
            })
            .collect();
            ranked_rows(&mut s.ranker, query, entries)
        }
    }
}

/// The application owning a window, by Wayland app id or X11 class.
fn find_app<'a>(apps: &'a [App], app_id: &str) -> Option<&'a App> {
    let id = app_id.to_lowercase();
    if id.is_empty() {
        return None;
    }
    apps.iter()
        .find(|a| a.id.trim_end_matches(".desktop").to_lowercase() == id)
        .or_else(|| apps.iter().find(|a| a.wm_class.as_deref().is_some_and(|c| c.to_lowercase() == id)))
        .or_else(|| apps.iter().find(|a| a.id.to_lowercase().trim_end_matches(".desktop").ends_with(&format!(".{id}"))))
        .or_else(|| apps.iter().find(|a| a.name.to_lowercase() == id))
}

fn file_row(path: &Path, section: &str) -> Row {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let parent = path.parent().map(files::tilde).unwrap_or_default();
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    let (content_type, _) = gio::content_type_guess(Some(path), None);
    let content_type = if is_dir { "inode/directory".into() } else { content_type };
    let icon = Icon::Gio(gio::content_type_get_icon(&content_type));
    let mut facts = vec![("Kind".to_string(), gio::content_type_get_description(&content_type).to_string())];
    if let Some(m) = &meta {
        if !is_dir {
            facts.push(("Size".into(), gtk::glib::format_size(m.len()).to_string()));
        }
        if let Ok(modified) = m.modified() {
            let secs = modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
            facts.push(("Modified".into(), ago(secs)));
        }
    }
    facts.push(("Where".into(), parent.clone()));
    let target = path.display().to_string();
    Row {
        key: format!("file:{target}"),
        title: name.clone(),
        subtitle: Some(parent),
        icon: Some(icon.clone()),
        section: Some(section.into()),
        actions: vec![
            UiAction::new("Open", Op::Open(target.clone())).icon("document-open"),
            UiAction::new("Show in Folder", Op::Reveal(path.to_path_buf())).icon("folder-open"),
            UiAction::new("Copy Path", Op::Copy(target.clone())).icon("edit-copy").shortcut("<Ctrl><Shift>c"),
        ],
        preview: Some(Preview {
            icon: Some(icon),
            title: name,
            subtitle: None,
            facts,
            body: text_excerpt(path, &content_type),
        }),
        ..Default::default()
    }
}

/// The beginning of a text file, for the preview.
fn text_excerpt(path: &Path, content_type: &str) -> Option<String> {
    if !gio::content_type_is_a(content_type, "text/plain") {
        return None;
    }
    use std::io::Read;
    let mut buf = vec![0u8; 2048];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    let text = String::from_utf8_lossy(&buf).into_owned();
    (!text.trim().is_empty()).then_some(text)
}
