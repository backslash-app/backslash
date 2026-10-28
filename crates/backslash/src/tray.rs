//! The icon in the system tray, so you can see Backslash is running: a
//! StatusNotifierItem with a dbusmenu menu, on the application's own D-Bus
//! connection. KDE Plasma, Xfce, Cinnamon, Budgie, waybar and most bars show
//! it; GNOME needs the AppIndicator extension (installed by default on
//! Ubuntu). Without any tray host, nothing happens.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk_pixbuf, gio, glib};

/// What the user picked in the tray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Toggle,
    Settings,
    Update,
    Quit,
}

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
pub(crate) const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const ICON_NAME: &str = "app.backslash.Backslash";
const ICON_PNG: &[u8] = include_bytes!("../../../data/icons/backslash.png");

pub(crate) const ITEM_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <method name="ContextMenu"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Activate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="SecondaryActivate"><arg name="x" type="i" direction="in"/><arg name="y" type="i" direction="in"/></method>
    <method name="Scroll"><arg name="delta" type="i" direction="in"/><arg name="orientation" type="s" direction="in"/></method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewAttentionIcon"/>
    <signal name="NewOverlayIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus"><arg name="status" type="s"/></signal>
  </interface>
</node>"#;

pub(crate) const MENU_XML: &str = r#"<node>
  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
    <method name="GetLayout">
      <arg type="i" name="parentId" direction="in"/>
      <arg type="i" name="recursionDepth" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="u" name="revision" direction="out"/>
      <arg type="(ia{sv}av)" name="layout" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg type="ai" name="ids" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="a(ia{sv})" name="properties" direction="out"/>
    </method>
    <method name="GetProperty">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="name" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <method name="Event">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="eventId" direction="in"/>
      <arg type="v" name="data" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
    <method name="EventGroup">
      <arg type="a(isvu)" name="events" direction="in"/>
      <arg type="ai" name="idErrors" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg type="i" name="id" direction="in"/>
      <arg type="b" name="needUpdate" direction="out"/>
    </method>
    <method name="AboutToShowGroup">
      <arg type="ai" name="ids" direction="in"/>
      <arg type="ai" name="updatesNeeded" direction="out"/>
      <arg type="ai" name="idErrors" direction="out"/>
    </method>
    <signal name="ItemsPropertiesUpdated">
      <arg type="a(ia{sv})" name="updatedProps"/>
      <arg type="a(ias)" name="removedProps"/>
    </signal>
    <signal name="LayoutUpdated">
      <arg type="u" name="revision"/>
      <arg type="i" name="parent"/>
    </signal>
    <signal name="ItemActivationRequested">
      <arg type="i" name="id"/>
      <arg type="u" name="timestamp"/>
    </signal>
  </interface>
</node>"#;

/// One line of the menu.
struct MenuItem {
    id: i32,
    label: &'static str,
    separator: bool,
    event: Option<Event>,
}

/// The menu, which depends on whether an update is waiting.
fn menu(update: Option<&str>) -> Vec<(MenuItem, Option<String>)> {
    let item = |id, label, event| (MenuItem { id, label, separator: false, event: Some(event) }, None);
    let separator = |id| (MenuItem { id, label: "", separator: true, event: None }, None);
    let mut update_item = item(4, "Check for Updates…", Event::Update);
    if let Some(version) = update {
        update_item.1 = Some(format!("Update to {version}…"));
    }
    vec![
        item(1, "Open Backslash", Event::Toggle),
        item(2, "Settings…", Event::Settings),
        separator(3),
        update_item,
        separator(5),
        item(6, "Quit Backslash", Event::Quit),
    ]
}

fn item_properties(item: &MenuItem, label: Option<&str>) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    if item.separator {
        dict.insert_value("type", &"separator".to_variant());
    } else {
        dict.insert_value("label", &label.unwrap_or(item.label).to_variant());
        dict.insert_value("enabled", &true.to_variant());
        dict.insert_value("visible", &true.to_variant());
    }
    dict.end()
}

