//! Turns protocol icon strings into widgets: Backslash's own vector icons
//! (`bs-…`), theme icons, files, pictures from the web, emoji and color
//! swatches (`color:#8b93ff`).

use gtk::prelude::*;
use gtk::{gdk, gio};

use crate::glyphs::{self, Glyph};
use crate::images;

#[derive(Clone, Debug)]
pub enum Icon {
    /// Theme icon name, `bs-…`, absolute path, URL, emoji or `color:…`, as sent by extensions.
    Spec(String),
    /// From a `.desktop` file.
    Gio(gio::Icon),
}

impl Icon {
    pub fn named(name: &str) -> Option<Self> {
        Some(Self::Spec(name.to_string()))
    }
}

/// A plain icon of `size` pixels.
pub fn widget(icon: Option<&Icon>, size: i32) -> gtk::Widget {
    match icon {
        Some(Icon::Gio(icon)) => image(|| gtk::Image::from_gicon(icon), size),
        Some(Icon::Spec(spec)) if glyphs::exists(spec) => Glyph::new(spec, size).upcast(),
        Some(Icon::Spec(spec)) if is_emoji(spec) => {
            let label = gtk::Label::new(Some(spec));
            label.add_css_class("emoji-icon");
            label.set_size_request(size, size);
            label.upcast()
        }
        Some(Icon::Spec(spec)) if spec.starts_with("color:") => swatch(&spec["color:".len()..], size),
        Some(Icon::Spec(spec)) if images::is_url(spec) => match images::texture(spec) {
            Some(texture) => picture(&texture, size, 4),
            None => Glyph::new("bs-image", size * 3 / 4).upcast(),
        },
        Some(Icon::Spec(spec)) if spec.starts_with('/') => image(|| gtk::Image::from_file(spec), size),
        Some(Icon::Spec(spec)) if has_icon(spec) => image(|| gtk::Image::from_icon_name(spec), size),
        // Many themes (Adwaita included) only ship symbolic variants nowadays.
        Some(Icon::Spec(spec)) if has_icon(&format!("{spec}-symbolic")) => {
            image(|| gtk::Image::from_icon_name(&format!("{spec}-symbolic")), size)
        }
        // Unknown names fall back to a neutral icon instead of a broken image.
        Some(Icon::Spec(_)) | None => Glyph::new("bs-apps", size * 3 / 4).upcast(),
    }
}

/// Whether the icon is drawn in a single color (and can take a tint).
pub fn is_symbolic(icon: Option<&Icon>) -> bool {
    match icon {
        Some(Icon::Spec(spec)) => {
            glyphs::exists(spec) || spec.ends_with("-symbolic") || !(is_emoji(spec) || spec.contains(['/', ':']))
        }
        _ => false,
    }
}

/// An icon in a rounded tile. App icons (full color) sit on no tile; named
/// icons use their symbolic variant, colored by the tile's tint.
pub fn tile(icon: Option<&Icon>, tint: Option<&str>, size: i32) -> gtk::Widget {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("tile");
    tile.set_size_request(size, size);
    tile.set_valign(gtk::Align::Center);
    tile.set_halign(gtk::Align::Center);
    let inner = match icon {
        Some(Icon::Gio(_)) => {
            tile.add_css_class("app");
            widget(icon, size)
        }
        Some(Icon::Spec(spec)) if !is_symbolic(icon) => {
            tile.add_css_class("app");
            widget(icon, if spec.starts_with("color:") { size - 6 } else { size - 2 })
        }
        Some(Icon::Spec(spec)) => {
            tile.add_css_class(&format!("tint-{}", tint.unwrap_or("gray")));
            if glyphs::exists(spec) {
                Glyph::new(spec, size * 9 / 14).upcast()
            } else {
                let symbolic = if spec.ends_with("-symbolic") { spec.clone() } else { format!("{spec}-symbolic") };
                let name = if has_icon(&symbolic) { symbolic } else { spec.clone() };
                widget(Some(&Icon::Spec(name)), size * 9 / 14)
            }
        }
        None => {
            tile.add_css_class(&format!("tint-{}", tint.unwrap_or("gray")));
            Glyph::new("bs-apps", size * 9 / 14).upcast()
        }
    };
    // Centered in a fixed-size tile; nothing may set hexpand here, or the tile
    // would grow and push the row's text to the middle.
    inner.set_halign(gtk::Align::Center);
    inner.set_valign(gtk::Align::Center);
    tile.set_homogeneous(true);
    tile.append(&inner);
    tile.upcast()
}

