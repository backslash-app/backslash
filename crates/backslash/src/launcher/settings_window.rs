//! The settings window: appearance, shortcuts, extensions and their
//! preferences, clipboard, quicklinks, snippets. A real window (not a view
//! of the launcher) so it can stay open while you try things.

use std::cell::Cell;
use std::rc::Rc;

use backslash_core::catalog::{self, Locations};
use backslash_core::config::{Accent, Density, Quicklink, Reopen, Snippet, Theme, WindowSize};
use backslash_core::manifest::PreferenceKind;
use backslash_core::{paths, runtime, secrets};
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Map, Value};

use super::model::{tint_for, Op, UiAction};
use super::widgets::label;
use super::Launcher;
use crate::glyphs::Glyph;
use crate::icons::{self, Icon};
use crate::{autostart, paste};

const PAGES: [(&str, &str, &str); 6] = [
    ("general", "General", "bs-settings"),
    ("extensions", "Extensions", "bs-package"),
    ("clipboard", "Clipboard", "bs-paste"),
    ("quicklinks", "Quicklinks", "bs-link"),
    ("snippets", "Snippets", "bs-text"),
    ("advanced", "Advanced", "bs-code"),
];

fn heading(text: &str) -> gtk::Label {
    label(text, "settings-heading")
}

fn card(rows: &[gtk::Widget]) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    for r in rows {
        card.append(r);
    }
    card
}

/// Title (and subtitle) on the left, a control on the right.
fn card_row(title: &str, subtitle: Option<&str>, control: Option<&gtk::Widget>) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("card-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = label(title, "card-title");
    t.set_wrap(true);
    text.append(&t);
    if let Some(sub) = subtitle.filter(|s| !s.is_empty()) {
        let s = label(sub, "card-subtitle");
        s.set_wrap(true);
        text.append(&s);
    }
    row.append(&text);
    if let Some(c) = control {
        c.set_valign(gtk::Align::Center);
        row.append(c);
    }
    row.upcast()
}

fn button(text: &str, class: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    b
}

fn switch(active: bool, on_change: impl Fn(bool) + 'static) -> gtk::Widget {
    let s = gtk::Switch::new();
    s.set_active(active);
    s.set_valign(gtk::Align::Center);
    s.connect_active_notify(move |s| on_change(s.is_active()));
    s.upcast()
}

/// Buttons side by side, one active.
/// A tile per theme, painted in its colors.
fn theme_picker(current: Theme, on_pick: impl Fn(Theme) + 'static) -> gtk::Widget {
    let flow = gtk::FlowBox::new();
    flow.add_css_class("theme-picker");
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_max_children_per_line(5);
    flow.set_min_children_per_line(5);
    flow.set_homogeneous(true);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    let on_pick = Rc::new(on_pick);
    let tiles: Rc<std::cell::RefCell<Vec<gtk::Button>>> = Rc::default();
    for theme in Theme::ALL {
        let (bg, fg, accent) = crate::theme::swatch(theme);
        let background = if theme == Theme::System {
            "linear-gradient(135deg, #ffffff 50%, #141417 50%)".to_string()
        } else {
            bg.to_string()
        };
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("theme-accent");
        bar.set_halign(gtk::Align::Start);
        content.append(&bar);
        let name = gtk::Label::new(Some(theme.name()));
        name.set_xalign(0.0);
        content.append(&name);
        let tile = gtk::Button::builder().child(&content).css_classes(["theme-tile"]).build();
        tile.update_property(&[gtk::accessible::Property::Label(theme.name())]);
        let provider = gtk::CssProvider::new();
        let label = if theme == Theme::System { "#17171c" } else { fg };
        provider.load_from_string(&format!(
            "button {{ background: {background}; color: {label}; }} box.theme-accent {{ background: {accent}; }}"
        ));
        #[allow(deprecated)]
        {
            tile.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            bar.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        }
        if theme == current {
            tile.add_css_class("active");
        }
        let (on_pick, all) = (on_pick.clone(), tiles.clone());
        tile.connect_clicked(move |me| {
            for t in all.borrow().iter() {
                t.remove_css_class("active");
            }
            me.add_css_class("active");
            on_pick(theme);
        });
        tiles.borrow_mut().push(tile.clone());
        flow.insert(&tile, -1);
    }
    flow.upcast()
}

fn segmented(options: &[&str], active: usize, on_pick: impl Fn(usize) + 'static) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("segmented");
    let on_pick = Rc::new(on_pick);
    let buttons: Rc<std::cell::RefCell<Vec<gtk::Button>>> = Rc::default();
    for (i, text) in options.iter().enumerate() {
        let button = gtk::Button::with_label(text);
        if i == active {
            button.add_css_class("active");
        }
        let (on_pick, all) = (on_pick.clone(), buttons.clone());
        button.connect_clicked(move |_| {
            for (j, other) in all.borrow().iter().enumerate() {
                if j == i {
                    other.add_css_class("active");
                } else {
                    other.remove_css_class("active");
                }
            }
            on_pick(i);
        });
        buttons.borrow_mut().push(button.clone());
        b.append(&button);
    }
    b.upcast()
}

