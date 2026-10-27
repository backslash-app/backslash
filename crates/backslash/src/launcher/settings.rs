//! Forms built by the launcher (preferences, aliases, shortcuts) and the
//! approval and runtime screens. Settings themselves live in their window.

use backslash_core::config::Config;
use backslash_core::manifest::{Manifest, PreferenceKind};
use backslash_core::protocol::{Field, FieldKind, FormView};
use backslash_core::{approvals, runtime, secrets};
use serde_json::Value;

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

pub fn hotkey_form(current: &str) -> FormView {
    FormView {
        id: None,
        title: Some("Global Shortcut".into()),
        submit_title: Some("Save".into()),
        fields: vec![text("hotkey", "Shortcut (empty to disable)", current, "Super+Space", false)],
    }
}

pub fn alias_form(title: &str, current: &str) -> FormView {
    FormView {
        id: None,
        title: Some(format!("Alias for {title}")),
        submit_title: Some("Save".into()),
        fields: vec![text("alias", "Typing this shows it first (empty to remove)", current, "ff", false)],
    }
}

pub fn key_hotkey_form(title: &str, current: &str) -> FormView {
    FormView {
        id: None,
        title: Some(format!("Hotkey for {title}")),
        submit_title: Some("Save".into()),
        fields: vec![text("hotkey", "Global shortcut (empty to remove)", current, "Super+V", false)],
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

/// The screen asking to download Deno.
pub fn runtime_markdown(busy: bool) -> String {
    if busy {
        return format!(
            "# Downloading Deno {}…\n\nAbout 40 MB. The command starts as soon as it is ready.",
            runtime::DENO_VERSION
        );
    }
    format!(
        "# This extension needs Deno\n\n\
         Extensions written in TypeScript run in **Deno**, which keeps each of them in a sandbox \
         with only the permissions it declares.\n\n\
         Backslash can download Deno {} for you (about 40 MB) into `~/.local/share/backslash/runtime`. \
         It comes from Deno's official npm package and is checked before use.\n\n\
         Prefer your package manager? Install `deno` with it and run the command again.\n\n\
         Press **Enter** to download, **Esc** to cancel.",
        runtime::DENO_VERSION
    )
}
