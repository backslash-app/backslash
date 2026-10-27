//! The welcome screen: shown on first launch, then from "Welcome to Backslash".
//! Each row is a setup step with its real state on this desktop.

use backslash_core::config::Theme;
use backslash_core::protocol::Accessory;
use backslash_core::{paths, runtime};

use super::model::{Builtin, Op, Preview, Row, UiAction};
use super::State;
use crate::hotkey::Status;
use crate::icons::Icon;

/// Created the first time the welcome screen shows.
fn marker() -> std::path::PathBuf {
    paths::state_dir().join("welcomed")
}

pub fn seen() -> bool {
    marker().exists()
}

pub fn mark_seen() {
    let path = marker();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, "");
}

/// What the welcome screen needs to know about the desktop.
pub struct Desktop {
    pub shortcut: Status,
    pub layer_shell: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Gnome,
    Kde,
    Hyprland,
    Sway,
    Other,
}

fn desktop_kind() -> Kind {
    let d = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if d.contains("gnome") {
        Kind::Gnome
    } else if d.contains("kde") {
        Kind::Kde
    } else if d.contains("hyprland") {
        Kind::Hyprland
    } else if d.contains("sway") || d.contains("i3") {
        Kind::Sway
    } else {
        Kind::Other
    }
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// How to bind `backslash` to a key by hand on this desktop.
fn manual_shortcut(kind: Kind) -> &'static str {
    match kind {
        Kind::Gnome => {
            "Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → +\nCommand: backslash\nShortcut: Super+Space (remove it from \"Switch input source\" first)"
        }
        Kind::Kde => "System Settings → Keyboard → Shortcuts → Add New → Command or Script\nCommand: backslash",
        Kind::Hyprland => "In ~/.config/hypr/hyprland.conf:\nbind = SUPER, SPACE, exec, backslash",
        Kind::Sway => "In your sway or i3 config:\nbindsym $mod+space exec backslash",
        Kind::Other => "Bind the command `backslash` to a key in your desktop's keyboard settings.",
    }
}

fn row(key: &str, title: &str, subtitle: &str, icon: &str, tint: &'static str, section: &str) -> Row {
    Row {
        key: format!("welcome:{key}"),
        title: title.into(),
        subtitle: Some(subtitle.into()),
        icon: Some(Icon::Spec(icon.into())),
        tint: Some(tint),
        section: Some(section.into()),
        ..Default::default()
    }
}

fn status(text: &str, color: &str) -> Accessory {
    Accessory { tag: Some(text.into()), color: Some(color.into()), ..Default::default() }
}

fn with_body(mut r: Row, body: String) -> Row {
    r.preview = Some(Preview {
        icon: r.icon.clone(),
        title: r.title.clone(),
        subtitle: r.subtitle.clone(),
        facts: Vec::new(),
        body: Some(body),
    });
    r
}

