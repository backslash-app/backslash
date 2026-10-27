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
