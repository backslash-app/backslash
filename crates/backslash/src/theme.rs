//! Styling: one layout stylesheet using named colors, two palettes (light and
//! dark) swapped at runtime, the user's override, and the desktop's color
//! scheme, followed live when the theme is "system".

use backslash_core::config::Theme;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

const LAYOUT_CSS: &str = include_str!("style.css");

const LIGHT: &str = "
@define-color bs_bg #ffffff;
@define-color bs_preview_bg #f8f9fb;
@define-color bs_border #dcdfe6;
@define-color bs_divider #eceef2;
@define-color bs_fg #15171c;
@define-color bs_dim #5b6170;
@define-color bs_faint #8a90a0;
@define-color bs_surface #f1f3f6;
@define-color bs_selected rgba(36, 72, 200, 0.09);
@define-color bs_accent #2448c8;
@define-color bs_on_accent #ffffff;
@define-color bs_shadow rgba(20, 24, 40, 0.18);
@define-color bs_blue_bg #e8f0ff;   @define-color bs_blue #2448c8;
@define-color bs_orange_bg #fff1e6; @define-color bs_orange #c2410c;
@define-color bs_green_bg #e3f7ea;  @define-color bs_green #157a3c;
@define-color bs_purple_bg #f1eaff; @define-color bs_purple #6d28d9;
@define-color bs_yellow_bg #fdf6dc; @define-color bs_yellow #8a6a00;
@define-color bs_red_bg #fde8e8;    @define-color bs_red #c62828;
@define-color bs_teal_bg #ddf4f2;   @define-color bs_teal #0f766e;
@define-color bs_gray_bg #eef0f3;   @define-color bs_gray #4a5060;
";

const DARK: &str = "
@define-color bs_bg #17181c;
@define-color bs_preview_bg #1c1d22;
@define-color bs_border rgba(255, 255, 255, 0.09);
@define-color bs_divider rgba(255, 255, 255, 0.06);
@define-color bs_fg #ececf1;
@define-color bs_dim #a1a5b2;
@define-color bs_faint #7d8190;
@define-color bs_surface rgba(255, 255, 255, 0.07);
@define-color bs_selected rgba(124, 156, 255, 0.15);
@define-color bs_accent #8fa8ff;
@define-color bs_on_accent #101218;
@define-color bs_shadow rgba(0, 0, 0, 0.5);
@define-color bs_blue_bg rgba(96, 140, 255, 0.16);   @define-color bs_blue #9db8ff;
@define-color bs_orange_bg rgba(255, 140, 60, 0.16); @define-color bs_orange #ffab78;
@define-color bs_green_bg rgba(74, 222, 128, 0.14);  @define-color bs_green #7be8a4;
@define-color bs_purple_bg rgba(167, 139, 250, 0.17); @define-color bs_purple #c2b0ff;
@define-color bs_yellow_bg rgba(250, 204, 21, 0.14); @define-color bs_yellow #f5d565;
@define-color bs_red_bg rgba(248, 113, 113, 0.16);   @define-color bs_red #ff9b9b;
@define-color bs_teal_bg rgba(45, 212, 191, 0.14);   @define-color bs_teal #6ee9d6;
@define-color bs_gray_bg rgba(255, 255, 255, 0.08);  @define-color bs_gray #c0c4cf;
";

pub struct Styles {
    palette: gtk::CssProvider,
}

impl Styles {
    pub fn install(display: &gdk::Display) -> Self {
        let palette = gtk::CssProvider::new();
        palette.load_from_string(LIGHT);
        gtk::style_context_add_provider_for_display(display, &palette, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let layout = gtk::CssProvider::new();
        layout.load_from_string(LAYOUT_CSS);
        gtk::style_context_add_provider_for_display(display, &layout, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let user = backslash_core::paths::config_dir().join("style.css");
        if user.is_file() {
            let provider = gtk::CssProvider::new();
            provider.connect_parsing_error(|_, section, err| {
                eprintln!("backslash: style.css:{}: {err}", section.start_location().lines() + 1);
            });
            provider.load_from_path(&user);
            gtk::style_context_add_provider_for_display(display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
        }
        Self { palette }
    }

    /// Applies `theme` (resolving "system") to the palette and `window`.
    pub fn apply(&self, theme: Theme, window: &impl IsA<gtk::Widget>) {
        let dark = match theme {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => system_prefers_dark(),
        };
        self.palette.load_from_string(if dark { DARK } else { LIGHT });
        window.remove_css_class(if dark { "light" } else { "dark" });
        window.add_css_class(if dark { "dark" } else { "light" });
    }
}

/// `org.freedesktop.appearance color-scheme`: 1 = prefer dark, 2 = prefer
/// light, 0 = no preference. Without a preference we look at the GTK theme
/// name, then default to light (the main design).
fn system_prefers_dark() -> bool {
    match portal_color_scheme() {
        Some(1) => true,
        Some(2) => false,
        _ => gtk::Settings::default().is_some_and(|s| {
            s.is_gtk_application_prefer_dark_theme()
                || s.gtk_theme_name().is_some_and(|n| n.to_lowercase().contains("dark"))
        }),
    }
}

fn portal_color_scheme() -> Option<u32> {
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let reply = bus
        .call_sync(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
            "ReadOne",
            Some(&("org.freedesktop.appearance", "color-scheme").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            300,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let mut value = reply.child_value(0);
    while let Some(inner) = value.as_variant() {
        value = inner;
    }
    value.get::<u32>()
}

/// Calls `on_change` when the desktop switches between light and dark.
pub fn watch_system(on_change: impl Fn() + 'static) {
    glib::spawn_future_local(async move {
        let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else { return };
        #[allow(deprecated)]
        bus.signal_subscribe(
            Some("org.freedesktop.portal.Desktop"),
            Some("org.freedesktop.portal.Settings"),
            Some("SettingChanged"),
            Some("/org/freedesktop/portal/desktop"),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, params| {
                let namespace = params.child_value(0);
                let key = params.child_value(1);
                if namespace.str() == Some("org.freedesktop.appearance") && key.str() == Some("color-scheme") {
                    on_change();
                }
            },
        );
        std::mem::forget(bus);
    });
}
