//! Watches the clipboard for the history.
//!
//! Wayland only lets the focused window read the clipboard, except through
//! the data-control protocol (wlroots compositors, KDE, Hyprland…) that
//! `wl-paste --watch` uses. Where that is missing (GNOME) or on X11, GTK's own
//! clipboard notifications are used, and the launcher also records the
//! clipboard each time it opens.
//!
//! Text is preferred; an image is recorded when nothing textual is offered
//! (a screenshot, "Copy Image" in a browser).

use std::io::Read;
use std::process::{Command, Stdio};

use gtk::prelude::*;
use gtk::{gdk, glib};

/// Something copied.
pub enum Copied {
    Text(String),
    /// PNG data and its size.
    Image(Vec<u8>, u32, u32),
}

/// Calls `on_copy` with everything new copied, on the GTK thread.
pub fn watch(display: &gdk::Display, on_copy: impl Fn(Copied) + 'static) {
    let on_copy = std::rc::Rc::new(on_copy);
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland && backslash_core::system::which(&["wl-paste"]).is_some() {
        let (tx, rx) = async_channel::unbounded::<Option<Copied>>();
        std::thread::spawn(move || watch_wl_paste(tx));
        let fallback_display = display.clone();
        let on_copy_fallback = on_copy.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Some(copied) => on_copy(copied),
                    // wl-paste refused: the compositor lacks data-control.
                    None => {
                        watch_gdk(&fallback_display, on_copy_fallback.clone());
                        break;
                    }
                }
            }
        });
    } else {
        watch_gdk(display, on_copy);
    }
}

fn watch_gdk(display: &gdk::Display, on_copy: std::rc::Rc<impl Fn(Copied) + 'static>) {
    display.clipboard().connect_changed(move |clipboard| {
        let on_copy = on_copy.clone();
        let clipboard = clipboard.clone();
        glib::spawn_future_local(async move {
            if let Some(copied) = read(&clipboard).await {
                on_copy(copied);
            }
        });
    });
}

/// What the clipboard holds, unless a password manager marked it.
async fn read(clipboard: &gdk::Clipboard) -> Option<Copied> {
    let formats = clipboard.formats();
    if is_sensitive(&formats) {
        return None;
    }
    let textual =
        formats.mime_types().iter().any(|m| m.starts_with("text/plain")) || formats.contains_type(glib::Type::STRING);
    let image = formats.mime_types().iter().any(|m| m.starts_with("image/"));
    if !textual && image {
        let texture = clipboard.read_texture_future().await.ok().flatten()?;
        let png = texture.save_to_png_bytes();
        return Some(Copied::Image(png.to_vec(), texture.width() as u32, texture.height() as u32));
    }
    if !textual {
        return None;
    }
    let text = clipboard.read_text_future().await.ok().flatten()?;
    // Binary data offered as text by a careless app.
    (!text.contains('\0') && !text.contains('\u{fffd}')).then(|| Copied::Text(text.to_string()))
}

/// Password managers mark their copies so clipboard managers skip them.
fn is_sensitive(formats: &gdk::ContentFormats) -> bool {
    formats.contain_mime_type("x-kde-passwordManagerHint")
}

/// Reads the clipboard once (when the launcher opens), for desktops where
/// background watching is impossible.
pub fn read_now(display: &gdk::Display, on_copy: impl Fn(Copied) + 'static) {
    let clipboard = display.clipboard();
    glib::spawn_future_local(async move {
        if let Some(copied) = read(&clipboard).await {
            on_copy(copied);
        }
    });
}

/// `wl-paste --watch` prints one byte per clipboard change; each change is
/// then read with a separate `wl-paste`, skipping password-manager copies.
fn watch_wl_paste(tx: async_channel::Sender<Option<Copied>>) {
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
        if types.contains("x-kde-passwordManagerHint") {
            continue;
        }
        if types.lines().any(|t| t.starts_with("text/") || t.contains("STRING")) {
            if let Ok(out) = Command::new("wl-paste").args(["--no-newline", "--type", "text"]).output() {
                if out.status.success() {
                    let _ = tx.send_blocking(Some(Copied::Text(String::from_utf8_lossy(&out.stdout).into_owned())));
                }
            }
        } else if types.lines().any(|t| t == "image/png") {
            if let Ok(out) = Command::new("wl-paste").args(["--type", "image/png"]).output() {
                if let Some((w, h)) = backslash_core::system::clipboard::png_size(&out.stdout) {
                    let _ = tx.send_blocking(Some(Copied::Image(out.stdout, w, h)));
                }
            }
        }
    }
    // The watcher ended at once or failed: let the caller fall back.
    let _ = child.wait();
    let _ = tx.send_blocking(None);
}
