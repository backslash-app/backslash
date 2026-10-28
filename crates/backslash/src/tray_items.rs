//! Tray icons of extensions' `menu-bar` commands: one StatusNotifierItem per
//! command, with its dbusmenu, on the application's D-Bus connection next to
//! Backslash's own icon (see `tray.rs`, whose interfaces they share).
//!
//! Trays draw the icon themselves, so extension icons that are not theme
//! icons (Backslash glyphs, emoji, files) are rendered to pixels here.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use backslash_core::protocol::{ActionKind, MenuItem, MenuView};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, pango};

use crate::{glyphs, icons, tray};

/// Ids of the clicked item, from the top level down.
pub type Path = Vec<String>;

const LABEL_XML: &str = r#"<property name="XAyatanaLabel" type="s" access="read"/>
    <property name="XAyatanaLabelGuide" type="s" access="read"/>
    <signal name="XAyatanaNewLabel"><arg name="label" type="s"/><arg name="guide" type="s"/></signal>
    <signal name="NewTitle"/>"#;

/// One line of the menu as dbusmenu sees it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Node {
    id: i32,
    label: String,
    enabled: bool,
    separator: bool,
    icon_name: Option<String>,
    icon_png: Option<Vec<u8>>,
    /// Set on items that do something when clicked.
    target: Option<Path>,
    children: Vec<i32>,
}

struct Data {
    name: String,
    label: String,
    tooltip: String,
    icon_name: String,
    pixmaps: glib::Variant,
    /// Index 0 is the root.
    nodes: Vec<Node>,
    revision: u32,
    on_click: Rc<dyn Fn(Path)>,
}

struct Entry {
    bus: gio::DBusConnection,
    path: String,
    registrations: Vec<gio::RegistrationId>,
    data: Data,
}

thread_local! {
    static ITEMS: RefCell<HashMap<String, Entry>> = RefCell::default();
    static NEXT: Cell<u32> = const { Cell::new(1) };
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    static RENDERER: RefCell<Option<gsk::Renderer>> = const { RefCell::new(None) };
}

fn with_data<R>(key: &str, f: impl FnOnce(&Data) -> R) -> Option<R> {
    ITEMS.with(|items| items.borrow().get(key).map(|e| f(&e.data)))
}

/// The keys of the menus shown.
pub fn shown() -> Vec<String> {
    ITEMS.with(|items| items.borrow().keys().cloned().collect())
}

/// Shows or updates the tray menu `key`. `name` is the command's title,
/// `fallback_icon` its icon when the menu names none. `on_click` gets the
/// path of a clicked item, on the main thread.
pub fn show(
    bus: &gio::DBusConnection,
    key: &str,
    name: &str,
    view: &MenuView,
    fallback_icon: Option<&str>,
    on_click: impl Fn(Path) + 'static,
) {
    let icon = view.icon.as_deref().or(fallback_icon);
    let (icon_name, pixmaps) = tray_icon(icon, view.icon_tint.as_deref());
    let data = Data {
        name: name.to_string(),
        label: view.title.clone().unwrap_or_default(),
        tooltip: view.tooltip.clone().or_else(|| view.title.clone()).unwrap_or_default(),
        icon_name,
        pixmaps,
        nodes: nodes(&view.items),
        revision: 1,
        on_click: Rc::new(on_click),
    };
    if !ITEMS.with(|items| items.borrow().contains_key(key)) {
        return register(bus, key, data);
    }
    let (bus, path, revision) = ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        let entry = items.get_mut(key).expect("checked above");
        let revision = entry.data.revision + 1;
        entry.data = Data { revision, ..data };
        (entry.bus.clone(), entry.path.clone(), revision)
    });
    let label = with_data(key, |d| d.label.clone()).unwrap_or_default();
    for signal in ["NewIcon", "NewTitle", "NewToolTip"] {
        emit(&bus, &path, "org.kde.StatusNotifierItem", signal, None);
    }
    emit(&bus, &path, "org.kde.StatusNotifierItem", "XAyatanaNewLabel", Some((label, "").to_variant()));
    emit(&bus, &format!("{path}/menu"), "com.canonical.dbusmenu", "LayoutUpdated", Some((revision, 0i32).to_variant()));
}

