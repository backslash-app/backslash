//! Turning state into rows: the root search, extension lists, built-in views.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use backslash_core::config::{Quicklink, Snippet, Theme};
use backslash_core::protocol::{self as proto, Accessory, Filtering};
use backslash_core::registry::CommandEntry;
use backslash_core::scripts::{Mode as ScriptMode, Script};
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
        UiAction::new("Copy Result", Op::Copy(value.to_string())).icon("bs-copy"),
        UiAction::new("Copy Full Answer", Op::Copy(full.to_string())).icon("bs-copy"),
    ]
}

fn answer(key: &str, value: String, detail: String, kind: &str, facts: Vec<(String, String)>) -> Row {
    Row {
        key: key.into(),
        title: value.clone(),
        subtitle: Some(detail.clone()),
        icon: Icon::named("bs-calculator"),
        tint: Some("orange"),
        style: RowStyle::Answer,
        actions: copy_actions(&value, &detail),
        preview: Some(Preview {
            icon: Icon::named("bs-calculator"),
            tint: Some("orange".into()),
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
                icon: Icon::named("bs-refresh"),
                tint: Some("orange"),
                ..Default::default()
            }),
            RatesStatus::Failed(e) => rows.push(Row {
                key: "currency".into(),
                title: "Exchange rates unavailable".into(),
                subtitle: Some(e),
                icon: Icon::named("bs-wifi-off"),
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
                    icon: Some(Icon::Spec(link.icon.clone().unwrap_or_else(|| "bs-link".into()))),
                    tint: Some("blue"),
                    actions: vec![
                        UiAction::new("Open", Op::Open(url.clone())).icon("bs-external"),
                        UiAction::new("Copy URL", Op::Copy(url)).icon("bs-copy"),
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
            subtitle: Some("Backslash".into()),
            icon: Icon::named(icon),
            tint: Some(tint),
            section: Some("Commands".into()),
            actions: vec![UiAction::new("Open Command", op).icon("bs-arrow-right")],
            preview: Some(Preview {
                icon: Icon::named(icon),
                tint: Some(tint.into()),
                title: title.into(),
                subtitle: Some("Built into Backslash".into()),
                markdown: Some(about.into()),
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
            "bs-paste",
            "purple",
            Op::OpenBuiltin(Builtin::Clipboard),
            "Everything you copied, searchable. Pin what you reuse; Enter pastes it where you were.",
        );
    }
    add(
        "files",
        "Search Files",
        &["find", "document", "fichier", "recent"],
        "bs-folder",
        "blue",
        Op::OpenBuiltin(Builtin::Files),
        "Find files in your home folder. Recent files appear first.",
    );
    if s.window_backend.is_some() || s.kwin || s.gnome_helper_needed {
        add(
            "windows",
            "Switch Windows",
            &["window", "focus", "fenêtre", "alt tab"],
            "bs-window",
            "teal",
            Op::OpenBuiltin(Builtin::Windows),
            "Jump to any open window, on any workspace.",
        );
    }
    add(
        "theme",
        "Change Theme",
        &["dark", "light", "appearance", "mode", "thème", "sombre", "clair"],
        "bs-palette",
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
    add("autostart", title, &["startup", "autostart", "login", "démarrage"], "bs-power", "green", op, about);
    add(
        "store",
        "Browse Extensions",
        &["store", "install", "extensions", "plugins", "registry", "update"],
        "bs-package",
        "blue",
        Op::OpenBuiltin(Builtin::Store),
        "Find, install and update extensions. You see what each one is allowed to do before installing.",
    );
    add(
        "new-script",
        "Create Script Command",
        &["script", "shell", "bash", "python", "new", "automation", "créer"],
        "bs-terminal",
        "green",
        Op::NewScript,
        "Turn any shell, Python or other script into a command: a few comment lines at the top describe it.",
    );
    add(
        "scripts",
        "Open Scripts Folder",
        &["script", "scripts", "bs-folder", "dossier"],
        "bs-folder",
        "green",
        Op::OpenScriptsFolder,
        "Script commands live in `~/.config/backslash/scripts`. Edit them with any editor; changes show up the next time Backslash opens.",
    );
    add(
        "welcome",
        "Welcome to Backslash",
        &["setup", "onboarding", "getting started", "tour", "bienvenue", "configurer", "help"],
        "bs-sparkle",
        "teal",
        Op::OpenBuiltin(Builtin::Welcome),
        "Set up the shortcut, theme, clipboard history and extensions.",
    );
    add(
        "settings",
        "Settings",
        &["preferences", "extensions", "quicklinks", "snippets", "réglages", "configure", "accent", "density"],
        "bs-settings",
        "gray",
        Op::OpenSettings,
        "Appearance, shortcuts, extensions and their accounts, quicklinks, snippets.",
    );
    add(
        "config",
        "Open Config File",
        &["settings", "preferences", "quicklinks", "snippets", "réglages"],
        "bs-file",
        "gray",
        Op::Open(backslash_core::config::Config::path().display().to_string()),
        "Everything in the settings lives in `config.toml`, easy to keep in your dotfiles.",
    );
    if let Some((_, row)) =
        out.iter_mut().find(|(e, _)| e.key == "builtin:store").filter(|_| !s.catalog.updates.is_empty())
    {
        let n = s.catalog.updates.len();
        row.accessories.push(Accessory {
            tag: Some(if n == 1 { "1 update".into() } else { format!("{n} updates") }),
            color: Some("orange".into()),
            ..Default::default()
        });
    }
    out
}

fn script_row(script: &Script) -> Row {
    let icon = script.icon.clone().unwrap_or_else(|| "bs-terminal".into());
    let mut run = UiAction::new("Run Script", Op::RunScript(script.id.clone(), None)).icon("bs-play");
    if let Some(question) = &script.confirm {
        run = run.confirm(question).destructive();
    }
    let mode = match script.mode {
        ScriptMode::Silent => "Runs in the background",
        ScriptMode::Output => "Shows its output",
        ScriptMode::List => "Lists what it prints",
        ScriptMode::Paste => "Pastes what it prints",
    };
    Row {
        key: format!("script:{}", script.id),
        title: script.title.clone(),
        subtitle: Some("Script".into()),
        icon: Some(Icon::Spec(icon.clone())),
        tint: Some("green"),
        section: Some("Commands".into()),
        actions: vec![
            run,
            UiAction::new("Edit Script", Op::Open(script.path.display().to_string())).icon("bs-code"),
            UiAction::new("Show in Folder", Op::Reveal(script.path.clone())).icon("bs-folder"),
        ],
        preview: Some(Preview {
            icon: Some(Icon::Spec(icon)),
            tint: Some("green".into()),
            title: script.title.clone(),
            subtitle: script.description.clone(),
            facts: vec![
                ("File".into(), files::tilde(&script.path)),
                ("Mode".into(), mode.into()),
                (
                    "Arguments".into(),
                    if script.arguments.is_empty() {
                        "None".into()
                    } else {
                        script.arguments.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ")
                    },
                ),
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn snippet_row(snippet: &Snippet, can_paste: bool) -> Row {
    let mut actions = Vec::new();
    if can_paste {
        actions.push(UiAction::new("Paste Snippet", Op::Snippet(snippet.text.clone(), true)).icon("bs-paste"));
    }
    actions.push(UiAction::new("Copy Snippet", Op::Snippet(snippet.text.clone(), false)).icon("bs-copy"));
    Row {
        key: format!("snippet:{}", snippet.name),
        title: snippet.name.clone(),
        subtitle: Some("Snippet".into()),
        icon: Icon::named("bs-text"),
        tint: Some("green"),
        section: Some("Commands".into()),
        actions,
        preview: Some(Preview {
            icon: Icon::named("bs-text"),
            tint: Some("green".into()),
            title: snippet.name.clone(),
            subtitle: Some("Snippet".into()),
            body: Some(snippet.text.clone()),
            facts: snippet.keyword.iter().map(|k| ("Keyword".to_string(), k.clone())).collect(),
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
        "script" => s.scripts.iter().find(|x| x.id == id).map(script_row),
        "snippet" => s.config.snippets.iter().find(|n| n.name == id).map(|n| snippet_row(n, s.paste.is_some())),
        _ => None,
    }
}

/// Whether a row can get an alias and a hotkey.
pub fn is_assignable(key: &str) -> bool {
    ["app:", "cmd:", "builtin:", "quicklink:", "snippet:", "script:"].iter().any(|p| key.starts_with(p))
}

/// Shows the alias, hotkey and usage of a root row, and actions to change them.
fn decorate(s: &State, row: &mut Row) {
    if !is_assignable(&row.key) {
        return;
    }
    let alias = s.config.aliases.get(&row.key);
    let hotkey = s.config.hotkeys.get(&row.key);
    row.alias = alias.cloned();
    if let Some(p) = row.preview.as_mut() {
        let mut lines = Vec::new();
        if let Some(alias) = alias {
            lines.push(proto::MetadataEntry { label: "Alias".into(), value: alias.clone(), ..Default::default() });
        }
        if let Some(hotkey) = hotkey {
            lines.push(proto::MetadataEntry {
                label: "Hotkey".into(),
                value: hotkey.clone(),
                icon: Some("bs-keyboard".into()),
                ..Default::default()
            });
        }
        if let Some((count, last)) = s.usage.stats(&row.key) {
            lines.push(proto::MetadataEntry { label: "Last used".into(), value: ago(last), ..Default::default() });
            lines.push(proto::MetadataEntry {
                label: "Used".into(),
                value: if count == 1 { "once".into() } else { format!("{count} times") },
                ..Default::default()
            });
        }
        if !lines.is_empty() && !(p.facts.is_empty() && p.metadata.is_empty()) {
            lines.push(proto::MetadataEntry { separator: true, ..Default::default() });
        }
        lines.append(&mut p.metadata);
        p.metadata = lines;
    }
    row.actions.push(
        UiAction::new(if alias.is_some() { "Change Alias…" } else { "Set Alias…" }, Op::EditAlias(row.key.clone()))
            .icon("bs-text")
            .section("Shortcuts"),
    );
    row.actions.push(
        UiAction::new(
            if hotkey.is_some() { "Change Hotkey…" } else { "Set Hotkey…" },
            Op::EditKeyHotkey(row.key.clone()),
        )
        .icon("bs-keyboard")
        .section("Shortcuts"),
    );
}

fn quicklink_row(link: &Quicklink) -> Row {
    let (title, op) = match (&link.keyword, link.takes_query()) {
        (Some(kw), true) => ("Type a search…", Op::FillQuery(format!("{kw} "))),
        (None, true) => ("Open", Op::Open(link.url_for(""))),
        _ => ("Open", Op::Open(link.url.clone())),
    };
    let icon = link.icon.clone().unwrap_or_else(|| "bs-link".into());
    Row {
        key: format!("quicklink:{}", link.name),
        title: link.name.clone(),
        subtitle: Some("Quicklink".into()),
        icon: Some(Icon::Spec(icon.clone())),
        tint: Some("blue"),
        section: Some("Commands".into()),
        actions: vec![
            UiAction::new(title, op).icon("bs-external"),
            UiAction::new("Copy URL", Op::Copy(link.url.clone())).icon("bs-copy"),
        ],
        preview: Some(Preview {
            icon: Some(Icon::Spec(icon)),
            tint: Some("blue".into()),
            title: link.name.clone(),
            subtitle: Some("Quicklink".into()),
            metadata: vec![
                proto::MetadataEntry {
                    label: "URL".into(),
                    value: link.url.clone(),
                    link: (!link.takes_query()).then(|| link.url.clone()),
                    ..Default::default()
                },
                proto::MetadataEntry {
                    label: "Keyword".into(),
                    value: link.keyword.clone().unwrap_or_else(|| "—".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn app_row(a: &App, _usage: Option<(u32, u64)>) -> Row {
    let mut actions = vec![UiAction::new("Open Application", Op::LaunchApp(a.id.clone(), None)).icon("bs-external")];
    actions.extend(a.actions.iter().map(|(id, name)| {
        UiAction::new(name.clone(), Op::LaunchApp(a.id.clone(), Some(id.clone())))
            .icon("bs-arrow-right")
            .section("Application")
    }));
    actions.push(UiAction::new("Hide from Results", Op::HideApp(a.id.clone())).icon("bs-eye-off"));
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
            facts: vec![("Source".to_string(), a.source.to_string())],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn command_row(s: &State, c: &CommandEntry) -> Row {
    let ext = &s.registry.extensions[c.extension];
    let manifest = &ext.manifest;
    let command = manifest.commands.iter().find(|cmd| format!("{}/{}", manifest.extension.id, cmd.name) == c.id);
    let icon = c.icon.clone().or_else(|| manifest.extension.icon.clone()).unwrap_or_else(|| "bs-bolt".into());
    let tint = tint_for(&manifest.extension.id);
    let mut accessories = Vec::new();
    if let Some(sub) = s.command_meta.get(&c.id) {
        accessories.push(Accessory { tag: Some(sub.to_string()), color: Some(tint.into()), ..Default::default() });
    }
    let permissions = manifest.permissions.describe();
    let mut metadata = vec![proto::MetadataEntry {
        label: "Extension".into(),
        value: format!("{} {}", manifest.extension.title, manifest.extension.version),
        ..Default::default()
    }];
    if !manifest.extension.author.is_empty() {
        metadata.push(proto::MetadataEntry {
            label: "Author".into(),
            value: manifest.extension.author.clone(),
            ..Default::default()
        });
    }
    metadata.push(proto::MetadataEntry {
        label: "Can".into(),
        value: if permissions.is_empty() { "Nothing outside its sandbox".into() } else { permissions.join("\n") },
        ..Default::default()
    });
    Row {
        key: format!("cmd:{}", c.id),
        title: c.title.clone(),
        subtitle: Some(manifest.extension.title.clone()),
        icon: Some(Icon::Spec(icon.clone())),
        tint: Some(tint),
        accessories,
        section: Some("Commands".into()),
        actions: vec![UiAction::new("Open Command", Op::StartCommand(c.id.clone())).icon("bs-arrow-right")],
        preview: Some(Preview {
            icon: Some(Icon::Spec(icon)),
            tint: Some(tint.into()),
            title: c.title.clone(),
            subtitle: command.map(|c| c.description.clone()).filter(|d| !d.is_empty()),
            markdown: s.command_meta.get(&c.id).map(|m| format!("**{m}**")),
            metadata,
            ..Default::default()
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
        for script in &s.scripts {
            let row = script_row(script);
            let mut keywords = script.keywords.clone();
            keywords.push(script.id.clone());
            extra.push((Entry { key: row.key.clone(), title: script.title.clone(), keywords, subtitle: None }, row));
        }
        for snippet in &s.config.snippets {
            let row = snippet_row(snippet, s.paste.is_some());
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

/// Rows of an extension list. `hosts` are the extension's `net` hosts:
/// pictures elsewhere are not fetched (that would leak what is shown).
pub fn list_rows(ranker: &mut Ranker, list: &proto::ListView, query: &str, hosts: &[String]) -> Vec<Row> {
    let ranked = list.filtering == Filtering::Host && !query.trim().is_empty();
    let order: Vec<usize> = if ranked {
        let refs: Vec<ItemRef> = list.items.iter().map(ItemRef).collect();
        ranker.rank(query, &refs, None).into_iter().map(|(i, _)| i).collect()
    } else {
        (0..list.items.len()).collect()
    };
    let allowed = |spec: &Option<String>| -> Option<String> {
        spec.clone().filter(|s| !crate::images::is_url(s) || crate::images::allowed(s, hosts))
    };
    order
        .into_iter()
        .take(500)
        .map(|i| {
            let item = &list.items[i];
            let icon = allowed(&item.icon).map(Icon::Spec);
            let accessories: Vec<Accessory> = item
                .accessories
                .iter()
                .map(|a| Accessory {
                    icon: allowed(&a.icon),
                    avatar: a.avatar.clone().filter(|v| !crate::images::is_url(v) || crate::images::allowed(v, hosts)),
                    ..a.clone()
                })
                .collect();
            let preview = item.detail.as_ref().map(|d| Preview {
                picture: d.image.as_ref().filter(|i| i.starts_with('/')).map(std::path::PathBuf::from),
                image_url: allowed(&d.image).filter(|i| crate::images::is_url(i)),
                icon: icon.clone(),
                tint: item.icon_tint.clone(),
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                stats: d.stats.clone(),
                markdown: d.markdown.clone(),
                metadata: d
                    .metadata
                    .iter()
                    .map(|m| proto::MetadataEntry {
                        icon: allowed(&m.icon),
                        avatar: m
                            .avatar
                            .clone()
                            .filter(|v| !crate::images::is_url(v) || crate::images::allowed(v, hosts)),
                        ..m.clone()
                    })
                    .collect(),
                ..Default::default()
            });
            Row {
                key: item.id.clone(),
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                icon,
                icon_tint: item.icon_tint.clone(),
                accessories,
                // Sections only make sense in the extension's own order.
                section: if ranked { None } else { item.section.clone() },
                actions: item.actions.iter().map(UiAction::from_proto).collect(),
                preview,
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
            let today = day_start(0);
            let entries: Vec<(Entry, Row)> = s
                .clipboard
                .entries()
                .into_iter()
                .map(|e| {
                    let kind = clip_kind(&e.text);
                    let lines = e.text.lines().count().max(1);
                    let chars = e.text.chars().count();
                    let pin_title = if e.pinned { "Unpin" } else { "Pin" };
                    let section = if e.pinned {
                        "Pinned"
                    } else if e.time >= today {
                        "Today"
                    } else if e.time >= day_start(1) {
                        "Yesterday"
                    } else if e.time >= day_start(6) {
                        "This Week"
                    } else {
                        "Older"
                    };
                    let mut metadata = vec![
                        proto::MetadataEntry {
                            label: "Type".into(),
                            value: kind.label(),
                            icon: Some(kind.icon().into()),
                            ..Default::default()
                        },
                        proto::MetadataEntry { label: "Copied".into(), value: when(e.time), ..Default::default() },
                        proto::MetadataEntry {
                            label: "Size".into(),
                            value: format!(
                                "{lines} line{} · {chars} character{}",
                                if lines > 1 { "s" } else { "" },
                                if chars > 1 { "s" } else { "" }
                            ),
                            ..Default::default()
                        },
                    ];
                    if let ClipKind::Link = kind {
                        if let Some(host) = crate::images::host(e.text.trim()) {
                            metadata.push(proto::MetadataEntry {
                                label: "Site".into(),
                                value: host.to_string(),
                                link: Some(e.text.trim().to_string()),
                                ..Default::default()
                            });
                        }
                    }
                    let mut actions: Vec<UiAction> = [
                        s.paste.map(|_| UiAction::new("Paste", Op::Paste(e.text.clone())).icon("bs-paste")),
                        Some(UiAction::new("Copy to Clipboard", Op::ClipCopy(e.id)).icon("bs-copy")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    if matches!(kind, ClipKind::Link) {
                        actions
                            .push(UiAction::new("Open Link", Op::Open(e.text.trim().to_string())).icon("bs-external"));
                    }
                    actions.extend([
                        UiAction::new(pin_title, Op::ClipPin(e.id)).icon("bs-pin").shortcut("<Ctrl><Shift>p"),
                        UiAction::new("Delete", Op::ClipDelete(e.id))
                            .icon("bs-trash")
                            .shortcut("<Ctrl>d")
                            .destructive()
                            .section("Manage"),
                        UiAction::new("Clear History", Op::ClipClear)
                            .icon("bs-trash")
                            .destructive()
                            .section("Manage")
                            .confirm("Delete all unpinned entries?"),
                    ]);
                    let (icon, color) = match &kind {
                        ClipKind::Color(c) => (format!("color:{c}"), None),
                        k => (
                            k.icon().to_string(),
                            Some(if e.pinned { "orange".to_string() } else { "gray".to_string() }),
                        ),
                    };
                    let row = Row {
                        key: format!("clip:{}", e.id),
                        title: clipboard::summary(&e.text),
                        icon: Some(Icon::Spec(icon.clone())),
                        icon_tint: color,
                        accessories: vec![Accessory { text: Some(short_ago(e.time)), ..Default::default() }],
                        section: Some(section.into()),
                        actions,
                        preview: Some(Preview {
                            icon: Some(Icon::Spec(icon)),
                            title: kind.label(),
                            subtitle: Some(when(e.time)),
                            swatch: match &kind {
                                ClipKind::Color(c) => Some(c.clone()),
                                _ => None,
                            },
                            body: matches!(kind, ClipKind::Code(_) | ClipKind::Path)
                                .then(|| e.text.chars().take(4000).collect()),
                            text: (!matches!(kind, ClipKind::Code(_) | ClipKind::Path | ClipKind::Color(_)))
                                .then(|| e.text.chars().take(4000).collect()),
                            metadata,
                            ..Default::default()
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
                    let icon = app.and_then(|a| a.icon.clone()).or(Some(Icon::Spec("bs-box".into())));
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
                        actions: vec![
                            UiAction::new("Switch to Window", Op::FocusWindow(w.id.clone())).icon("bs-arrow-right")
                        ],
                        preview: Some(Preview {
                            icon,
                            title: w.title.clone(),
                            subtitle: Some(app_name.clone()),
                            facts,
                            body: None,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    let keywords = vec![app_name, w.app_id.clone()];
                    (Entry { key: row.key.clone(), title: row.title.clone(), keywords, subtitle: None }, row)
                })
                .collect();
            if entries.is_empty() && s.gnome_helper_needed {
                return vec![gnome_helper_row()];
            }
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
        Builtin::Welcome => {
            let entries = super::welcome::rows(s, &s.desktop)
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
        Builtin::Theme => {
            let current = s.config.theme;
            let entries = [
                (Theme::System, "System", "Follow the desktop's light or dark preference", "bs-monitor"),
                (Theme::Light, "Light", "Bright and calm", "bs-sun"),
                (Theme::Dark, "Dark", "Easy on the eyes at night", "bs-moon"),
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
                    actions: vec![UiAction::new("Use Theme", Op::SetTheme(theme)).icon("bs-check")],
                    ..Default::default()
                };
                (entry(title, &[]), row)
            })
            .collect();
            ranked_rows(&mut s.ranker, query, entries)
        }
    }
}

/// What a clipboard entry looks like it is.
#[derive(Clone, PartialEq)]
pub enum ClipKind {
    Link,
    Email,
    Color(String),
    Path,
    Number,
    Code(&'static str),
    Text,
}

impl ClipKind {
    pub fn label(&self) -> String {
        match self {
            Self::Link => "Link".into(),
            Self::Email => "Email address".into(),
            Self::Color(_) => "Color".into(),
            Self::Path => "Path".into(),
            Self::Number => "Number".into(),
            Self::Code("") => "Code".into(),
            Self::Code(lang) => format!("Code · {lang}"),
            Self::Text => "Text".into(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Link => "bs-link",
            Self::Email => "bs-mail",
            Self::Color(_) => "bs-palette",
            Self::Path => "bs-folder",
            Self::Number => "bs-hash",
            Self::Code(_) => "bs-code",
            Self::Text => "bs-text",
        }
    }
}

/// Guesses what copied text is, to pick an icon and a preview.
pub fn clip_kind(text: &str) -> ClipKind {
    let t = text.trim();
    let one_line = !t.contains('\n');
    if one_line && (t.starts_with("https://") || t.starts_with("http://")) && !t.contains(' ') {
        return ClipKind::Link;
    }
    if one_line && !t.contains(' ') && t.contains('@') && t.rsplit('@').next().is_some_and(|d| d.contains('.')) {
        return ClipKind::Email;
    }
    let hex = t.strip_prefix('#').filter(|h| matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()));
    if hex.is_some()
        || (one_line && (t.starts_with("rgb(") || t.starts_with("rgba(") || t.starts_with("hsl(")) && t.ends_with(')'))
    {
        return ClipKind::Color(t.to_string());
    }
    if one_line && !t.contains(' ') && (t.starts_with('/') || t.starts_with("~/")) && t.len() > 1 {
        return ClipKind::Path;
    }
    if one_line && !t.is_empty() && t.replace([' ', ',', '.', '_'], "").chars().all(|c| c.is_ascii_digit() || c == '-')
    {
        return ClipKind::Number;
    }
    let lines: Vec<&str> = t.lines().collect();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    let lang = if has(&["fn ", "let mut ", "impl ", "pub fn", "::new("]) {
        Some("Rust")
    } else if has(&["def ", "import ", "self.", "elif "]) && !t.contains(';') {
        Some("Python")
    } else if has(&["const ", "=> ", "function ", "export ", "await "]) {
        Some("JavaScript")
    } else if t.starts_with('{') && t.ends_with('}') && t.contains("\":") {
        Some("JSON")
    } else if t.starts_with('<') && t.ends_with('>') {
        Some("HTML")
    } else if has(&["sudo ", "#!/bin", " | ", "apt ", "dnf ", "export "]) && lines.len() <= 20 {
        Some("Shell")
    } else if has(&["SELECT ", "INSERT INTO", "UPDATE "]) {
        Some("SQL")
    } else {
        None
    };
    if let Some(lang) = lang {
        return ClipKind::Code(lang);
    }
    let codey = lines
        .iter()
        .filter(|l| l.ends_with(';') || l.ends_with('{') || l.ends_with('}') || l.starts_with("    "))
        .count();
    if lines.len() > 2 && codey * 2 >= lines.len() {
        return ClipKind::Code("");
    }
    ClipKind::Text
}

/// Start of the day `days` days ago, local time, in seconds.
fn day_start(days: i64) -> u64 {
    gtk::glib::DateTime::now_local()
        .ok()
        .and_then(|now| gtk::glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 0, 0, 0.0).ok())
        .and_then(|d| d.add_days(-(days as i32)).ok())
        .map_or(0, |d| d.to_unix().max(0) as u64)
}

/// "Today at 22:41", "Yesterday at 9:05", or a date.
pub fn when(time: u64) -> String {
    let Ok(t) = gtk::glib::DateTime::from_unix_local(time as i64) else { return String::new() };
    let clock = t.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
    if time >= day_start(0) {
        format!("Today at {clock}")
    } else if time >= day_start(1) {
        format!("Yesterday at {clock}")
    } else {
        t.format("%e %b %Y at %H:%M").map(|s| s.trim().to_string()).unwrap_or_default()
    }
}

/// "now", "5m", "3h", "2d" or a short date: fits a row.
pub fn short_ago(time: u64) -> String {
    let age = now().saturating_sub(time);
    match age {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", age / 60),
        3600..=86_399 => format!("{}h", age / 3600),
        86_400..=604_799 => format!("{}d", age / 86_400),
        _ => gtk::glib::DateTime::from_unix_local(time as i64)
            .ok()
            .and_then(|d| d.format("%e %b").ok())
            .map_or_else(String::new, |s| s.trim().to_string()),
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
            UiAction::new("Open", Op::Open(target.clone())).icon("bs-external"),
            UiAction::new("Show in Folder", Op::Reveal(path.to_path_buf())).icon("bs-folder"),
            UiAction::new("Copy Path", Op::Copy(target.clone())).icon("bs-copy").shortcut("<Ctrl><Shift>c"),
        ],
        preview: Some(Preview {
            icon: Some(icon),
            title: name,
            subtitle: None,
            facts,
            picture: (content_type.starts_with("image/") && meta.as_ref().is_some_and(|m| m.len() < 20_000_000))
                .then(|| path.to_path_buf()),
            body: text_excerpt(path, &content_type),
            ..Default::default()
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

/// Lines printed by a `list` script, one item each.
pub fn script_lines(s: &mut State, query: &str) -> Vec<Row> {
    let Some(super::model::FrameKind::Script { result: Some(Ok(out)), .. }) = s.frames.last().map(|f| &f.kind) else {
        return Vec::new();
    };
    let can_paste = s.paste.is_some();
    let entries: Vec<(Entry, Row)> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            let mut actions = Vec::new();
            if can_paste {
                actions.push(UiAction::new("Paste", Op::Paste(line.to_string())).icon("bs-paste"));
            }
            actions.push(UiAction::new("Copy", Op::Copy(line.to_string())).icon("bs-copy"));
            let row = Row {
                key: format!("line:{i}"),
                title: line.to_string(),
                icon: Some(Icon::Spec("bs-text".into())),
                actions,
                ..Default::default()
            };
            (Entry { key: row.key.clone(), title: line.to_string(), keywords: Vec::new(), subtitle: None }, row)
        })
        .collect();
    ranked_rows(&mut s.ranker, query, entries)
}

/// GNOME needs a small Shell extension before windows can be listed.
pub fn gnome_helper_row() -> Row {
    let installed = backslash_core::system::gnome::is_installed();
    Row {
        key: "gnome-helper".into(),
        title: if installed { "Log out and back in once" } else { "Install the GNOME helper" }.into(),
        subtitle: Some(if installed {
            "GNOME loads the Backslash extension at the next login".into()
        } else {
            "A tiny GNOME Shell extension that lets Backslash see your windows".into()
        }),
        icon: Some(Icon::Spec("bs-window".into())),
        tint: Some("teal"),
        actions: vec![UiAction::new(if installed { "Install Again" } else { "Install" }, Op::InstallGnomeHelper)
            .icon("bs-package")],
        preview: Some(Preview {
            icon: Some(Icon::Spec("bs-window".into())),
            title: "Switch windows on GNOME".into(),
            subtitle: Some("GNOME does not let apps list windows".into()),
            body: Some(
                "Backslash ships a small GNOME Shell extension (backslash@backslash.app) that only lists your windows and switches to the one you pick. It is installed in ~/.local/share/gnome-shell/extensions and enabled; GNOME loads it at your next login.\n\nThe \"Window Calls\" extension works too, if you already have it.".into(),
            ),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{clip_kind, ClipKind};

    #[test]
    fn clipboard_kinds() {
        assert!(clip_kind("https://github.com/backslash-app") == ClipKind::Link);
        assert!(clip_kind("ada@example.com") == ClipKind::Email);
        assert!(clip_kind("#8B93FF") == ClipKind::Color("#8B93FF".into()));
        assert!(clip_kind("~/projects/backslash") == ClipKind::Path);
        assert!(clip_kind("1 234,50") == ClipKind::Number);
        assert!(clip_kind("pub fn main() {\n    let x = 1;\n}") == ClipKind::Code("Rust"));
        assert!(clip_kind("sudo ss -ltnp 'sport = :3000'") == ClipKind::Code("Shell"));
        assert!(clip_kind("Merci pour ton retour, je regarde demain.") == ClipKind::Text);
    }
}