fn layout(update: Option<&str>) -> glib::Variant {
    let children = menu(update)
        .iter()
        .map(|(item, label)| {
            let node = glib::Variant::tuple_from_iter([
                item.id.to_variant(),
                item_properties(item, label.as_deref()),
                glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, Vec::<glib::Variant>::new()),
            ]);
            glib::Variant::from_variant(&node)
        })
        .collect::<Vec<_>>();
    let root_props = glib::VariantDict::new(None);
    root_props.insert_value("children-display", &"submenu".to_variant());
    glib::Variant::tuple_from_iter([
        0i32.to_variant(),
        root_props.end(),
        glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, children),
    ])
}

/// The icon at the sizes trays ask for, as `a(iiay)` ARGB32 big-endian.
fn pixmaps() -> glib::Variant {
    let ty = glib::VariantTy::new("(iiay)").expect("pixmap type");
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(ICON_PNG));
    let Ok(full) = gdk_pixbuf::Pixbuf::from_stream(&stream, None::<&gio::Cancellable>) else {
        return glib::Variant::array_from_iter_with_type(ty, Vec::<glib::Variant>::new());
    };
    let images = [16, 22, 24, 32, 48, 64]
        .into_iter()
        .filter_map(|size| {
            let pb = full.scale_simple(size, size, gdk_pixbuf::InterpType::Hyper)?.add_alpha(false, 0, 0, 0).ok()?;
            let (stride, bytes) = (pb.rowstride() as usize, pb.read_pixel_bytes());
            let mut argb = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size as usize {
                for x in 0..size as usize {
                    let p = &bytes[y * stride + x * 4..][..4];
                    argb.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
                }
            }
            Some(glib::Variant::tuple_from_iter([
                size.to_variant(),
                size.to_variant(),
                glib::Variant::array_from_fixed_array(&argb),
            ]))
        })
        .collect::<Vec<_>>();
    glib::Variant::array_from_iter_with_type(ty, images)
}

struct Tray {
    bus: gio::DBusConnection,
    service: String,
    registrations: Vec<gio::RegistrationId>,
    owner: Option<gio::OwnerId>,
    /// Stops watching for tray hosts (gio's `WatcherId` here can't be named).
    unwatch: Option<Box<dyn FnOnce()>>,
    update: Option<String>,
    revision: u32,
    pixmaps: glib::Variant,
    handler: Rc<dyn Fn(Event)>,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn with_tray<R>(f: impl FnOnce(&mut Tray) -> R) -> Option<R> {
    TRAY.with(|t| t.borrow_mut().as_mut().map(f))
}

fn emit(signal: &str, path: &str, iface: &str, args: Option<glib::Variant>) {
    with_tray(|t| {
        if let Err(e) = t.bus.emit_signal(None, path, iface, signal, args.as_ref()) {
            eprintln!("backslash: tray signal {signal}: {e}");
        }
    });
}

fn tooltip(update: Option<&str>) -> glib::Variant {
    let text = match update {
        Some(v) => format!("Backslash {v} is available"),
        None => "Backslash is running".to_string(),
    };
    let empty = glib::Variant::array_from_iter_with_type(
        glib::VariantTy::new("(iiay)").expect("pixmap type"),
        Vec::<glib::Variant>::new(),
    );
    glib::Variant::tuple_from_iter([ICON_NAME.to_variant(), empty, "Backslash".to_variant(), text.to_variant()])
}

fn item_property(name: &str) -> glib::Variant {
    let no_pixmaps = || {
        glib::Variant::array_from_iter_with_type(
            glib::VariantTy::new("(iiay)").expect("pixmap type"),
            Vec::<glib::Variant>::new(),
        )
    };
    let (update, pixmaps) =
        with_tray(|t| (t.update.clone(), t.pixmaps.clone())).unwrap_or_else(|| (None, no_pixmaps()));
    match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" => "backslash".to_variant(),
        "Title" => "Backslash".to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconName" => ICON_NAME.to_variant(),
        "IconThemePath" | "OverlayIconName" | "AttentionIconName" => "".to_variant(),
        "IconPixmap" if pixmaps.n_children() > 0 => pixmaps,
        "IconPixmap" | "OverlayIconPixmap" | "AttentionIconPixmap" => no_pixmaps(),
        "ToolTip" => tooltip(update.as_deref()),
        "ItemIsMenu" => false.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH).expect("menu path").to_variant(),
        _ => "".to_variant(),
    }
}

