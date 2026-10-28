//! Widget builders for rows, the preview panel and metadata lines.

use backslash_core::protocol::{MetadataEntry, Stat};
use gtk::prelude::*;

use super::model::{Preview, Row, RowStyle};
use crate::icons::{self, Icon};
use crate::markdown;

pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l
}

/// Keyboard keys as small caps: `Actions [Ctrl] [K]`.
pub fn key_hint(text: &str, keys: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    b.add_css_class("hint");
    b.set_valign(gtk::Align::Center);
    if !text.is_empty() {
        let t = gtk::Label::new(Some(text));
        t.set_margin_end(2);
        b.append(&t);
    }
    for k in keys.split(' ').filter(|k| !k.is_empty()) {
        let key = label(k, "key");
        key.set_valign(gtk::Align::Center);
        key.set_xalign(0.5);
        b.append(&key);
    }
    b
}

/// Keys of an accelerator, one cap each, Ctrl first: `<Shift><Ctrl>c` → `Ctrl Shift C`.
pub fn accel_keys(accel: &str) -> String {
    let label = pretty_accel(accel);
    let mut parts: Vec<&str> = label.split('+').filter(|p| !p.is_empty()).collect();
    let rank = |p: &str| match p {
        "Ctrl" => 0,
        "Alt" => 1,
        "Shift" => 2,
        "Super" | "Meta" => 3,
        _ => 4,
    };
    parts.sort_by_key(|p| rank(p));
    parts.join(" ")
}

