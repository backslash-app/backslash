//! Styling: one layout stylesheet using named colors, two palettes (light and
//! dark, each with an accent) swapped at runtime, the user's override, and the desktop's color
//! scheme, followed live when the theme is "system".

use backslash_core::config::{Accent, Config, Density, Theme};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

const LAYOUT_CSS: &str = include_str!("style.css");

/// Named colors of one appearance. Status colors come in pairs: the color
/// itself (text, icons) and a faint background for pills.
struct Palette {
    bg: &'static str,
    panel: &'static str,
    raise: &'static str,
    line: &'static str,
    fg: &'static str,
    dim: &'static str,
    faint: &'static str,
    shadow: &'static str,
    /// Selected rows: the accent at this opacity.
    selection: f32,
    /// red, orange, yellow, green, blue, purple, teal, pink, gray.
    colors: [(&'static str, &'static str); 9],
    /// Background opacity of colored pills.
    pill: f32,
}

const NAMES: [&str; 9] = ["red", "orange", "yellow", "green", "blue", "purple", "teal", "pink", "gray"];

const LIGHT: Palette = Palette {
    bg: "#ffffff",
    panel: "#f8f8fa",
    raise: "#efeff3",
    line: "rgba(0, 0, 0, 0.08)",
    fg: "#17171c",
    dim: "#55555f",
    faint: "#6b6b75",
    shadow: "rgba(20, 22, 40, 0.20)",
    selection: 0.10,
    colors: [
        ("red", "#cf222e"),
        ("orange", "#bc4c00"),
        ("yellow", "#9a6700"),
        ("green", "#1a7f37"),
        ("blue", "#0969da"),
        ("purple", "#8250df"),
        ("teal", "#1b7c83"),
        ("pink", "#bf3989"),
        ("gray", "#6b6b75"),
    ],
    pill: 0.10,
};

const DARK: Palette = Palette {
    bg: "#141417",
    panel: "#18181c",
    raise: "#24242b",
    line: "rgba(255, 255, 255, 0.08)",
    fg: "#ececf1",
    dim: "#a1a1aa",
    faint: "#8e8e98",
    shadow: "rgba(0, 0, 0, 0.55)",
    selection: 0.15,
    colors: [
        ("red", "#ff7b72"),
        ("orange", "#f0883e"),
        ("yellow", "#e3b341"),
        ("green", "#3fb950"),
        ("blue", "#58a6ff"),
        ("purple", "#bc8cff"),
        ("teal", "#39c5cf"),
        ("pink", "#f778ba"),
        ("gray", "#8e8e98"),
    ],
    pill: 0.15,
};

/// The accent of each preset, for light and dark backgrounds.
fn accent_colors(accent: Accent) -> (&'static str, &'static str) {
    match accent {
        Accent::Indigo => ("#5359e0", "#8b93ff"),
        Accent::Blue => ("#0969da", "#58a6ff"),
        Accent::Green => ("#1a7f37", "#3fb950"),
        Accent::Orange => ("#bc4c00", "#f0883e"),
        Accent::Pink => ("#bf3989", "#f778ba"),
        Accent::Graphite => ("#3d3d45", "#c6c6cf"),
    }
}

/// `#rrggbb` → `rgba(r, g, b, alpha)`.
fn with_alpha(hex: &str, alpha: f32) -> String {
    let h = hex.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0);
    format!("rgba({}, {}, {}, {alpha})", c(0), c(2), c(4))
}

/// The `@define-color` lines for an appearance and accent.
fn palette_css(dark: bool, accent: Accent) -> String {
    let p = if dark { &DARK } else { &LIGHT };
    let (light_accent, dark_accent) = accent_colors(accent);
    let accent = if dark { dark_accent } else { light_accent };
    let on_accent = if dark && accent != "#c6c6cf" { "#101015" } else { "#ffffff" };
    let mut css = format!(
        "@define-color bs_bg {};\n@define-color bs_preview_bg {};\n@define-color bs_surface {};\n\
         @define-color bs_border {};\n@define-color bs_divider {};\n@define-color bs_fg {};\n\
         @define-color bs_dim {};\n@define-color bs_faint {};\n@define-color bs_shadow {};\n\
         @define-color bs_accent {accent};\n@define-color bs_on_accent {on_accent};\n\
         @define-color bs_selected {};\n@define-color bs_accent_bg {};\n",
        p.bg,
        p.panel,
        p.raise,
        p.line,
        p.line,
        p.fg,
        p.dim,
        p.faint,
        p.shadow,
        with_alpha(accent, p.selection),
        with_alpha(accent, p.pill),
    );
    for (name, color) in p.colors {
        css.push_str(&format!(
            "@define-color bs_{name} {color};\n@define-color bs_{name}_bg {};\n",
            with_alpha(color, p.pill)
        ));
    }
    css
}

/// Color classes: `.accent-red` colors text and icons, a pill inside gets
/// the faint background.
fn color_classes() -> String {
    let mut css = String::new();
    for name in NAMES.iter().chain(["accent"].iter()) {
        css.push_str(&format!(
            ".accent-{name}, .tint-{name} glyph {{ color: @bs_{name}; }}\n\
             .accent-{name} .tag, .tag.accent-{name} {{ color: @bs_{name}; background-color: @bs_{name}_bg; }}\n\
             .tile.tint-{name} {{ background-color: @bs_{name}_bg; color: @bs_{name}; }}\n"
        ));
    }
    css
}

pub struct Styles {
    palette: gtk::CssProvider,
}

impl Styles {
    pub fn install(display: &gdk::Display) -> Self {
        let palette = gtk::CssProvider::new();
        palette.load_from_string(&palette_css(false, Accent::default()));
        gtk::style_context_add_provider_for_display(display, &palette, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let layout = gtk::CssProvider::new();
        layout.load_from_string(&format!("{LAYOUT_CSS}\n{}", color_classes()));
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

    /// Applies the appearance (resolving "system"), accent and density to
    /// the palette and to `windows`.
    pub fn apply(&self, config: &Config, windows: &[&gtk::Widget]) {
        let dark = match config.theme {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => system_prefers_dark(),
        };
        self.palette.load_from_string(&palette_css(dark, config.accent));
        for window in windows {
            window.remove_css_class(if dark { "light" } else { "dark" });
            window.add_css_class(if dark { "dark" } else { "light" });
            if config.density == Density::Compact {
                window.add_css_class("compact");
            } else {
                window.remove_css_class("compact");
            }
        }
    }
}

/// `org.freedesktop.appearance color-scheme`: 1 = prefer dark, 2 = prefer
/// light, 0 = no preference. Without a preference we look at the GTK theme
/// name, then default to light (the main design).
pub fn system_prefers_dark() -> bool {
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