fn dispatch(event: Event) {
    // The handler may call back into this module: no borrow held.
    if let Some(handler) = with_tray(|t| t.handler.clone()) {
        handler(event);
    }
}

fn item_call(method: &str, invocation: gio::DBusMethodInvocation) {
    invocation.return_value(None);
    if matches!(method, "Activate" | "SecondaryActivate") {
        dispatch(Event::Toggle);
    }
}

fn menu_call(method: &str, params: glib::Variant, invocation: gio::DBusMethodInvocation) {
    let update = with_tray(|t| t.update.clone()).flatten();
    let revision = with_tray(|t| t.revision).unwrap_or(1);
    let items = menu(update.as_deref());
    let props_of = |id: i32| {
        items.iter().find(|(i, _)| i.id == id).map(|(i, label)| item_properties(i, label.as_deref())).unwrap_or_else(
            || {
                let d = glib::VariantDict::new(None);
                if id == 0 {
                    d.insert_value("children-display", &"submenu".to_variant());
                }
                d.end()
            },
        )
    };
    match method {
        "GetLayout" => {
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([
                revision.to_variant(),
                layout(update.as_deref()),
            ])));
        }
        "GetGroupProperties" => {
            let ids: Vec<i32> = params.child_value(0).get().unwrap_or_default();
            let ids = if ids.is_empty() { items.iter().map(|(i, _)| i.id).collect() } else { ids };
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
                if let Some(event) = items.iter().find(|(i, _)| i.id == id).and_then(|(i, _)| i.event) {
                    dispatch(event);
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
                    picked.extend(items.iter().find(|(i, _)| i.id == id).and_then(|(i, _)| i.event));
                }
            }
            invocation.return_value(Some(&(Vec::<i32>::new(),).to_variant()));
            for event in picked {
                dispatch(event);
            }
        }
        "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
        "AboutToShowGroup" => invocation.return_value(Some(&(Vec::<i32>::new(), Vec::<i32>::new()).to_variant())),
        _ => invocation.return_value(None),
    }
}

fn menu_property(name: &str) -> glib::Variant {
    match name {
        "Version" => 3u32.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        "IconThemePath" => Vec::<String>::new().to_variant(),
        _ => "".to_variant(),
    }
}

pub(crate) fn interface(xml: &str, name: &str) -> Option<gio::DBusInterfaceInfo> {
    gio::DBusNodeInfo::for_xml(xml).ok()?.lookup_interface(name)
}