/// A colored icon without a tile: statuses, priorities, item icons.
pub fn plain(icon: Option<&Icon>, tint: Option<&str>, size: i32) -> gtk::Widget {
    let w = widget(icon, size);
    if is_symbolic(icon) {
        w.add_css_class("plain-icon");
        if let Some(tint) = tint {
            w.add_css_class(&format!("accent-{tint}"));
        }
    }
    w.set_valign(gtk::Align::Center);
    w
}

fn swatch(color: &str, size: i32) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("swatch");
    b.set_size_request(size, size);
    b.set_valign(gtk::Align::Center);
    if let Ok(rgba) = gdk::RGBA::parse(color) {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!("box {{ background-color: {rgba}; }}"));
        #[allow(deprecated)]
        b.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    b.upcast()
}

/// A big color sample for the preview panel.
pub fn big_swatch(color: &str) -> gtk::Widget {
    let w = swatch(color, 120);
    w.add_css_class("big-swatch");
    w.set_hexpand(true);
    w.set_valign(gtk::Align::Fill);
    w
}

fn picture(texture: &gdk::Texture, size: i32, radius: i32) -> gtk::Widget {
    let p = gtk::Picture::for_paintable(texture);
    p.set_size_request(size, size);
    p.set_content_fit(gtk::ContentFit::Cover);
    p.set_can_shrink(true);
    p.set_valign(gtk::Align::Center);
    let frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_valign(gtk::Align::Center);
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("box {{ border-radius: {radius}px; }}"));
    #[allow(deprecated)]
    frame.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    frame.append(&p);
    frame.upcast()
}

/// Initials of a name: "Ada Lovelace" → "AL", "octocat" → "OC".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> =
        name.split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-').filter(|w| !w.is_empty()).collect();
    let first = |w: &str| w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [a, .., b] => first(a) + &first(b),
    }
}

/// A person: their picture, or their initials on a color picked from the name.
pub fn avatar(spec: &str, size: i32) -> gtk::Widget {
    if images::is_url(spec) || spec.starts_with('/') {
        let texture =
            if spec.starts_with('/') { gdk::Texture::from_filename(spec).ok() } else { images::texture(spec) };
        if let Some(t) = texture {
            return picture(&t, size, size / 2);
        }
    }
    let name = if images::is_url(spec) { "?" } else { spec };
    let l = gtk::Label::new(Some(&initials(name)));
    l.add_css_class("avatar");
    if size > 22 {
        l.add_css_class("big");
    }
    let hash = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    l.add_css_class(&format!("c{}", hash % 8));
    l.set_size_request(size, size);
    l.set_valign(gtk::Align::Center);
    l.set_tooltip_text(Some(name));
    l.upcast()
}

fn has_icon(name: &str) -> bool {
    gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name))
}

fn image(make: impl FnOnce() -> gtk::Image, size: i32) -> gtk::Widget {
    let image = make();
    image.set_pixel_size(size);
    image.upcast()
}

/// One or two code points outside ASCII with no path or dash: treat as emoji.
pub fn is_emoji(s: &str) -> bool {
    let n = s.chars().count();
    (1..=4).contains(&n) && s.chars().next().is_some_and(|c| !c.is_ascii() && !c.is_alphanumeric())
}

/// A pill, a person, a colored icon or dimmed text on the right of a row.
pub fn accessory(a: &backslash_core::protocol::Accessory) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    b.add_css_class("accessory");
    b.set_valign(gtk::Align::Center);
    if let Some(color) = &a.color {
        b.add_css_class(&format!("accent-{}", css_name(color)));
    }
    if let Some(icon) = &a.icon {
        let icon = Icon::Spec(icon.clone());
        b.append(&plain(Some(&icon), None, 14));
    }
    if let Some(text) = &a.text {
        b.append(&gtk::Label::new(Some(text)));
    }
    if let Some(tag) = &a.tag {
        let l = gtk::Label::new(Some(tag));
        l.add_css_class("tag");
        b.append(&l);
    }
    if let Some(person) = &a.avatar {
        b.append(&avatar(person, 20));
    }
    if let Some(tip) = &a.tooltip {
        b.set_tooltip_text(Some(tip));
    }
    b.upcast()
}

/// Keeps color names safe to use as CSS classes.
pub fn css_name(color: &str) -> String {
    color.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn emoji_detection() {
        assert!(super::is_emoji("👋"));
        assert!(super::is_emoji("🇫🇷"));
        assert!(!super::is_emoji("folder"));
        assert!(!super::is_emoji("é"));
        assert!(!super::is_emoji("/usr/share/icons/x.png"));
    }

    #[test]
    fn initials() {
        assert_eq!(super::initials("Ada Lovelace"), "AL");
        assert_eq!(super::initials("octocat"), "OC");
        assert_eq!(super::initials("steven.persia"), "SP");
        assert_eq!(super::initials(""), "?");
    }
}
