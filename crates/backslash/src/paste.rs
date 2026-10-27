//! Pasting into the app that had the focus: the text goes to the clipboard,
//! the launcher hides, then Ctrl+V (Ctrl+Shift+V in terminals) is sent.
//!
//! Linux has no single way to type into another app, so each session gets
//! the method that works there: `xdotool` on X11, the RemoteDesktop portal on
//! GNOME and KDE (asks once, then remembered), `wtype` on wlroots compositors
//! (Sway, Hyprland, niri), `ydotool` as a last resort.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use backslash_core::paths;
use backslash_core::system::which;
use backslash_core::system::windows::Backend;
use gtk::gio;
use gtk::glib::{self, Variant};
use gtk::prelude::*;

use crate::portal::{dict, request, DEST, PATH};

const IFACE: &str = "org.freedesktop.portal.RemoteDesktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Xdotool,
    Portal,
    Wtype,
    Ydotool,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Self::Xdotool => "xdotool",
            Self::Portal => "the desktop's remote control portal",
            Self::Wtype => "wtype",
            Self::Ydotool => "ydotool",
        }
    }
}

/// App ids and X11 classes of terminals, which paste with Ctrl+Shift+V.
const TERMINALS: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "wezterm",
    "ghostty",
    "terminator",
    "tilix",
    "konsole",
    "xterm",
    "urxvt",
    "st",
    "xfce4-terminal",
    "gnome-terminal",
    "kgx",
    "ptyxis",
    "contour",
    "rio",
    "blackbox",
];

pub fn is_terminal(app_id: &str) -> bool {
    let id = app_id.to_lowercase();
    let short = id.rsplit('.').next().unwrap_or(&id);
    TERMINALS.iter().any(|t| short == *t || short.starts_with(&format!("{t}-")) || (t.len() >= 5 && id.contains(t)))
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase()
}

/// Version of the RemoteDesktop portal; restoring a granted session needs 2.
fn portal_version() -> u32 {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else { return 0 };
    bus.call_sync(
        Some(DEST),
        PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(IFACE, "version").to_variant()),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        1000,
        None::<&gio::Cancellable>,
    )
    .ok()
    .and_then(|v| v.child_value(0).as_variant())
    .and_then(|v| v.get::<u32>())
    .unwrap_or(0)
}

/// The method for this session, if any.
pub fn detect() -> Option<Method> {
    if !wayland() {
        return which(&["xdotool"]).map(|_| Method::Xdotool);
    }
    let d = desktop();
    let portal = portal_version() >= 2;
    if portal && (d.contains("gnome") || d.contains("kde")) {
        return Some(Method::Portal);
    }
    if which(&["wtype"]).is_some() && !d.contains("gnome") && !d.contains("kde") {
        return Some(Method::Wtype);
    }
    if portal {
        return Some(Method::Portal);
    }
    which(&["ydotool"]).map(|_| Method::Ydotool)
}

/// Where the paste will land: the focused app, noted before the launcher shows.
pub fn focused_app(backend: Option<Backend>) -> Option<String> {
    if !wayland() && which(&["xdotool"]).is_some() {
        let out = Command::new("xdotool").args(["getactivewindow", "getwindowclassname"]).output().ok()?;
        return Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty());
    }
    let windows = backend?.list().ok()?;
    windows.into_iter().find(|w| w.focused).map(|w| w.app_id)
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

const XK_CONTROL_L: i32 = 0xffe3;
const XK_SHIFT_L: i32 = 0xffe1;
const XK_V: i32 = 0x76;

/// A granted RemoteDesktop session, kept for the life of the launcher.
struct Session {
    bus: gio::DBusConnection,
    handle: String,
}

#[derive(Default)]
pub struct Paster {
    session: RefCell<Option<Rc<Session>>>,
}

fn token_file() -> std::path::PathBuf {
    paths::state_dir().join("remote-desktop-token")
}