/// Removes the tray menu `key`.
pub fn remove(key: &str) {
    let Some(entry) = ITEMS.with(|items| items.borrow_mut().remove(key)) else { return };
    for id in entry.registrations {
        let _ = entry.bus.unregister_object(id);
    }
}

fn register(bus: &gio::DBusConnection, key: &str, data: Data) {
    let item_xml = tray::ITEM_XML.replace(r#"<signal name="NewTitle"/>"#, LABEL_XML);
    let (Some(item_info), Some(menu_info)) = (
        tray::interface(&item_xml, "org.kde.StatusNotifierItem"),
        tray::interface(tray::MENU_XML, "com.canonical.dbusmenu"),
    ) else {
        return;
    };
    let index = NEXT.with(|n| n.replace(n.get() + 1));
    let path = format!("/app/backslash/Tray/{index}");
    let mut registrations = Vec::new();
    let (k1, k2, k3, k4) = (key.to_string(), key.to_string(), key.to_string(), key.to_string());
    let menu_path = format!("{path}/menu");
    let menu_path_prop = menu_path.clone();
    match bus
        .register_object(&path, &item_info)
        .method_call(|_, _, _, _, _, _, invocation| invocation.return_value(None))
        .property(move |_, _, _, _, name| item_property(&k1, name, &menu_path_prop))
        .build()
    {
        Ok(id) => registrations.push(id),
        Err(e) => return eprintln!("backslash: cannot show a tray menu: {e}"),
    }
    match bus
        .register_object(&menu_path, &menu_info)
        .method_call(move |_, _, _, _, method, params, invocation| menu_call(&k2, method, params, invocation))
        .property(move |_, _, _, _, name| menu_property(&k3, name))
        .build()
    {
        Ok(id) => registrations.push(id),
        Err(e) => eprintln!("backslash: cannot show a tray menu: {e}"),
    }
    ITEMS.with(|items| {
        items.borrow_mut().insert(k4, Entry { bus: bus.clone(), path: path.clone(), registrations, data })
    });
    tray::register_with_watcher(bus, &path);
    // Register every menu again when a tray (re)starts.
    if !WATCHING.with(|w| w.replace(true)) {
        let _watch = gio::bus_watch_name_on_connection(
            bus,
            tray::WATCHER,
            gio::BusNameWatcherFlags::NONE,
            |bus, _, _| {
                let paths: Vec<String> = ITEMS.with(|items| items.borrow().values().map(|e| e.path.clone()).collect());
                for path in paths {
                    tray::register_with_watcher(&bus, &path);
                }
            },
            |_, _| {},
        );
    }
}

fn emit(bus: &gio::DBusConnection, path: &str, iface: &str, signal: &str, args: Option<glib::Variant>) {
    if let Err(e) = bus.emit_signal(None, path, iface, signal, args.as_ref()) {
        eprintln!("backslash: tray signal {signal}: {e}");
    }
}

fn empty_pixmaps() -> glib::Variant {
    glib::Variant::array_from_iter_with_type(
        glib::VariantTy::new("(iiay)").expect("pixmap type"),
        Vec::<glib::Variant>::new(),
    )
}

fn item_property(key: &str, name: &str, menu_path: &str) -> glib::Variant {
    let value = with_data(key, |d| match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" => format!("backslash-{key}").to_variant(),
        "Title" => d.name.to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconName" => d.icon_name.to_variant(),
        "IconPixmap" => d.pixmaps.clone(),
        "OverlayIconPixmap" | "AttentionIconPixmap" => empty_pixmaps(),
        "ToolTip" => glib::Variant::tuple_from_iter([
            d.icon_name.to_variant(),
            empty_pixmaps(),
            d.name.to_variant(),
            d.tooltip.to_variant(),
        ]),
        "ItemIsMenu" => true.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(menu_path).expect("menu path").to_variant(),
        "XAyatanaLabel" => d.label.to_variant(),
        _ => "".to_variant(),
    });
    value.unwrap_or_else(|| "".to_variant())
}

fn menu_property(_key: &str, name: &str) -> glib::Variant {
    match name {
        "Version" => 3u32.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        "IconThemePath" => Vec::<String>::new().to_variant(),
        _ => "".to_variant(),
    }
}