pub fn rows(s: &State, desktop: &Desktop) -> Vec<Row> {
    let kind = desktop_kind();
    let c = &s.config;
    let mut rows = Vec::new();

    // 1. The shortcut, the one thing that makes a launcher useful.
    let hotkey = if c.hotkey.is_empty() { "No shortcut".to_string() } else { c.hotkey.clone() };
    let (subtitle, tag, body) = match desktop.shortcut {
        Status::Active => (
            format!("{hotkey}, registered with your desktop"),
            status("Ready", "green"),
            "Your desktop owns the shortcut: it works in every app. Change it here or in your desktop's settings."
                .to_string(),
        ),
        Status::Pending => (
            format!("{hotkey}: confirm it if your desktop asks"),
            status("Waiting", "blue"),
            "Your desktop asks once whether Backslash may use this shortcut.".to_string(),
        ),
        Status::Unavailable | Status::Disabled => (
            "Bind `backslash` to a key in your desktop settings".to_string(),
            status("To do", "orange"),
            format!(
                "Your desktop does not let apps register shortcuts, so set one yourself. Running `backslash` again shows or hides the window.\n\n{}",
                manual_shortcut(kind)
            ),
        ),
    };
    let mut shortcut = row("shortcut", "Open Backslash with a shortcut", &subtitle, "input-keyboard", "blue", "Set up");
    shortcut.accessories = vec![tag];
    shortcut.actions = match desktop.shortcut {
        Status::Active | Status::Pending => {
            vec![UiAction::new("Change Shortcut…", Op::EditHotkey).icon("input-keyboard")]
        }
        _ => {
            let mut a = Vec::new();
            match kind {
                Kind::Gnome => a.push(
                    UiAction::new(
                        "Open Keyboard Settings",
                        Op::Spawn("gnome-control-center".into(), vec!["keyboard".into()]),
                    )
                    .icon("preferences-desktop-keyboard"),
                ),
                Kind::Kde => a.push(
                    UiAction::new(
                        "Open Shortcut Settings",
                        Op::Spawn("systemsettings".into(), vec!["kcm_keys".into()]),
                    )
                    .icon("preferences-desktop-keyboard"),
                ),
                _ => {}
            }
            a.push(UiAction::new("Copy Command", Op::Copy("backslash".into())).icon("edit-copy"));
            a
        }
    };
    rows.push(with_body(shortcut, body));

    // 2. Look.
    let theme = match c.theme {
        Theme::System => "Follows your system",
        Theme::Light => "Light",
        Theme::Dark => "Dark",
    };
    let mut look = row("theme", "Choose light or dark", theme, "preferences-desktop-appearance", "yellow", "Set up");
    look.actions = vec![UiAction::new("Change Theme", Op::OpenBuiltin(Builtin::Theme))];
    rows.push(look);

    // 3. Start with the session, so the first press is instant.
    let mut login = row(
        "autostart",
        "Start with your session",
        if s.autostart { "On: always ready" } else { "Off: starts on first use" },
        "system-run",
        "green",
        "Set up",
    );
    login.accessories = vec![if s.autostart { status("On", "green") } else { status("Off", "gray") }];
    login.actions =
        vec![UiAction::new(if s.autostart { "Turn Off" } else { "Turn On" }, Op::SetAutostart(!s.autostart))];
    rows.push(login);

    // 4. Clipboard history, and what it needs on Wayland.
    let watcher = !wayland() || backslash_core::system::which(&["wl-paste"]).is_some();
    let mut clip = row(
        "clipboard",
        "Clipboard history",
        if !c.clipboard.enabled {
            "Off"
        } else if watcher || kind == Kind::Gnome {
            "On: everything you copy, searchable"
        } else {
            "Install wl-clipboard to record in the background"
        },
        "edit-paste",
        "purple",
        "Set up",
    );
    clip.accessories = vec![if !c.clipboard.enabled {
        status("Off", "gray")
    } else if watcher || kind == Kind::Gnome {
        status("On", "green")
    } else {
        status("Needs wl-clipboard", "orange")
    }];
    clip.actions = vec![UiAction::new(
        if c.clipboard.enabled { "Turn Off" } else { "Turn On" },
        Op::SetClipboard(!c.clipboard.enabled),
    )];
    if c.clipboard.enabled && !watcher && kind != Kind::Gnome {
        clip.actions.push(UiAction::new("Copy Package Name", Op::Copy("wl-clipboard".into())).icon("edit-copy"));
    }
    rows.push(with_body(
        clip,
        "Copied text is kept on this computer only, in Backslash's data folder. Text copied from password managers is skipped. Pin what you reuse."
            .into(),
    ));

    // 5. Overlay window on wlroots-style compositors.
    if wayland() && kind != Kind::Gnome && !desktop.layer_shell {
        let mut overlay = row(
            "overlay",
            "Show as an overlay",
            "Install gtk4-layer-shell, or make the window float",
            "preferences-system-windows",
            "teal",
            "Set up",
        );
        overlay.accessories = vec![status("Optional", "gray")];
        overlay.actions =
            vec![UiAction::new("Copy Package Name", Op::Copy("gtk4-layer-shell".into())).icon("edit-copy")];
        rows.push(with_body(
            overlay,
            "With gtk4-layer-shell installed, Backslash opens above everything, centered, on Sway, Hyprland, KDE and other Wayland desktops. Without it, a window rule does the job:\n\nHyprland:\nwindowrulev2 = float, class:^(app\\.backslash\\.Backslash)$\n\nSway:\nfor_window [app_id=\"app.backslash.Backslash\"] floating enable".into(),
        ));
    }

    // 6. Runtime of TypeScript extensions.
    let deno = runtime::is_installed();
    let mut rt = row(
        "runtime",
        "Extensions runtime",
        if deno { "Deno is installed" } else { "Deno, about 40 MB, downloaded when first needed" },
        "application-x-executable",
        "gray",
        "Set up",
    );
    rt.accessories = vec![if s.runtime_busy {
        status("Downloading…", "blue")
    } else if deno {
        status("Ready", "green")
    } else {
        status("Optional", "gray")
    }];
    if !deno {
        rt.actions = vec![UiAction::new("Download Now", Op::InstallRuntime).icon("folder-download")];
    }
    rows.push(with_body(
        rt,
        "Extensions run in Deno, which sandboxes each of them: they only get the permissions they declare, and you approve them once.".into(),
    ));

    // Then: what to try.
    let mut store = row(
        "store",
        "Add extensions",
        "AI, browser history, media, projects, ports, Wi-Fi and Bluetooth…",
        "system-software-install",
        "blue",
        "Then",
    );
    store.actions = vec![UiAction::new("Browse Extensions", Op::OpenBuiltin(Builtin::Store))];
    rows.push(store);

    let mut tips = row("tips", "Make it yours", "A few keys worth knowing", "help-about", "teal", "Then");
    tips.actions = vec![UiAction::new("Open Settings", Op::OpenBuiltin(Builtin::Settings))];
    rows.push(with_body(
        tips,
        "Type anything: apps, commands, 12*7, 5 km in miles, 20 € in $.\nTab: filter apps or commands.\nCtrl+K: every action of the selected item.\nCtrl+K → Set Alias…: type \"ff\" to get Firefox first.\nCtrl+K → Set Hotkey…: open anything with its own shortcut.\nEsc: back, then close."
            .into(),
    ));

    let mut done = row(
        "done",
        "Start using Backslash",
        "You can come back here with \"Welcome\"",
        "object-select",
        "green",
        "Then",
    );
    done.actions = vec![UiAction::new("Get Started", Op::FinishWelcome).icon("object-select")];
    rows.push(done);

    rows
}