impl Paster {
    /// Sends the paste shortcut to the focused app.
    pub async fn paste(&self, method: Method, terminal: bool) -> Result<(), String> {
        // Let the compositor give the focus back to the previous window.
        glib::timeout_future(Duration::from_millis(180)).await;
        match method {
            Method::Xdotool => {
                run("xdotool", &["key", "--clearmodifiers", if terminal { "ctrl+shift+v" } else { "ctrl+v" }])
            }
            Method::Wtype => {
                let mut args = vec!["-M", "ctrl"];
                if terminal {
                    args.extend(["-M", "shift"]);
                }
                args.extend(["-k", "v"]);
                if terminal {
                    args.extend(["-m", "shift"]);
                }
                args.extend(["-m", "ctrl"]);
                run("wtype", &args)
            }
            Method::Ydotool => {
                // Linux input codes: 29 left Ctrl, 42 left Shift, 47 V.
                let keys: &[&str] = if terminal {
                    &["29:1", "42:1", "47:1", "47:0", "42:0", "29:0"]
                } else {
                    &["29:1", "47:1", "47:0", "29:0"]
                };
                let mut args = vec!["key"];
                args.extend(keys);
                run("ydotool", &args)
            }
            Method::Portal => {
                let session = self.portal_session().await?;
                let mut keys = vec![XK_CONTROL_L];
                if terminal {
                    keys.push(XK_SHIFT_L);
                }
                keys.push(XK_V);
                let result = press(&session, &keys).await;
                if result.is_err() {
                    // The desktop may have closed the session: ask again next time.
                    self.session.replace(None);
                }
                result
            }
        }
    }

    async fn portal_session(&self) -> Result<Rc<Session>, String> {
        if let Some(s) = self.session.borrow().clone() {
            return Ok(s);
        }
        let s = Rc::new(start_session().await?);
        self.session.replace(Some(s.clone()));
        Ok(s)
    }
}

/// Presses the keys in order, then releases them in reverse.
async fn press(session: &Session, keys: &[i32]) -> Result<(), String> {
    let path = glib::variant::ObjectPath::try_from(session.handle.clone()).map_err(|e| e.to_string())?;
    let send = |keysym: i32, state: u32| {
        let args = Variant::tuple_from_iter([path.to_variant(), dict(&[]), keysym.to_variant(), state.to_variant()]);
        session.bus.call_future(
            Some(DEST),
            PATH,
            IFACE,
            "NotifyKeyboardKeysym",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            2000,
        )
    };
    for &k in keys {
        send(k, 1).await.map_err(|e| e.to_string())?;
    }
    for &k in keys.iter().rev() {
        send(k, 0).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Creates a keyboard-only session. The first time, the desktop asks the user;
/// the restore token it returns lets later sessions start without asking.
async fn start_session() -> Result<Session, String> {
    let bus = gio::bus_get_future(gio::BusType::Session).await.map_err(|e| e.to_string())?;
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let token = format!("backslashrd{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));

    let created = request(
        &bus,
        IFACE,
        "CreateSession",
        vec![dict(&[("handle_token", token.to_variant()), ("session_handle_token", format!("{token}s").to_variant())])],
        &token,
    )
    .await?;
    let handle = created
        .lookup_value("session_handle", None)
        .and_then(|v| v.str().map(str::to_owned))
        .ok_or("no session handle")?;
    let path = glib::variant::ObjectPath::try_from(handle.clone()).map_err(|e| e.to_string())?;

    let restore = std::fs::read_to_string(token_file()).unwrap_or_default();
    let select_token = format!("{token}d");
    let mut options = vec![
        ("handle_token", select_token.to_variant()),
        ("types", 1u32.to_variant()),        // keyboard
        ("persist_mode", 2u32.to_variant()), // until revoked
    ];
    if !restore.trim().is_empty() {
        options.push(("restore_token", restore.trim().to_variant()));
    }
    request(&bus, IFACE, "SelectDevices", vec![path.to_variant(), dict(&options)], &select_token).await?;

    let start_token = format!("{token}t");
    let started = request(
        &bus,
        IFACE,
        "Start",
        vec![path.to_variant(), "".to_variant(), dict(&[("handle_token", start_token.to_variant())])],
        &start_token,
    )
    .await?;
    if let Some(next) = started.lookup_value("restore_token", None).and_then(|v| v.str().map(str::to_owned)) {
        let file = token_file();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, next);
    }
    Ok(Session { bus, handle })
}

#[cfg(test)]
mod tests {
    use super::is_terminal;

    #[test]
    fn terminals() {
        for id in ["kitty", "Alacritty", "org.gnome.Ptyxis", "org.kde.konsole", "foot", "com.mitchellh.ghostty"] {
            assert!(is_terminal(id), "{id}");
        }
        for id in ["firefox", "org.gnome.TextEditor", "code", "Slack", "postman", "obsidian"] {
            assert!(!is_terminal(id), "{id}");
        }
    }
}