fn node_properties(node: &Node) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    if node.id == 0 || !node.children.is_empty() {
        dict.insert_value("children-display", &"submenu".to_variant());
    }
    if node.id == 0 {
        return dict.end();
    }
    if node.separator {
        dict.insert_value("type", &"separator".to_variant());
        return dict.end();
    }
    // `_` marks a mnemonic in dbusmenu labels.
    dict.insert_value("label", &node.label.replace('_', "__").to_variant());
    dict.insert_value("enabled", &node.enabled.to_variant());
    dict.insert_value("visible", &true.to_variant());
    if let Some(name) = &node.icon_name {
        dict.insert_value("icon-name", &name.to_variant());
    }
    if let Some(png) = &node.icon_png {
        dict.insert_value("icon-data", &glib::Variant::array_from_fixed_array(png));
    }
    dict.end()
}

fn layout_of(nodes: &[Node], id: i32) -> glib::Variant {
    let node = nodes.iter().find(|n| n.id == id).cloned().unwrap_or_default();
    let children = node.children.iter().map(|c| glib::Variant::from_variant(&layout_of(nodes, *c))).collect::<Vec<_>>();
    glib::Variant::tuple_from_iter([
        id.to_variant(),
        node_properties(&node),
        glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, children),
    ])
}

fn menu_call(key: &str, method: &str, params: glib::Variant, invocation: gio::DBusMethodInvocation) {
    let Some((nodes, revision, on_click)) = with_data(key, |d| (d.nodes.clone(), d.revision, d.on_click.clone()))
    else {
        return invocation.return_value(None);
    };
    let props_of = |id: i32| {
        nodes.iter().find(|n| n.id == id).map(node_properties).unwrap_or_else(|| glib::VariantDict::new(None).end())
    };
    let clicked = |id: i32| nodes.iter().find(|n| n.id == id).and_then(|n| n.target.clone());
    match method {
        "GetLayout" => {
            let parent: i32 = params.child_value(0).get().unwrap_or_default();
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([
                revision.to_variant(),
                layout_of(&nodes, parent),
            ])));
        }
        "GetGroupProperties" => {
            let ids: Vec<i32> = params.child_value(0).get().unwrap_or_default();
            let ids = if ids.is_empty() { nodes.iter().map(|n| n.id).collect() } else { ids };
            let entries = ids
                .into_iter()
                .map(|id| glib::Variant::tuple_from_iter([id.to_variant(), props_of(id)]))
                .collect::<Vec<_>>();
            let ty = glib::VariantTy::new("(ia{sv})").expect("props type");
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([glib::Variant::array_from_iter_with_type(
                ty, entries,
            )])));
        }
        "GetProperty" => {
            let id: i32 = params.child_value(0).get().unwrap_or_default();
            let name: String = params.child_value(1).get().unwrap_or_default();
            let value = glib::VariantDict::new(Some(&props_of(id)))
                .lookup_value(&name, None)
                .unwrap_or_else(|| "".to_variant());
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([glib::Variant::from_variant(&value)])));
        }
        "Event" => {
            let id: i32 = params.child_value(0).get().unwrap_or_default();
            let kind: String = params.child_value(1).get().unwrap_or_default();
            invocation.return_value(None);
            if kind == "clicked" {
                if let Some(path) = clicked(id) {
                    on_click(path);
                }
            }
        }
        "EventGroup" => {
            let events = params.child_value(0);
            let mut picked = Vec::new();
            for i in 0..events.n_children() {
                let e = events.child_value(i);
                let id: i32 = e.child_value(0).get().unwrap_or_default();
                let kind: String = e.child_value(1).get().unwrap_or_default();
                if kind == "clicked" {
                    picked.extend(clicked(id));
                }
            }
            invocation.return_value(Some(&(Vec::<i32>::new(),).to_variant()));
            for path in picked {
                on_click(path);
            }
        }
        "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
        "AboutToShowGroup" => invocation.return_value(Some(&(Vec::<i32>::new(), Vec::<i32>::new()).to_variant())),
        _ => invocation.return_value(None),
    }
}

// ---------------------------------------------------------------------------
// Menu structure

