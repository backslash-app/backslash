//! Widget builders for rows and the preview panel.

use gtk::prelude::*;

use super::model::{Preview, Row, RowStyle, UiAction};
use crate::icons;

pub fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l
}

/// `Actions [Ctrl] [K]`
pub fn key_hint(text: &str, keys: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    b.add_css_class("hint");
    if !text.is_empty() {
        b.append(&gtk::Label::new(Some(text)));
    }
    for k in keys.split(' ') {
        b.append(&label(k, "key"));
    }
    b
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
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.append(&icons::tile(row.icon.as_ref(), row.tint, 34));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let title = label(&row.title, "title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    if let Some(subtitle) = &row.subtitle {
        let s = label(subtitle, "subtitle");
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&s);
    }
    b.append(&text);
    // Keep rows compact: the preview panel shows every accessory.
    for a in row.accessories.iter().take(2) {
        b.append(&icons::accessory(a));
    }
    widget.set_child(Some(&b));
    widget
}

/// Fills `container` with the preview of the selected row.
pub fn fill_preview(container: &gtk::Box, preview: &Preview, actions: &[UiAction]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    head.append(&icons::tile(preview.icon.as_ref(), Some("blue"), 52));
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

    let facts: Vec<&(String, String)> = preview.facts.iter().filter(|(_, v)| !v.is_empty()).collect();
    if !facts.is_empty() {
        let grid = gtk::Grid::new();
        grid.add_css_class("facts");
        grid.set_column_spacing(16);
        grid.set_row_spacing(8);
        for (i, (k, v)) in facts.iter().enumerate() {
            grid.attach(&label(k, "fact-label"), 0, i as i32, 1, 1);
            let value = label(v, "fact-value");
            value.set_wrap(true);
            value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            value.set_selectable(true);
            value.set_hexpand(true);
            grid.attach(&value, 1, i as i32, 1, 1);
        }
        container.append(&grid);
    }

    if let Some(body) = &preview.body {
        let text = label(body, "preview-body");
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.set_selectable(true);
        text.set_yalign(0.0);
        let scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&text).vexpand(true).build();
        scroll.add_css_class("preview-body-box");
        container.append(&scroll);
    }

    if !actions.is_empty() {
        container.append(&label("ACTIONS", "preview-section"));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (i, a) in actions.iter().take(5).enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let t = label(&a.title, "preview-action");
            t.set_hexpand(true);
            if a.destructive {
                t.add_css_class("destructive");
            }
            row.append(&t);
            let keys = match (i, &a.shortcut) {
                (0, _) => Some("↵".to_string()),
                (1, None) => Some("Ctrl ↵".to_string()),
                (_, Some(s)) => Some(pretty_accel(s)),
                _ => None,
            };
            if let Some(k) = keys {
                row.append(&label(&k, "preview-key"));
            }
            list.append(&row);
        }
        container.append(&list);
    }
}

pub fn pretty_accel(accel: &str) -> String {
    gtk::accelerator_parse(accel)
        .map(|(k, m)| gtk::accelerator_get_label(k, m).to_string())
        .unwrap_or_else(|| accel.to_string())
}