pub fn row_widget(row: &Row) -> gtk::ListBoxRow {
    let widget = gtk::ListBoxRow::new();
    widget.set_focusable(false);
    if row.style == RowStyle::Answer {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.add_css_class("answer");
        b.append(&label(&row.title, "result"));
        b.append(&label(row.subtitle.as_deref().unwrap_or_default(), "expression"));
        widget.set_child(Some(&b));
        return widget;
    }
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = match row.tint {
        Some(tint) => icons::tile(row.icon.as_ref(), Some(tint), 24),
        None => {
            let w = icons::plain(row.icon.as_ref(), row.icon_tint.as_deref(), 18);
            // Same column width as tiles, so titles line up.
            let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            slot.set_size_request(24, -1);
            slot.set_hexpand(false);
            slot.set_homogeneous(true);
            w.set_halign(gtk::Align::Center);
            slot.append(&w);
            slot.upcast()
        }
    };
    b.append(&icon);
    // The title gets the room; the subtitle and accessories keep theirs.
    let title = label(&row.title, "title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_hexpand(true);
    let text = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    text.set_hexpand(true);
    text.append(&title);
    if let Some(subtitle) = &row.subtitle {
        let s = label(subtitle, "row-subtitle");
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_max_width_chars(22);
        // Long subtitles give way before the title does.
        title.set_hexpand(false);
        s.set_hexpand(true);
        s.set_width_chars(3);
        text.append(&s);
    }
    b.append(&text);
    for a in row.accessories.iter().take(5) {
        b.append(&icons::accessory(a));
    }
    if let Some(alias) = &row.alias {
        let k = label(alias, "kbd");
        k.set_valign(gtk::Align::Center);
        k.set_tooltip_text(Some("Alias"));
        b.append(&k);
    }
    widget.set_child(Some(&b));
    widget
}

/// A grey placeholder shape, pulsing while something loads.
fn bone(width: i32, height: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("bone");
    b.set_size_request(width, height);
    b.set_hexpand(false);
    b.set_halign(gtk::Align::Start);
    b.set_valign(gtk::Align::Center);
    b
}

/// Rows shaped like results (icon, title, a short accessory), with varied
/// lengths so they read as a list about to appear rather than a pattern.
pub fn skeleton_rows() -> gtk::Box {
    const WIDTHS: [(i32, i32); 9] =
        [(210, 44), (160, 0), (240, 56), (130, 36), (190, 0), (225, 48), (150, 0), (200, 40), (170, 0)];
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("skeleton-rows");
    for (title, accessory) in WIDTHS {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("skeleton-row");
        let icon = bone(24, 24);
        icon.add_css_class("bone-icon");
        row.append(&icon);
        let text = bone(title, 10);
        text.set_hexpand(true);
        row.append(&text);
        if accessory > 0 {
            row.append(&bone(accessory, 10));
        }
        rows.append(&row);
    }
    rows
}

/// The preview panel's shape: a tile, a title, and a few lines of text.
pub fn skeleton_preview() -> gtk::Box {
    let preview = gtk::Box::new(gtk::Orientation::Vertical, 16);
    preview.add_css_class("preview");
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let tile = bone(40, 40);
    tile.add_css_class("bone-tile");
    head.append(&tile);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 8);
    names.set_valign(gtk::Align::Center);
    names.append(&bone(150, 12));
    names.append(&bone(90, 9));
    head.append(&names);
    preview.append(&head);
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for width in [230, 200, 215, 120] {
        lines.append(&bone(width, 9));
    }
    preview.append(&lines);
    preview
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Fills `container` with the preview of the selected row.
pub fn fill_preview(container: &gtk::Box, preview: &Preview) {
    clear(container);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = match (&preview.icon, icons::is_symbolic(preview.icon.as_ref())) {
        (Some(Icon::Spec(spec)), true) if preview.tint.is_none() && crate::glyphs::exists(spec) => {
            icons::tile(preview.icon.as_ref(), Some("gray"), 40)
        }
        _ => icons::tile(preview.icon.as_ref(), Some(preview.tint.as_deref().unwrap_or("accent")), 40),
    };
    head.append(&icon);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_valign(gtk::Align::Center);
    names.set_hexpand(true);
    let title = label(&preview.title, "preview-title");
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(3);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    names.append(&title);
    if let Some(subtitle) = &preview.subtitle {
        let s = label(subtitle, "subtitle");
        s.set_wrap(true);
        s.set_lines(2);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        names.append(&s);
    }
    head.append(&names);
    container.append(&head);

    if !preview.stats.is_empty() {
        container.append(&stats(&preview.stats));
    }
    if let Some(color) = &preview.swatch {
        container.append(&icons::big_swatch(color));
    }
    if let Some(md) = preview.markdown.as_deref().filter(|m| !m.trim().is_empty()) {
        let text = gtk::Label::new(None);
        markdown::set(&text, md);
        text.add_css_class("preview-text");
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_xalign(0.0);
        text.set_selectable(true);
        text.set_can_focus(false);
        container.append(&text);
    }
    if let Some(texture) = preview.image_url.as_deref().and_then(crate::images::texture) {
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_size_request(-1, 180);
        container.append(&picture);
    }
    if let Some(path) = &preview.picture {
        let picture = gtk::Picture::for_filename(path);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_vexpand(true);
        picture.set_size_request(-1, 160);
        container.append(&picture);
    }
    if let Some(body) = preview.body.as_ref().or(preview.text.as_ref()) {
        let text = label(body, "preview-body");
        if preview.body.is_none() {
            text.add_css_class("preview-prose");
        }
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_selectable(true);
        text.set_can_focus(false);
        text.set_yalign(0.0);
        let scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&text).vexpand(true).build();
        scroll.add_css_class("preview-body-box");
        container.append(&scroll);
    }

    let mut lines: Vec<MetadataEntry> = preview
        .facts
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| MetadataEntry { label: k.clone(), value: v.clone(), ..Default::default() })
        .collect();
    lines.extend(preview.metadata.iter().cloned());
    if !lines.is_empty() {
        container.append(&metadata_grid(&lines, 104));
    }
}

/// A row of small cards with a big number each.
pub fn stats(stats: &[Stat]) -> gtk::Widget {
    let grid = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    grid.set_homogeneous(true);
    for s in stats.iter().take(4) {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("stat");
        let value = label(&s.value, "stat-value");
        if let Some(c) = &s.color {
            value.add_css_class(&format!("accent-{}", icons::css_name(c)));
        }
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        card.append(&value);
        let l = label(&s.label, "stat-label");
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        card.append(&l);
        grid.append(&card);
    }
    grid.upcast()
}

