//! Watches the clipboard for the history.
//!
//! Wayland only lets the focused window read the clipboard, except through
//! the data-control protocol (wlroots compositors, KDE, Hyprland…) that
//! `wl-paste --watch` uses. Where that is missing (GNOME) or on X11, GTK's own
//! clipboard notifications are used, and the launcher also records the
//! clipboard each time it opens.

use std::io::Read;
use std::process::{Command, Stdio};

use gtk::prelude::*;
use gtk::{gdk, glib};

/// Calls `on_text` with every new text copied, on the GTK thread.
pub fn watch(display: &gdk::Display, on_text: impl Fn(String) + 'static) {
    let on_text = std::rc::Rc::new(on_text);
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland && backslash_core::system::which(&["wl-paste"]).is_some() {
        let (tx, rx) = async_channel::unbounded::<Option<String>>();
        std::thread::spawn(move || watch_wl_paste(tx));
        let fallback_display = display.clone();
        let on_text_fallback = on_text.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Some(text) => on_text(text),
                    // wl-paste refused: the compositor lacks data-control.
                    None => {
                        watch_gdk(&fallback_display, on_text_fallback.clone());
                        break;
                    }
                }
            }
        });
    } else {
        watch_gdk(display, on_text);
    }
}

fn watch_gdk(display: &gdk::Display, on_text: std::rc::Rc<impl Fn(String) + 'static>) {
    display.clipboard().connect_changed(move |clipboard| {
        if is_sensitive(&clipboard.formats()) {
            return;
        }
        let on_text = on_text.clone();
        let clipboard = clipboard.clone();
        glib::spawn_future_local(async move {
            if let Ok(Some(text)) = clipboard.read_text_future().await {
                on_text(text.to_string());
            }
        });
    });
}

/// Password managers mark their copies so clipboard managers skip them.
fn is_sensitive(formats: &gdk::ContentFormats) -> bool {
    formats.contain_mime_type("x-kde-passwordManagerHint")
}

/// Reads the clipboard once (when the launcher opens), for desktops where
/// background watching is impossible.
pub fn read_now(display: &gdk::Display, on_text: impl Fn(String) + 'static) {
    let clipboard = display.clipboard();
    if is_sensitive(&clipboard.formats()) {
        return;
    }
    glib::spawn_future_local(async move {
        if let Ok(Some(text)) = clipboard.read_text_future().await {
            on_text(text.to_string());
        }
    });
}

/// `wl-paste --watch` prints one byte per clipboard change; each change is
/// then read with a separate `wl-paste`, skipping password-manager copies.
fn watch_wl_paste(tx: async_channel::Sender<Option<String>>) {
    let child = Command::new("wl-paste")
        .args(["--watch", "sh", "-c", "printf x"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        let _ = tx.send_blocking(None);
        return;
    };
    let mut stdout = child.stdout.take().expect("piped");
    let mut byte = [0u8; 1];
    while stdout.read(&mut byte).is_ok_and(|n| n == 1) {
        let types = Command::new("wl-paste").arg("--list-types").output();
        let Ok(types) = types else { continue };
        let types = String::from_utf8_lossy(&types.stdout);
        if types.contains("x-kde-passwordManagerHint")
            || !types.lines().any(|t| t.starts_with("text/") || t.contains("STRING"))
        {
            continue;
        }
        if let Ok(out) = Command::new("wl-paste").args(["--no-newline", "--type", "text"]).output() {
            if out.status.success() {
                let _ = tx.send_blocking(Some(String::from_utf8_lossy(&out.stdout).into_owned()));
            }
        }
    }
    // The watcher ended at once or failed: let the caller fall back.
    let _ = child.wait();
    let _ = tx.send_blocking(None);
}
