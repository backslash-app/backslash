//! Styling: one layout stylesheet using named colors, palettes (light and
//! dark, plus named themes such as Nord or Dracula) swapped at runtime with an
//! accent, the user's override, and the desktop's color scheme, followed live
//! when the theme is "system".

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
    /// The theme's own accent (`Accent::Theme`).
    accent: &'static str,
    /// Text on that accent.
    on_accent: &'static str,
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
    accent: "#5359e0",
    on_accent: "#ffffff",
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
    accent: "#8b93ff",
    on_accent: "#101015",
};

/// A dark named theme: its base colors, then red, orange, yellow, green,
/// blue, purple, teal, pink and gray.
const fn dark(base: [&'static str; 6], colors: [&'static str; 9], accent: &'static str) -> Palette {
    named(base, colors, accent, true)
}

const fn light(base: [&'static str; 6], colors: [&'static str; 9], accent: &'static str) -> Palette {
    named(base, colors, accent, false)
}

const fn named(base: [&'static str; 6], c: [&'static str; 9], accent: &'static str, dark: bool) -> Palette {
    Palette {
        bg: base[0],
        panel: base[1],
        raise: base[2],
        line: if dark { "rgba(255, 255, 255, 0.08)" } else { "rgba(0, 0, 0, 0.08)" },
        fg: base[3],
        dim: base[4],
        faint: base[5],
        shadow: if dark { "rgba(0, 0, 0, 0.55)" } else { "rgba(20, 22, 40, 0.20)" },
        selection: if dark { 0.18 } else { 0.12 },
        colors: [
            ("red", c[0]),
            ("orange", c[1]),
            ("yellow", c[2]),
            ("green", c[3]),
            ("blue", c[4]),
            ("purple", c[5]),
            ("teal", c[6]),
            ("pink", c[7]),
            ("gray", c[8]),
        ],
        pill: if dark { 0.16 } else { 0.12 },
        accent,
        on_accent: if dark { "#101015" } else { "#ffffff" },
    }
}

const NORD: Palette = dark(
    ["#2e3440", "#2b303b", "#3b4252", "#eceff4", "#c2c9d6", "#8f9ab0"],
    ["#bf616a", "#d08770", "#ebcb8b", "#a3be8c", "#81a1c1", "#b48ead", "#8fbcbb", "#c895b5", "#8f9ab0"],
    "#88c0d0",
);
const DRACULA: Palette = dark(
    ["#282a36", "#21222c", "#343746", "#f8f8f2", "#c5c6d6", "#8c93b8"],
    ["#ff5555", "#ffb86c", "#f1fa8c", "#50fa7b", "#82aaff", "#bd93f9", "#8be9fd", "#ff79c6", "#8c93b8"],
    "#bd93f9",
);
const MOCHA: Palette = dark(
    ["#1e1e2e", "#181825", "#313244", "#cdd6f4", "#bac2de", "#9399b2"],
    ["#f38ba8", "#fab387", "#f9e2af", "#a6e3a1", "#89b4fa", "#cba6f7", "#94e2d5", "#f5c2e7", "#9399b2"],
    "#cba6f7",
);
const LATTE: Palette = light(
    ["#eff1f5", "#e6e9ef", "#dce0e8", "#4c4f69", "#5c5f77", "#6c6f85"],
    ["#d20f39", "#fe640b", "#df8e1d", "#40a02b", "#1e66f5", "#8839ef", "#179299", "#ea76cb", "#7c7f93"],
    "#8839ef",
);
const GRUVBOX: Palette = dark(
    ["#282828", "#1d2021", "#3c3836", "#ebdbb2", "#d5c4a1", "#a89984"],
    ["#fb4934", "#fe8019", "#fabd2f", "#b8bb26", "#83a598", "#d3869b", "#8ec07c", "#d3869b", "#a89984"],
    "#fabd2f",
);
const SOLARIZED: Palette = light(
    ["#fdf6e3", "#eee8d5", "#e6dfc8", "#073642", "#586e75", "#657b83"],
    ["#dc322f", "#cb4b16", "#b58900", "#859900", "#268bd2", "#6c71c4", "#2aa198", "#d33682", "#839496"],
    "#268bd2",
);
const TOKYO: Palette = dark(
    ["#1a1b26", "#16161e", "#292e42", "#c0caf5", "#a9b1d6", "#7f86a8"],
    ["#f7768e", "#ff9e64", "#e0af68", "#9ece6a", "#7aa2f7", "#bb9af7", "#73daca", "#ff8fc6", "#7f86a8"],
    "#7aa2f7",
);

/// The palette of a theme (System resolved to light or dark).
fn palette(theme: Theme, system_dark: bool) -> &'static Palette {
    match theme {
        Theme::System if system_dark => &DARK,
        Theme::System | Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
        Theme::Nord => &NORD,
        Theme::Dracula => &DRACULA,
        Theme::CatppuccinMocha => &MOCHA,
        Theme::CatppuccinLatte => &LATTE,
        Theme::Gruvbox => &GRUVBOX,
        Theme::SolarizedLight => &SOLARIZED,
        Theme::TokyoNight => &TOKYO,
    }
}

/// The background, text and accent of a theme, for previews in the settings.
pub fn swatch(theme: Theme) -> (&'static str, &'static str, &'static str) {
    let p = palette(theme, false);
    (p.bg, p.fg, p.accent)
}

/// The accent of each preset, for light and dark backgrounds
/// (`Accent::Theme` is resolved by the palette).
fn accent_colors(accent: Accent) -> (&'static str, &'static str) {
    match accent {
        Accent::Indigo | Accent::Theme => ("#5359e0", "#8b93ff"),
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

/// The `@define-color` lines for a theme and accent.
fn palette_css(theme: Theme, system_dark: bool, accent: Accent) -> String {
    let p = palette(theme, system_dark);
    let dark = theme.is_dark().unwrap_or(system_dark);
    let (accent, on_accent) = if accent == Accent::Theme {
        (p.accent, p.on_accent)
    } else {
        let (light_accent, dark_accent) = accent_colors(accent);
        let accent = if dark { dark_accent } else { light_accent };
        (accent, if dark && accent != "#c6c6cf" { "#101015" } else { "#ffffff" })
    };
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
        palette.load_from_string(&palette_css(Theme::Light, false, Accent::default()));
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
        let system_dark = config.theme == Theme::System && system_prefers_dark();
        let dark = config.theme.is_dark().unwrap_or(system_dark);
        self.palette.load_from_string(&palette_css(config.theme, system_dark, config.accent));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_defines_every_color() {
        for theme in Theme::ALL {
            for accent in Accent::ALL {
                let css = palette_css(theme, true, accent);
                for name in NAMES {
                    assert!(css.contains(&format!("@define-color bs_{name} #")), "{theme:?} {name}");
                }
                assert!(css.contains("@define-color bs_accent #"), "{theme:?} {accent:?}");
            }
        }
    }

    #[test]
    fn theme_accent_follows_the_theme() {
        assert!(palette_css(Theme::Dracula, false, Accent::Theme).contains("bs_accent #bd93f9"));
        assert!(palette_css(Theme::Dracula, false, Accent::Blue).contains("bs_accent #58a6ff"));
        assert!(palette_css(Theme::SolarizedLight, true, Accent::Blue).contains("bs_accent #0969da"));
        assert!(palette_css(Theme::System, true, Accent::Theme).contains("bs_bg #141417"));
    }
}