/// Label/value lines: text with an icon, tags, a link, a person, progress.
pub fn metadata_grid(entries: &[MetadataEntry], label_width: i32) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(10);
    let mut row = 0;
    for m in entries {
        if m.separator {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            line.add_css_class("meta-separator");
            line.set_hexpand(true);
            grid.attach(&line, 0, row, 2, 1);
            row += 1;
            continue;
        }
        let name = label(&m.label, "meta-label");
        name.set_size_request(label_width, -1);
        name.set_valign(gtk::Align::Start);
        name.set_wrap(true);
        grid.attach(&name, 0, row, 1, 1);
        grid.attach(&metadata_value(m), 1, row, 1, 1);
        row += 1;
    }
    grid.upcast()
}

fn metadata_value(m: &MetadataEntry) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.set_hexpand(true);
    if let Some(color) = &m.color {
        b.add_css_class(&format!("accent-{}", icons::css_name(color)));
    }
    if let Some(icon) = &m.icon {
        b.append(&icons::plain(Some(&Icon::Spec(icon.clone())), None, 14));
    }
    if let Some(person) = &m.avatar {
        b.append(&icons::avatar(person, 18));
    }
    if let Some(progress) = m.progress {
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("meta-progress");
        bar.set_fraction(progress.clamp(0.0, 1.0));
        bar.set_valign(gtk::Align::Center);
        bar.set_hexpand(true);
        bar.set_size_request(60, -1);
        b.append(&bar);
    }
    if !m.value.is_empty() {
        let value = label(&m.value, "meta-value");
        value.set_wrap(true);
        value.set_max_width_chars(30);
        value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        value.set_hexpand(m.progress.is_none());
        if m.color.is_none() && m.link.is_none() {
            value.set_selectable(true);
            value.set_can_focus(false);
        }
        if let Some(link) = &m.link {
            value.add_css_class("link");
            value.set_markup(&format!(
                "<a href=\"{}\">{}</a>",
                gtk::glib::markup_escape_text(link),
                gtk::glib::markup_escape_text(&m.value)
            ));
            value.set_tooltip_text(Some(link));
        }
        b.append(&value);
    }
    if !m.tags.is_empty() {
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(4);
        flow.set_row_spacing(4);
        flow.set_max_children_per_line(8);
        flow.set_hexpand(true);
        // Pills hug their text: no stretching to the line's width or height.
        flow.set_halign(gtk::Align::Start);
        flow.set_valign(gtk::Align::Center);
        for t in &m.tags {
            let tag = label(&t.text, "tag");
            tag.set_halign(gtk::Align::Start);
            tag.set_valign(gtk::Align::Center);
            if let Some(c) = &t.color {
                tag.add_css_class(&format!("accent-{}", icons::css_name(c)));
            }
            flow.append(&tag);
            if let Some(child) = flow.last_child() {
                child.set_focusable(false);
                child.set_halign(gtk::Align::Start);
                child.set_valign(gtk::Align::Center);
            }
        }
        b.append(&flow);
    }
    b.upcast()
}

pub fn pretty_accel(accel: &str) -> String {
    gtk::accelerator_parse(accel)
        .map(|(k, m)| gtk::accelerator_get_label(k, m).to_string())
        .unwrap_or_else(|| accel.to_string())
}

/// A tile of the grid layout: a big icon (or emoji) and the title below.
pub fn grid_cell(row: &Row) -> gtk::Box {
    let cell = gtk::Box::new(gtk::Orientation::Vertical, 6);
    cell.add_css_class("grid-cell");
    let icon = icons::plain(row.icon.as_ref(), row.icon_tint.as_deref(), 44);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    icon.set_vexpand(true);
    cell.append(&icon);
    let title = label(&row.title, "grid-title");
    title.set_xalign(0.5);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(1);
    title.set_hexpand(true);
    cell.set_tooltip_text(Some(&row.title));
    cell.append(&title);
    cell
}