/// Flattens the menu: node 0 is the root, sections become a separator and a
/// greyed header.
fn nodes(items: &[MenuItem]) -> Vec<Node> {
    let mut nodes = vec![Node { id: 0, ..Default::default() }];
    let children = add_items(&mut nodes, items, &[]);
    nodes[0].children = children;
    nodes
}

fn add_items(nodes: &mut Vec<Node>, items: &[MenuItem], parent: &[String]) -> Vec<i32> {
    let mut ids = Vec::new();
    let mut section: Option<&str> = None;
    for (i, item) in items.iter().enumerate() {
        let next = item.section.as_deref();
        if next != section {
            if i > 0 {
                ids.push(push(nodes, Node { separator: true, ..Default::default() }));
            }
            if let Some(title) = next {
                ids.push(push(nodes, Node { label: title.to_string(), enabled: false, ..Default::default() }));
            }
            section = next;
        }
        let mut path = parent.to_vec();
        path.push(item.id.clone());
        let label = match &item.subtitle {
            Some(sub) => format!("{} — {sub}", item.title),
            None => item.title.clone(),
        };
        let clickable = item.action.as_ref().is_some_and(|a| !matches!(a.kind, ActionKind::Submenu { .. }));
        let (icon_name, icon_png) = item_icon(item.icon.as_deref(), item.icon_tint.as_deref());
        let id = push(
            nodes,
            Node {
                label,
                enabled: item.enabled,
                icon_name,
                icon_png,
                target: clickable.then(|| path.clone()),
                ..Default::default()
            },
        );
        let children = add_items(nodes, &item.items, &path);
        nodes.iter_mut().find(|n| n.id == id).expect("just added").children = children;
        ids.push(id);
    }
    ids
}

fn push(nodes: &mut Vec<Node>, mut node: Node) -> i32 {
    node.id = nodes.len() as i32;
    let id = node.id;
    nodes.push(node);
    id
}

// ---------------------------------------------------------------------------
// Icons

/// The default color of glyphs in the tray: panels are dark on GNOME
/// whatever the theme, and follow it elsewhere.
fn panel_color() -> gdk::RGBA {
    let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("GNOME"));
    let light = gnome || crate::theme::system_prefers_dark();
    gdk::RGBA::parse(if light { "#f2f2f2" } else { "#303030" }).expect("color")
}

fn tint_color(tint: Option<&str>) -> gdk::RGBA {
    let hex = match tint {
        Some("red") => "#e5484d",
        Some("orange") => "#f76b15",
        Some("yellow") => "#e5a50a",
        Some("green") => "#30a46c",
        Some("teal") => "#12a594",
        Some("blue") => "#3e8ef7",
        Some("purple") => "#8e4ec6",
        Some("pink") => "#d6409f",
        Some("gray") => "#8b8d98",
        Some(other) if other.starts_with('#') => other,
        _ => return panel_color(),
    };
    gdk::RGBA::parse(hex).unwrap_or_else(|_| panel_color())
}

/// Whether the tray can find `spec` by name in the icon theme.
fn is_theme_name(spec: &str) -> bool {
    !(glyphs::exists(spec) || icons::is_emoji(spec) || spec.contains(['/', ':']))
}

fn renderer() -> Option<gsk::Renderer> {
    RENDERER.with(|r| {
        if r.borrow().is_none() {
            let renderer: gsk::Renderer = gsk::CairoRenderer::new().upcast();
            let display = gdk::Display::default()?;
            renderer.realize_for_display(&display).ok()?;
            *r.borrow_mut() = Some(renderer);
        }
        r.borrow().clone()
    })
}

/// `spec` drawn in a `size`×`size` picture, for icons a tray can't load by name.
fn render(spec: &str, tint: Option<&str>, size: i32) -> Option<gdk::Texture> {
    let snapshot = gtk::Snapshot::new();
    let side = size as f32;
    if glyphs::exists(spec) {
        glyphs::draw(&snapshot, spec, side, &tint_color(tint));
    } else if icons::is_emoji(spec) {
        let layout = gtk::Label::new(None).create_pango_layout(Some(spec));
        let mut font = pango::FontDescription::new();
        font.set_absolute_size(side as f64 * 0.8 * pango::SCALE as f64);
        layout.set_font_description(Some(&font));
        let (_, logical) = layout.pixel_extents();
        snapshot.translate(&graphene::Point::new(
            (side - logical.width() as f32) / 2.0,
            (side - logical.height() as f32) / 2.0,
        ));
        snapshot.append_layout(&layout, &tint_color(tint));
    } else if spec.starts_with('/') {
        let texture = gdk::Texture::from_filename(spec).ok()?;
        snapshot.append_texture(&texture, &graphene::Rect::new(0.0, 0.0, side, side));
    } else {
        return None;
    }
    let node = snapshot.to_node()?;
    Some(renderer()?.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, side, side))))
}