/// `Super+Space` style, from a key press. `None` for a lone modifier.
fn accel_from(key: gdk::Key, mods: gdk::ModifierType) -> Option<String> {
    let name = key.name()?.to_string();
    if name.starts_with("Control")
        || name.starts_with("Shift")
        || name.starts_with("Alt")
        || name.starts_with("Super")
        || name.starts_with("Meta")
        || name.starts_with("ISO_")
        || name.starts_with("Hyper")
    {
        return None;
    }
    let mut parts = Vec::new();
    if mods.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl".to_string());
    }
    if mods.contains(gdk::ModifierType::ALT_MASK) {
        parts.push("Alt".into());
    }
    if mods.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift".into());
    }
    if mods.contains(gdk::ModifierType::SUPER_MASK) || mods.contains(gdk::ModifierType::META_MASK) {
        parts.push("Super".into());
    }
    let key = if name.chars().count() == 1 {
        name.to_uppercase()
    } else {
        let mut c = name.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let function_key = key.starts_with('F') && key[1..].parse::<u8>().is_ok();
    if parts.is_empty() && !function_key {
        return None;
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// A button showing a shortcut; click it, then press the keys to record.
/// Backspace clears it, Escape keeps the old one.
fn shortcut_capture(current: &str, on_set: impl Fn(String) + 'static) -> gtk::Widget {
    let button = gtk::Button::with_label(if current.is_empty() { "Record" } else { current });
    button.add_css_class("flat-button");
    button.add_css_class("shortcut-capture");
    button.set_valign(gtk::Align::Center);
    let listening = Rc::new(Cell::new(false));
    let shown = Rc::new(std::cell::RefCell::new(current.to_string()));
    {
        let listening = listening.clone();
        button.connect_clicked(move |b| {
            listening.set(true);
            b.add_css_class("listening");
            b.set_label("Press keys…");
            b.grab_focus();
        });
    }
    let keys = gtk::EventControllerKey::new();
    let on_set = Rc::new(on_set);
    let weak = button.downgrade();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(b) = weak.upgrade() else { return glib::Propagation::Proceed };
        if !listening.get() {
            return glib::Propagation::Proceed;
        }
        let finish = |text: &str| {
            listening.set(false);
            b.remove_css_class("listening");
            b.set_label(if text.is_empty() { "Record" } else { text });
        };
        match key {
            gdk::Key::Escape => finish(&shown.borrow().clone()),
            gdk::Key::BackSpace | gdk::Key::Delete => {
                shown.replace(String::new());
                finish("");
                on_set(String::new());
            }
            _ => match accel_from(key, mods) {
                Some(accel) => {
                    shown.replace(accel.clone());
                    finish(&accel);
                    on_set(accel);
                }
                None => return glib::Propagation::Stop,
            },
        }
        glib::Propagation::Stop
    });
    button.add_controller(keys);
    button.upcast()
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(child)
        .vexpand(true)
        .hexpand(true)
        .build()
}

fn page_box() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("settings-page");
    b.set_valign(gtk::Align::Start);
    b
}

