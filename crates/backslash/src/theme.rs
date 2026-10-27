//! Styling: built-in CSS, the user's override, and the desktop color scheme.

use gtk::prelude::*;
use gtk::{gdk, gio};

const DEFAULT_CSS: &str = include_str!("style.css");

pub fn install(display: &gdk::Display) {
    let base = gtk::CssProvider::new();
    base.load_from_string(DEFAULT_CSS);
    gtk::style_context_add_provider_for_display(display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let user = backslash_core::paths::config_dir().join("style.css");
    if user.is_file() {
        let provider = gtk::CssProvider::new();
        provider.connect_parsing_error(|_, section, err| {
            eprintln!("backslash: style.css:{}: {err}", section.start_location().lines() + 1);
        });
        provider.load_from_path(&user);
        gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
    }
}

/// Reads `org.freedesktop.appearance color-scheme` from the settings portal:
/// 1 = prefer dark, 2 = prefer light, 0 = no preference (we default to dark).
pub fn prefers_light() -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    let reply = bus.call_sync(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "ReadOne",
        Some(&("org.freedesktop.appearance", "color-scheme").to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        300,
        gio::Cancellable::NONE,
    );
    let Ok(reply) = reply else { return false };
    // Reply is (v) wrapping a u32; unwrap nested variants defensively.
    let mut value = reply.child_value(0);
    while let Some(inner) = value.as_variant() {
        value = inner;
    }
    value.get::<u32>() == Some(2)
}

pub fn apply_scheme(window: &impl IsA<gtk::Widget>) {
    if prefers_light() {
        window.add_css_class("light");
    } else {
        window.remove_css_class("light");
    }
}