/// The tray icon: a theme icon name, or pixels at the sizes trays ask for
/// (`a(iiay)`, ARGB32 big-endian).
fn tray_icon(spec: Option<&str>, tint: Option<&str>) -> (String, glib::Variant) {
    let spec = spec.unwrap_or("bs-apps");
    if is_theme_name(spec) {
        return (spec.to_string(), empty_pixmaps());
    }
    let images = [16, 22, 24, 32, 48]
        .into_iter()
        .filter_map(|size| {
            let texture = render(spec, tint, size)?;
            let mut downloader = gdk::TextureDownloader::new(&texture);
            downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
            let (bytes, stride) = downloader.download_bytes();
            let (w, h) = (texture.width() as usize, texture.height() as usize);
            let mut argb = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for x in 0..w {
                    let p = &bytes[y * stride + x * 4..][..4];
                    argb.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
                }
            }
            Some(glib::Variant::tuple_from_iter([
                (w as i32).to_variant(),
                (h as i32).to_variant(),
                glib::Variant::array_from_fixed_array(&argb),
            ]))
        })
        .collect::<Vec<_>>();
    let ty = glib::VariantTy::new("(iiay)").expect("pixmap type");
    (String::new(), glib::Variant::array_from_iter_with_type(ty, images))
}

/// A menu line's icon: by name, or as PNG data.
fn item_icon(spec: Option<&str>, tint: Option<&str>) -> (Option<String>, Option<Vec<u8>>) {
    match spec {
        None => (None, None),
        Some(spec) if is_theme_name(spec) => (Some(spec.to_string()), None),
        Some(spec) => (None, render(spec, tint, 32).map(|t| t.save_to_png_bytes().to_vec())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backslash_core::protocol::{Action, ActionKind};

    fn item(id: &str, section: Option<&str>, action: bool, items: Vec<MenuItem>) -> MenuItem {
        MenuItem {
            id: id.into(),
            title: id.to_uppercase(),
            section: section.map(Into::into),
            action: action.then(|| Action {
                id: format!("v1:{id}"),
                title: "Open".into(),
                icon: None,
                shortcut: None,
                destructive: false,
                confirm: None,
                section: None,
                kind: ActionKind::Callback,
            }),
            items,
            ..Default::default()
        }
    }

    #[test]
    fn sections_submenus_and_targets() {
        let items = vec![
            item("a", None, true, vec![]),
            item("b", Some("Review"), true, vec![]),
            item("c", Some("Review"), false, vec![item("d", None, true, vec![])]),
        ];
        let nodes = nodes(&items);
        let labels: Vec<_> = nodes[0]
            .children
            .iter()
            .map(|&i| (nodes[i as usize].label.as_str(), nodes[i as usize].separator))
            .collect();
        assert_eq!(labels, [("A", false), ("", true), ("Review", false), ("B", false), ("C", false)]);
        let c = nodes.iter().find(|n| n.label == "C").unwrap();
        assert!(c.target.is_none(), "no action: opens its submenu");
        let d = &nodes[c.children[0] as usize];
        assert_eq!(d.target, Some(vec!["c".to_string(), "d".to_string()]));
        let layout = layout_of(&nodes, 0);
        assert_eq!(layout.type_().as_str(), "(ia{sv}av)");
        assert_eq!(layout.child_value(2).n_children(), 5);
    }

    #[test]
    fn introspection_with_the_label_parses() {
        let xml = tray::ITEM_XML.replace(r#"<signal name="NewTitle"/>"#, LABEL_XML);
        assert!(tray::interface(&xml, "org.kde.StatusNotifierItem").is_some());
    }
}