impl Launcher {
    /// Opens (or brings back) the settings window on `page`.
    pub(super) fn open_settings(self: &Rc<Self>, page: Option<&str>) {
        if self.window.is_visible() {
            self.hide();
        }
        if let Some(window) = self.settings_window.borrow().clone() {
            if let Some(page) = page {
                self.show_settings_page(page);
            }
            window.present();
            return;
        }
        let window = gtk::Window::builder().title("Backslash Settings").default_width(1000).default_height(660).build();
        if let Some(app) = self.window.application() {
            window.set_application(Some(&app));
        }
        window.add_css_class("bs-settings");
        let header = gtk::HeaderBar::new();
        header.set_title_widget(Some(&label("Settings", "card-title")));
        // Our own close button: tiling compositors (Hyprland, Sway, i3…) and
        // some decoration layouts show no window buttons at all.
        header.set_show_title_buttons(false);
        let close = gtk::Button::builder()
            .child(&Glyph::new("bs-x", 14))
            .tooltip_text("Close (Esc)")
            .css_classes(["settings-close"])
            .valign(gtk::Align::Center)
            .build();
        {
            let window = window.downgrade();
            close.connect_clicked(move |_| {
                if let Some(w) = window.upgrade() {
                    w.close();
                }
            });
        }
        header.pack_end(&close);
        window.set_titlebar(Some(&header));
        // Escape closes it too, unless a field (a shortcut being recorded) used it.
        let keys = gtk::EventControllerKey::new();
        {
            let window = window.downgrade();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                if let Some(w) = window.upgrade() {
                    w.close();
                }
                glib::Propagation::Stop
            });
        }
        window.add_controller(keys);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("settings-sidebar");
        sidebar.set_selection_mode(gtk::SelectionMode::Browse);
        sidebar.set_size_request(210, -1);
        for (_, title, icon) in PAGES {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.append(&Glyph::new(icon, 15));
            b.append(&label(title, "title"));
            sidebar.append(&b);
        }
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(80);
        stack.set_hexpand(true);
        body.append(&sidebar);
        body.append(&stack);
        window.set_child(Some(&body));
        *self.settings_window.borrow_mut() = Some(window.clone());
        *self.settings_stack.borrow_mut() = Some(stack.clone());
        self.apply_style();

        let weak = Rc::downgrade(self);
        sidebar.connect_row_selected(move |_, row| {
            let (Some(l), Some(row)) = (weak.upgrade(), row) else { return };
            let page = PAGES[row.index() as usize].0;
            l.show_settings_page(page);
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(l) = weak.upgrade() {
                l.settings_window.replace(None);
                l.settings_stack.replace(None);
            }
            glib::Propagation::Proceed
        });
        let index = page.and_then(|p| PAGES.iter().position(|(id, _, _)| *id == p)).unwrap_or(0);
        if let Some(row) = sidebar.row_at_index(index as i32) {
            sidebar.select_row(Some(&row));
        }
        window.present();
    }

    /// Builds the page anew (it reflects the current config) and shows it.
    fn show_settings_page(self: &Rc<Self>, page: &str) {
        let Some(stack) = self.settings_stack.borrow().clone() else { return };
        if let Some(old) = stack.child_by_name(page) {
            stack.remove(&old);
        }
        let content = match page {
            "extensions" => self.extensions_page(None),
            "clipboard" => self.clipboard_page(),
            "quicklinks" => self.quicklinks_page(None),
            "snippets" => self.snippets_page(None),
            "advanced" => self.advanced_page(),
            _ => self.general_page(),
        };
        stack.add_named(&content, Some(page));
        stack.set_visible_child_name(page);
    }

    fn general_page(self: &Rc<Self>) -> gtk::Widget {
        let page = page_box();
        let config = self.state.borrow().config.clone();

        page.append(&heading("Launcher"));
        let weak = Rc::downgrade(self);
        let shortcut = shortcut_capture(&config.hotkey, move |accel| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.hotkey = accel);
                l.bind_shortcuts();
            }
        });
        let status = match self.shortcuts.status() {
            crate::hotkey::Status::Active => "Registered with your desktop",
            crate::hotkey::Status::Pending => "Waiting for your desktop to confirm",
            crate::hotkey::Status::Unavailable => {
                "Your desktop has no shortcut portal: bind `backslash` in its keyboard settings"
            }
            crate::hotkey::Status::Disabled => "Click, then press the keys",
        };
        let weak = Rc::downgrade(self);
        let login = switch(self.state.borrow().autostart, move |on| {
            let Some(l) = weak.upgrade() else { return };
            match autostart::set(on) {
                Ok(()) => l.state.borrow_mut().autostart = on,
                Err(e) => l.fail(e),
            }
        });
        let paste_status = match self.state.borrow().paste {
            Some(paste::Method::Portal) => "Through your desktop's remote control portal (asks once)".to_string(),
            Some(m) => format!("With {}", m.name()),
            None => "Not available here: items are copied instead".to_string(),
        };
        let weak = Rc::downgrade(self);
        let reopen = segmented(
            &["Where I was", "For a minute", "Search"],
            match config.reopen {
                Reopen::Restore => 0,
                Reopen::Minute => 1,
                Reopen::Root => 2,
            },
            move |i| {
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| c.reopen = [Reopen::Restore, Reopen::Minute, Reopen::Root][i]);
                }
            },
        );
        let weak = Rc::downgrade(self);
        let tray = switch(config.tray, move |on| {
            let Some(l) = weak.upgrade() else { return };
            l.update_config(|c| c.tray = on);
            if on {
                l.start_tray();
            } else {
                crate::tray::stop();
            }
        });
        let weak = Rc::downgrade(self);
        let updates = switch(config.check_updates, move |on| {
            let Some(l) = weak.upgrade() else { return };
            l.update_config(|c| c.check_updates = on);
            if on {
                l.check_app_update(false);
            }
        });
        page.append(&card(&[
            card_row("Global shortcut", Some(status), Some(&shortcut)),
            card_row("When reopened", Some("After an action, Escape or a click elsewhere"), Some(&reopen)),
            card_row("Launch at login", Some("Starts hidden, so the first press is instant"), Some(&login)),
            card_row("Icon in the system tray", Some("Shows that Backslash is running, with a menu"), Some(&tray)),
            card_row(
                "Check for updates",
                Some(&format!("You have Backslash {}", backslash_core::selfupdate::CURRENT)),
                Some(&updates),
            ),
            card_row("Paste into apps", Some(&paste_status), None),
        ]));

        page.append(&heading("Appearance"));
        let weak = Rc::downgrade(self);
        let theme = theme_picker(config.theme, move |theme| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.theme = theme);
                l.apply_style();
            }
        });
        let accents = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dots: Rc<std::cell::RefCell<Vec<gtk::Button>>> = Rc::default();
        for accent in Accent::ALL {
            let dot = gtk::Button::new();
            dot.add_css_class("accent-dot");
            dot.set_tooltip_text(Some(accent.name()));
            dot.update_property(&[gtk::accessible::Property::Label(accent.name())]);
            let color = match accent {
                // Two tones: whatever the theme brings.
                Accent::Theme => "linear-gradient(135deg, #8b93ff 0%, #f778ba 50%, #3fb950 100%)",
                Accent::Indigo => "#7178f0",
                Accent::Blue => "#3b82f6",
                Accent::Green => "#22a06b",
                Accent::Orange => "#e8833a",
                Accent::Pink => "#e5487b",
                Accent::Graphite => "#8e8e98",
            };
            let provider = gtk::CssProvider::new();
            provider.load_from_string(&format!("button {{ background: {color}; }}"));
            #[allow(deprecated)]
            dot.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
            if accent == config.accent {
                dot.add_css_class("active");
            }
            let (weak, all) = (Rc::downgrade(self), dots.clone());
            dot.connect_clicked(move |me| {
                for d in all.borrow().iter() {
                    d.remove_css_class("active");
                }
                me.add_css_class("active");
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| c.accent = accent);
                    l.apply_style();
                }
            });
            dots.borrow_mut().push(dot.clone());
            accents.append(&dot);
        }
        let weak = Rc::downgrade(self);
        let density =
            segmented(&["Comfortable", "Compact"], usize::from(config.density == Density::Compact), move |i| {
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| c.density = if i == 1 { Density::Compact } else { Density::Comfortable });
                    l.apply_style();
                }
            });
        let weak = Rc::downgrade(self);
        let sizes: Vec<&str> = WindowSize::ALL.iter().map(|s| s.name()).collect();
        let current = WindowSize::ALL.iter().position(|s| *s == config.window_size).unwrap_or(1);
        let size = segmented(&sizes, current, move |i| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.window_size = WindowSize::ALL[i]);
                l.apply_style();
            }
        });
        page.append(&card(&[card_row("Theme", None, None), theme]));
        page.append(&card(&[
            card_row(
                "Accent color",
                Some("Selection, keys and links; the first one is the theme's"),
                Some(&accents.upcast()),
            ),
            card_row("Density", Some("Compact fits a third more rows"), Some(&density)),
            card_row("Window size", None, Some(&size)),
        ]));
        scrolled(&page).upcast()
    }

    fn extensions_page(self: &Rc<Self>, selected: Option<String>) -> gtk::Widget {
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let list = gtk::ListBox::new();
        list.add_css_class("settings-sidebar");
        list.set_selection_mode(gtk::SelectionMode::Browse);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let detail_scroll = scrolled(&detail);
        let (ids, updates) = {
            let s = self.state.borrow();
            let mut exts: Vec<_> = s.registry.extensions.iter().map(|e| e.manifest.clone()).collect();
            exts.sort_by_key(|m| m.extension.title.to_lowercase());
            for m in &exts {
                let id = &m.extension.id;
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let icon = Icon::Spec(m.extension.icon.clone().unwrap_or_else(|| "bs-package".into()));
                row.append(&icons::tile(Some(&icon), Some(tint_for(id)), 22));
                let name = label(&m.extension.title, "title");
                name.set_hexpand(true);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                if !s.config.is_enabled(id) {
                    name.add_css_class("subtitle");
                }
                row.append(&name);
                if s.catalog.updates.contains(id) {
                    let tag = label("Update", "tag");
                    tag.add_css_class("accent-orange");
                    row.append(&tag);
                }
                list.append(&row);
            }
            (exts.iter().map(|m| m.extension.id.clone()).collect::<Vec<_>>(), s.catalog.updates.clone())
        };
        let list_scroll = scrolled(&list);
        list_scroll.set_hexpand(false);
        list_scroll.set_size_request(250, -1);
        split.append(&list_scroll);
        split.append(&detail_scroll);
        let ids = Rc::new(ids);
        let weak = Rc::downgrade(self);
        let (ids_c, detail_c) = (ids.clone(), detail.clone());
        list.connect_row_selected(move |_, row| {
            let (Some(l), Some(row)) = (weak.upgrade(), row) else { return };
            while let Some(c) = detail_c.first_child() {
                detail_c.remove(&c);
            }
            if let Some(id) = ids_c.get(row.index() as usize) {
                detail_c.append(&l.extension_detail(id, updates.contains(id)));
            }
        });
        let index = selected.and_then(|s| ids.iter().position(|i| *i == s)).unwrap_or(0);
        if let Some(row) = list.row_at_index(index as i32) {
            list.select_row(Some(&row));
        }
        if ids.is_empty() {
            let page = page_box();
            page.append(&card(&[card_row(
                "No extensions yet",
                Some("Browse the store from the launcher: “Browse Extensions”."),
                None,
            )]));
            return page.upcast();
        }
        split.upcast()
    }

    fn extension_detail(self: &Rc<Self>, id: &str, update: bool) -> gtk::Widget {
        let page = page_box();
        let s = self.state.borrow();
        let Some(ext) = s.registry.find(id) else { return page.upcast() };
        let m = ext.manifest.clone();
        let dir = ext.dir.clone();
        let enabled = s.config.is_enabled(id);
        let aliases = s.config.aliases.clone();
        let hotkeys = s.config.hotkeys.clone();
        let plain = s.config.extensions.get(id).map(|e| e.preferences.clone()).unwrap_or_default();
        let installed = catalog::installed(&Locations::user(&s.config), id).is_some();
        drop(s);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let icon = Icon::Spec(m.extension.icon.clone().unwrap_or_else(|| "bs-package".into()));
        head.append(&icons::tile(Some(&icon), Some(tint_for(id)), 44));
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_hexpand(true);
        names.set_valign(gtk::Align::Center);
        names.append(&label(&m.extension.title, "preview-title"));
        let by = if m.extension.author.is_empty() { String::new() } else { format!(" · {}", m.extension.author) };
        names.append(&label(&format!("Version {}{by}", m.extension.version), "card-subtitle"));
        head.append(&names);
        let weak = Rc::downgrade(self);
        let ext_id = id.to_string();
        head.append(&switch(enabled, move |on| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.extensions.entry(ext_id.clone()).or_default().enabled = on);
                l.reload();
            }
        }));
        page.append(&head);
        if !m.extension.description.is_empty() {
            let d = label(&m.extension.description, "subtitle");
            d.set_wrap(true);
            d.set_margin_top(12);
            page.append(&d);
        }

        page.append(&heading("Commands"));
        let mut rows = Vec::new();
        for c in &m.commands {
            let key = format!("cmd:{}/{}", id, c.name);
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let alias = gtk::Entry::new();
            alias.set_placeholder_text(Some("Alias"));
            alias.set_width_chars(6);
            alias.set_max_width_chars(8);
            alias.set_text(aliases.get(&key).map_or("", String::as_str));
            let (weak, k) = (Rc::downgrade(self), key.clone());
            let save_alias = move |e: &gtk::Entry| {
                let Some(l) = weak.upgrade() else { return };
                let text = e.text().trim().to_string();
                let unchanged = l.state.borrow().config.aliases.get(&k).map_or(text.is_empty(), |a| *a == text);
                if !unchanged {
                    l.update_config(|c| {
                        if text.is_empty() {
                            c.aliases.remove(&k);
                        } else {
                            c.aliases.insert(k.clone(), text.clone());
                        }
                    });
                }
            };
            let save = save_alias.clone();
            alias.connect_activate(move |e| save(e));
            let focus = gtk::EventControllerFocus::new();
            let entry = alias.clone();
            focus.connect_leave(move |_| save_alias(&entry));
            alias.add_controller(focus);
            controls.append(&alias);
            let (weak, k) = (Rc::downgrade(self), key.clone());
            controls.append(&shortcut_capture(hotkeys.get(&key).map_or("", String::as_str), move |accel| {
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| {
                        if accel.is_empty() {
                            c.hotkeys.remove(&k);
                        } else {
                            c.hotkeys.insert(k.clone(), accel.clone());
                        }
                    });
                    l.bind_shortcuts();
                }
            }));
            rows.push(card_row(&c.title, Some(&c.description), Some(&controls.upcast())));
        }
        page.append(&card(&rows));

        if !m.preferences.is_empty() {
            page.append(&heading("Preferences"));
            let mut rows = Vec::new();
            let mut inputs: Vec<(String, gtk::Widget)> = Vec::new();
            for p in &m.preferences {
                let current = plain.get(&p.name).or(p.default.as_ref());
                let text = current.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).unwrap_or_default();
                let input: gtk::Widget = match p.kind {
                    PreferenceKind::Secret => {
                        let e = gtk::PasswordEntry::new();
                        e.set_show_peek_icon(true);
                        let saved = secrets::get(id, &p.name).is_some();
                        e.set_placeholder_text(Some(if saved { "Saved in the keyring" } else { "Not set" }));
                        e.set_width_chars(24);
                        e.upcast()
                    }
                    PreferenceKind::Boolean => {
                        let s = gtk::Switch::new();
                        s.set_active(current.and_then(|v| v.as_bool()).unwrap_or(false));
                        s.upcast()
                    }
                    PreferenceKind::Text | PreferenceKind::Number => {
                        let e = gtk::Entry::new();
                        e.set_text(&text);
                        e.set_width_chars(24);
                        e.upcast()
                    }
                };
                let title = if p.required { format!("{} *", p.title) } else { p.title.clone() };
                rows.push(card_row(&title, Some(&p.description), Some(&input)));
                inputs.push((p.name.clone(), input));
            }
            page.append(&card(&rows));
            let save = button("Save Preferences", "suggested");
            save.set_halign(gtk::Align::End);
            save.set_margin_top(10);
            let (weak, ext_id) = (Rc::downgrade(self), id.to_string());
            save.connect_clicked(move |_| {
                let Some(l) = weak.upgrade() else { return };
                let mut values = Map::new();
                for (name, w) in &inputs {
                    let v = if let Some(e) = w.downcast_ref::<gtk::PasswordEntry>() {
                        Value::String(e.text().to_string())
                    } else if let Some(s) = w.downcast_ref::<gtk::Switch>() {
                        Value::Bool(s.is_active())
                    } else if let Some(e) = w.downcast_ref::<gtk::Entry>() {
                        Value::String(e.text().to_string())
                    } else {
                        Value::Null
                    };
                    values.insert(name.clone(), v);
                }
                match l.save_preferences(&ext_id, &values) {
                    Ok(()) => {
                        l.show_settings_toast("Preferences saved");
                        l.show_extension_settings(&ext_id);
                    }
                    Err(e) => l.show_settings_toast(&format!("{e:#}")),
                }
            });
            page.append(&save);
        }

        page.append(&heading("Permissions"));
        let perms = backslash_core::approvals::requested(&m);
        let mut rows: Vec<gtk::Widget> = perms.iter().map(|p| card_row(p, None, None)).collect();
        if rows.is_empty() {
            rows.push(card_row("None", Some("It runs fully sandboxed"), None));
        }
        page.append(&card(&rows));

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_margin_top(18);
        let folder = button("Open Folder", "flat-button");
        let path = dir.display().to_string();
        let weak = Rc::downgrade(self);
        folder.connect_clicked(move |_| {
            if let Some(l) = weak.upgrade() {
                l.execute(UiAction::new("", Op::Open(path.clone())));
            }
        });
        buttons.append(&folder);
        if update {
            let b = button("Update…", "suggested");
            let (weak, ext_id) = (Rc::downgrade(self), id.to_string());
            b.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    // The launcher shows what the update is allowed to do first.
                    l.settings_window.borrow().as_ref().inspect(|w| w.minimize());
                    l.show();
                    l.execute(UiAction::new("", Op::ReviewInstall(ext_id.clone())));
                }
            });
            buttons.append(&b);
        }
        if installed {
            let b = button("Uninstall", "flat-button");
            b.add_css_class("danger");
            let (weak, ext_id) = (Rc::downgrade(self), id.to_string());
            b.connect_clicked(move |_| {
                let Some(l) = weak.upgrade() else { return };
                let loc = Locations::user(&l.state.borrow().config);
                match catalog::uninstall(&loc, &ext_id) {
                    Ok(()) => {
                        l.reload();
                        l.recount_updates();
                        l.show_settings_page("extensions");
                    }
                    Err(e) => l.show_settings_toast(&format!("{e:#}")),
                }
            });
            buttons.append(&b);
        }
        page.append(&buttons);
        page.upcast()
    }

    fn show_extension_settings(self: &Rc<Self>, id: &str) {
        let Some(stack) = self.settings_stack.borrow().clone() else { return };
        if let Some(old) = stack.child_by_name("extensions") {
            stack.remove(&old);
        }
        let content = self.extensions_page(Some(id.to_string()));
        stack.add_named(&content, Some("extensions"));
        stack.set_visible_child_name("extensions");
    }

    /// A short message in the settings window's header.
    fn show_settings_toast(&self, text: &str) {
        let Some(window) = self.settings_window.borrow().clone() else { return };
        let Some(header) = window.titlebar().and_downcast::<gtk::HeaderBar>() else { return };
        let title = label(text, "card-subtitle");
        header.set_title_widget(Some(&title));
        let header = header.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(3000), move || {
            if let Some(h) = header.upgrade() {
                h.set_title_widget(Some(&label("Settings", "card-title")));
            }
        });
    }

    fn clipboard_page(self: &Rc<Self>) -> gtk::Widget {
        let page = page_box();
        let config = self.state.borrow().config.clone();
        page.append(&heading("Clipboard history"));
        let weak = Rc::downgrade(self);
        let record = switch(config.clipboard.enabled, move |on| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.clipboard.enabled = on);
            }
        });
        let keep = gtk::SpinButton::with_range(20.0, 5000.0, 10.0);
        keep.set_value(config.clipboard.max_items as f64);
        let weak = Rc::downgrade(self);
        keep.connect_value_changed(move |s| {
            if let Some(l) = weak.upgrade() {
                l.update_config(|c| c.clipboard.max_items = s.value() as usize);
            }
        });
        let count = self.state.borrow().clipboard.entries().len();
        let clear = button("Clear History", "flat-button");
        clear.add_css_class("danger");
        let weak = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(l) = weak.upgrade() {
                l.execute(UiAction::new("", Op::ClipClear));
                l.show_settings_page("clipboard");
            }
        });
        page.append(&card(&[
            card_row(
                "Record what you copy",
                Some("Kept on this computer only; password managers are skipped"),
                Some(&record),
            ),
            card_row("Entries kept", Some("Pinned entries are always kept"), Some(&keep.upcast())),
            card_row(&format!("{count} entries"), Some("Pinned entries stay"), Some(&clear.upcast())),
        ]));
        scrolled(&page).upcast()
    }

    fn quicklinks_page(self: &Rc<Self>, editing: Option<usize>) -> gtk::Widget {
        let page = page_box();
        let links = self.state.borrow().config.quicklinks.clone();
        page.append(&heading(if editing.is_some_and(|i| i < links.len()) {
            "Edit quicklink"
        } else {
            "New quicklink"
        }));
        let current = editing.and_then(|i| links.get(i).cloned());
        let name = entry("Search GitHub", current.as_ref().map_or("", |l| &l.name));
        let url = entry("https://github.com/search?q={query}", current.as_ref().map_or("", |l| &l.url));
        let keyword = entry("gh", current.as_ref().and_then(|l| l.keyword.as_deref()).unwrap_or_default());
        url.set_width_chars(34);
        let save = button(if current.is_some() { "Save" } else { "Add" }, "suggested");
        let weak = Rc::downgrade(self);
        let (n, u, k) = (name.clone(), url.clone(), keyword.clone());
        save.connect_clicked(move |_| {
            let Some(l) = weak.upgrade() else { return };
            let (name, url) = (n.text().trim().to_string(), u.text().trim().to_string());
            if name.is_empty() || url.is_empty() {
                return l.show_settings_toast("A quicklink needs a name and a URL");
            }
            if !url.contains("://") {
                return l.show_settings_toast("The URL must start with https:// (or another scheme)");
            }
            let kw = k.text().trim().to_string();
            let link = Quicklink { name, url, keyword: Some(kw).filter(|k| !k.is_empty()), icon: None };
            l.update_config(|c| match editing {
                Some(i) if i < c.quicklinks.len() => c.quicklinks[i] = link,
                _ => c.quicklinks.push(link),
            });
            l.replace_settings_page("quicklinks", l.quicklinks_page(None));
        });
        page.append(&card(&[
            card_row("Name", None, Some(&name.upcast())),
            card_row("URL", Some("{query} is replaced by what you type after the keyword"), Some(&url.upcast())),
            card_row("Keyword", Some("Type “gh tokio” to search GitHub for tokio"), Some(&keyword.upcast())),
        ]));
        save.set_halign(gtk::Align::End);
        save.set_margin_top(10);
        page.append(&save);

        page.append(&heading("Quicklinks"));
        let mut rows = Vec::new();
        for (i, link) in links.iter().enumerate() {
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            if let Some(kw) = &link.keyword {
                let k = label(kw, "kbd");
                k.set_valign(gtk::Align::Center);
                controls.append(&k);
            }
            let edit = button("Edit", "flat-button");
            let weak = Rc::downgrade(self);
            edit.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.replace_settings_page("quicklinks", l.quicklinks_page(Some(i)));
                }
            });
            let delete = button("Delete", "flat-button");
            delete.add_css_class("danger");
            let weak = Rc::downgrade(self);
            delete.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| {
                        if i < c.quicklinks.len() {
                            c.quicklinks.remove(i);
                        }
                    });
                    l.replace_settings_page("quicklinks", l.quicklinks_page(None));
                }
            });
            controls.append(&edit);
            controls.append(&delete);
            rows.push(card_row(&link.name, Some(&link.url), Some(&controls.upcast())));
        }
        if rows.is_empty() {
            rows.push(card_row("No quicklinks yet", Some("Add one above"), None));
        }
        page.append(&card(&rows));
        scrolled(&page).upcast()
    }

    fn snippets_page(self: &Rc<Self>, editing: Option<usize>) -> gtk::Widget {
        let page = page_box();
        let snippets = self.state.borrow().config.snippets.clone();
        let current = editing.and_then(|i| snippets.get(i).cloned());
        page.append(&heading(if current.is_some() { "Edit snippet" } else { "New snippet" }));
        let name = entry("Email signature", current.as_ref().map_or("", |s| &s.name));
        let keyword = entry("sig", current.as_ref().and_then(|s| s.keyword.as_deref()).unwrap_or_default());
        let text = gtk::TextView::new();
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        text.buffer().set_text(current.as_ref().map_or("", |s| &s.text));
        text.set_size_request(320, 90);
        text.add_css_class("card");
        text.set_left_margin(8);
        text.set_top_margin(6);
        let save = button(if current.is_some() { "Save" } else { "Add" }, "suggested");
        let weak = Rc::downgrade(self);
        let (n, k, t) = (name.clone(), keyword.clone(), text.clone());
        save.connect_clicked(move |_| {
            let Some(l) = weak.upgrade() else { return };
            let buffer = t.buffer();
            let body = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
            let name = n.text().trim().to_string();
            if name.is_empty() || body.is_empty() {
                return l.show_settings_toast("A snippet needs a name and some text");
            }
            let kw = k.text().trim().to_string();
            let snippet = Snippet { name, text: body, keyword: Some(kw).filter(|k| !k.is_empty()) };
            l.update_config(|c| match editing {
                Some(i) if i < c.snippets.len() => c.snippets[i] = snippet,
                _ => c.snippets.push(snippet),
            });
            l.replace_settings_page("snippets", l.snippets_page(None));
        });
        page.append(&card(&[
            card_row("Name", None, Some(&name.upcast())),
            card_row("Text", Some("{date}, {time} and {clipboard} are filled in"), Some(&text.upcast())),
            card_row("Keyword", None, Some(&keyword.upcast())),
        ]));
        save.set_halign(gtk::Align::End);
        save.set_margin_top(10);
        page.append(&save);

        page.append(&heading("Snippets"));
        let mut rows = Vec::new();
        for (i, snippet) in snippets.iter().enumerate() {
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            if let Some(kw) = &snippet.keyword {
                let k = label(kw, "kbd");
                k.set_valign(gtk::Align::Center);
                controls.append(&k);
            }
            let edit = button("Edit", "flat-button");
            let weak = Rc::downgrade(self);
            edit.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.replace_settings_page("snippets", l.snippets_page(Some(i)));
                }
            });
            let delete = button("Delete", "flat-button");
            delete.add_css_class("danger");
            let weak = Rc::downgrade(self);
            delete.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.update_config(|c| {
                        if i < c.snippets.len() {
                            c.snippets.remove(i);
                        }
                    });
                    l.replace_settings_page("snippets", l.snippets_page(None));
                }
            });
            controls.append(&edit);
            controls.append(&delete);
            let summary = backslash_core::system::clipboard::summary(&snippet.text);
            rows.push(card_row(&snippet.name, Some(&summary), Some(&controls.upcast())));
        }
        if rows.is_empty() {
            rows.push(card_row("No snippets yet", Some("Add one above"), None));
        }
        page.append(&card(&rows));
        scrolled(&page).upcast()
    }

    fn replace_settings_page(&self, name: &str, content: gtk::Widget) {
        let Some(stack) = self.settings_stack.borrow().clone() else { return };
        if let Some(old) = stack.child_by_name(name) {
            stack.remove(&old);
        }
        stack.add_named(&content, Some(name));
        stack.set_visible_child_name(name);
    }

    fn advanced_page(self: &Rc<Self>) -> gtk::Widget {
        let page = page_box();
        let open = |path: std::path::PathBuf| {
            let b = button("Open", "flat-button");
            b.connect_clicked(move |_| {
                let _ = std::fs::create_dir_all(&path);
                if let Err(e) = crate::apps::open(&path.display().to_string()) {
                    eprintln!("backslash: {e}");
                }
            });
            b.upcast::<gtk::Widget>()
        };
        page.append(&heading("Files"));
        page.append(&card(&[
            card_row(
                "Config file",
                Some(&backslash_core::system::files::tilde(&backslash_core::config::Config::path())),
                Some(&open(backslash_core::config::Config::path())),
            ),
            card_row(
                "Scripts",
                Some(&backslash_core::system::files::tilde(&backslash_core::scripts::dir())),
                Some(&open(backslash_core::scripts::dir())),
            ),
            card_row(
                "Extensions",
                Some(&backslash_core::system::files::tilde(&paths::data_dir().join("extensions"))),
                Some(&open(paths::data_dir().join("extensions"))),
            ),
        ]));

        page.append(&heading("Runtime"));
        let deno = runtime::version().unwrap_or_else(|| "Not installed: downloaded when an extension needs it".into());
        page.append(&card(&[
            card_row("Deno", Some(&deno), None),
            card_row("Secrets", Some(backslash_core::secrets::Backend::detect().describe()), None),
            card_row("Backslash", Some(env!("CARGO_PKG_VERSION")), None),
        ]));

        let hidden = self.state.borrow().config.hidden_apps.clone();
        if !hidden.is_empty() {
            page.append(&heading("Hidden applications"));
            let mut rows = Vec::new();
            for id in hidden {
                let name = gtk::gio::DesktopAppInfo::new(&id)
                    .map(|a| gtk::prelude::AppInfoExt::display_name(&a).to_string())
                    .unwrap_or_else(|| id.clone());
                let show = button("Show", "flat-button");
                let weak = Rc::downgrade(self);
                let app = id.clone();
                show.connect_clicked(move |_| {
                    if let Some(l) = weak.upgrade() {
                        l.execute(UiAction::new("", Op::UnhideApp(app.clone())));
                        l.show_settings_page("advanced");
                    }
                });
                rows.push(card_row(&name, Some(&id), Some(&show.upcast())));
            }
            page.append(&card(&rows));
        }
        scrolled(&page).upcast()
    }
}

fn entry(placeholder: &str, text: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_placeholder_text(Some(placeholder));
    e.set_text(text);
    e.set_width_chars(22);
    e
}

#[cfg(test)]
mod tests {
    use gtk::gdk;

    #[test]
    fn accelerators() {
        let m = gdk::ModifierType::SUPER_MASK;
        assert_eq!(super::accel_from(gdk::Key::space, m).as_deref(), Some("Super+Space"));
        let m = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK;
        assert_eq!(super::accel_from(gdk::Key::k, m).as_deref(), Some("Ctrl+Alt+K"));
        assert_eq!(super::accel_from(gdk::Key::F5, gdk::ModifierType::empty()).as_deref(), Some("F5"));
        assert_eq!(super::accel_from(gdk::Key::a, gdk::ModifierType::empty()), None);
        assert_eq!(super::accel_from(gdk::Key::Control_L, gdk::ModifierType::CONTROL_MASK), None);
    }
}
