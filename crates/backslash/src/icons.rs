//! Turns protocol icon strings into widgets.

use gtk::prelude::*;
use gtk::{gio, glib};

#[derive(Clone, Debug)]
pub enum Icon {
    /// Theme icon name, absolute path or emoji, as sent by extensions.
    Spec(String),
    /// From a `.desktop` file.
    Gio(gio::Icon),
}

pub fn widget(icon: Option<&Icon>, size: i32) -> gtk::Widget {
    match icon {
        Some(Icon::Gio(icon)) => image(|| gtk::Image::from_gicon(icon), size),
        Some(Icon::Spec(spec)) if is_emoji(spec) => {
            let label = gtk::Label::new(Some(spec));
            label.add_css_class("emoji-icon");
            label.set_size_request(size, size);
            label.upcast()
        }
        Some(Icon::Spec(spec)) if spec.starts_with('/') => image(|| gtk::Image::from_file(spec), size),
        Some(Icon::Spec(spec)) if has_icon(spec) => image(|| gtk::Image::from_icon_name(spec), size),
        // Many themes (Adwaita included) only ship symbolic variants nowadays.
        Some(Icon::Spec(spec)) if has_icon(&format!("{spec}-symbolic")) => {
            image(|| gtk::Image::from_icon_name(&format!("{spec}-symbolic")), size)
        }
        // Unknown names fall back to a neutral icon instead of a broken image.
        Some(Icon::Spec(_)) => image(|| gtk::Image::from_icon_name("application-x-executable"), size),
        None => image(|| gtk::Image::from_icon_name("application-x-executable"), size),
    }
}

/// An icon in a rounded tile. App icons (full color) sit on a neutral tile;
/// named icons use their symbolic variant, colored by the tile's tint.
pub fn tile(icon: Option<&Icon>, tint: Option<&str>, size: i32) -> gtk::Widget {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("tile");
    tile.set_size_request(size, size);
    tile.set_valign(gtk::Align::Center);
    tile.set_halign(gtk::Align::Center);
    let inner = match icon {
        Some(Icon::Gio(_)) => {
            tile.add_css_class("app");
            widget(icon, size - 4)
        }
        Some(Icon::Spec(spec)) if is_emoji(spec) || spec.starts_with('/') => {
            tile.add_css_class("app");
            widget(icon, size - 8)
        }
        Some(Icon::Spec(spec)) => {
            tile.add_css_class(&format!("tint-{}", tint.unwrap_or("gray")));
            let symbolic = if spec.ends_with("-symbolic") { spec.clone() } else { format!("{spec}-symbolic") };
            let name = if has_icon(&symbolic) { symbolic } else { spec.clone() };
            widget(Some(&Icon::Spec(name)), size * 10 / 17)
        }
        None => {
            tile.add_css_class(&format!("tint-{}", tint.unwrap_or("gray")));
            widget(Some(&Icon::Spec("application-x-executable-symbolic".into())), size * 10 / 17)
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

fn has_icon(name: &str) -> bool {
    gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name))
}

fn image(make: impl FnOnce() -> gtk::Image, size: i32) -> gtk::Widget {
    let image = make();
    image.set_pixel_size(size);
    image.upcast()
}

/// One or two code points outside ASCII with no path or dash: treat as emoji.
fn is_emoji(s: &str) -> bool {
    let n = s.chars().count();
    (1..=4).contains(&n) && s.chars().next().is_some_and(|c| !c.is_ascii() && !c.is_alphanumeric())
}

/// A colored pill or dimmed text on the right of a row.
pub fn accessory(a: &backslash_core::protocol::Accessory) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    b.add_css_class("accessory");
    if let Some(color) = &a.color {
        b.add_css_class(&format!("accent-{}", glib::markup_escape_text(color)));
    }
    if let Some(icon) = a
        .icon
        .as_ref()
        .filter(|i| is_emoji(i) || i.starts_with('/') || has_icon(i) || has_icon(&format!("{i}-symbolic")))
    {
        b.append(&widget(Some(&Icon::Spec(icon.clone())), 14));
    }
    if let Some(text) = &a.text {
        b.append(&gtk::Label::new(Some(text)));
    }
    if let Some(tag) = &a.tag {
        let l = gtk::Label::new(Some(tag));
        l.add_css_class("tag");
        b.append(&l);
    }
    b.upcast()
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
}