pub(crate) fn register_with_watcher(bus: &gio::DBusConnection, service: &str) {
    bus.call(
        Some(WATCHER),
        "/StatusNotifierWatcher",
        WATCHER,
        "RegisterStatusNotifierItem",
        Some(&(service,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(e) = result {
                eprintln!("backslash: the tray did not accept the icon: {e}");
            }
        },
    );
}

/// Shows the tray icon. `handler` runs on the main thread for each pick.
pub fn start(bus: &gio::DBusConnection, handler: impl Fn(Event) + 'static) {
    if TRAY.with(|t| t.borrow().is_some()) {
        return;
    }
    let (Some(item_info), Some(menu_info)) =
        (interface(ITEM_XML, "org.kde.StatusNotifierItem"), interface(MENU_XML, "com.canonical.dbusmenu"))
    else {
        return;
    };
    let mut registrations = Vec::new();
    match bus
        .register_object(ITEM_PATH, &item_info)
        .method_call(|_, _, _, _, method, _, invocation| item_call(method, invocation))
        .property(|_, _, _, _, name| item_property(name))
        .build()
    {
        Ok(id) => registrations.push(id),
        Err(e) => return eprintln!("backslash: cannot show the tray icon: {e}"),
    }
    match bus
        .register_object(MENU_PATH, &menu_info)
        .method_call(|_, _, _, _, method, params, invocation| menu_call(method, params, invocation))
        .property(|_, _, _, _, name| menu_property(name))
        .build()
    {
        Ok(id) => registrations.push(id),
        Err(e) => eprintln!("backslash: cannot show the tray menu: {e}"),
    }
    let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    let owner = gio::bus_own_name_on_connection(
        bus,
        &service,
        gio::BusNameOwnerFlags::NONE,
        |_, _| {},
        |_, name| eprintln!("backslash: lost the tray name {name}"),
    );
    // Register now and every time a tray (re)starts.
    let watcher = gio::bus_watch_name_on_connection(
        bus,
        WATCHER,
        gio::BusNameWatcherFlags::NONE,
        {
            let service = service.clone();
            move |bus, _, _| register_with_watcher(&bus, &service)
        },
        |_, _| {},
    );
    TRAY.with(|t| {
        *t.borrow_mut() = Some(Tray {
            bus: bus.clone(),
            service,
            registrations,
            owner: Some(owner),
            unwatch: Some(Box::new(move || gio::bus_unwatch_name(watcher))),
            update: None,
            revision: 1,
            pixmaps: pixmaps(),
            handler: Rc::new(handler),
        })
    });
}

/// Removes the tray icon.
pub fn stop() {
    let Some(mut tray) = TRAY.with(|t| t.borrow_mut().take()) else { return };
    for id in tray.registrations.drain(..) {
        let _ = tray.bus.unregister_object(id);
    }
    if let Some(owner) = tray.owner.take() {
        gio::bus_unown_name(owner);
    }
    if let Some(unwatch) = tray.unwatch.take() {
        unwatch();
    }
    let _ = &tray.service;
}

/// Tells the tray a new version is available (or not any more).
pub fn set_update(version: Option<String>) {
    let changed = with_tray(|t| {
        if t.update == version {
            return false;
        }
        t.update = version;
        t.revision += 1;
        true
    });
    if changed == Some(true) {
        let revision = with_tray(|t| t.revision).unwrap_or(1);
        emit("NewToolTip", ITEM_PATH, "org.kde.StatusNotifierItem", None);
        emit("LayoutUpdated", MENU_PATH, "com.canonical.dbusmenu", Some((revision, 0i32).to_variant()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn introspection_parses() {
        assert!(interface(ITEM_XML, "org.kde.StatusNotifierItem").is_some());
        assert!(interface(MENU_XML, "com.canonical.dbusmenu").is_some());
    }

    #[test]
    fn menu_layout() {
        let l = layout(None);
        assert_eq!(l.type_().as_str(), "(ia{sv}av)");
        assert_eq!(l.child_value(2).n_children(), 6);
        let with_update = layout(Some("0.5.0")).print(false).to_string();
        assert!(with_update.contains("Update to 0.5.0…"), "{with_update}");
        assert!(layout(None).print(false).contains("Check for Updates…"));
    }

    #[test]
    fn icon_pixmaps() {
        let p = pixmaps();
        assert_eq!(p.type_().as_str(), "a(iiay)");
        assert_eq!(p.n_children(), 6);
        let first = p.child_value(0);
        assert_eq!(first.child_value(0).get::<i32>(), Some(16));
        assert_eq!(first.child_value(2).n_children(), 16 * 16 * 4);
    }
}
