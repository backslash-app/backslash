//! The launcher window: a search bar with filters, a navigation stack of
//! views, a results list with a preview panel, a footer with the primary
//! action, and an action panel (Ctrl+K).
//!
//! The root view searches applications, extension commands and built-in
//! commands; extension views come from extension processes (see
//! `backslash_core::host`); built-in views (clipboard, files, windows, theme)
//! are implemented here.

mod appupdate;
mod background;
mod model;
mod rows;
mod settings;
mod settings_window;
mod signin;
mod store;
mod welcome;
mod widgets;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use backslash_core::config::{Config, Reopen};
use backslash_core::host::{Incoming, RpcError, Session};
use backslash_core::manifest::Runtime;
use backslash_core::protocol::{self as proto, Effect, FieldKind, Filtering, FormView, LaunchType, View};
use backslash_core::registry::{CommandEntry, Registry};
use backslash_core::scripts::{self, Mode as ScriptMode, Script};
use backslash_core::search::{Ranker, Usage};
use backslash_core::system::clipboard::ClipboardHistory;
use backslash_core::system::files;
use backslash_core::system::windows::{Backend, WindowInfo};
use backslash_core::{approvals, paths, runner, runtime, secrets};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{json, Map, Value};

use self::model::{tint_for, Builtin, Filter, FormInput, FormPurpose, Frame, FrameKind, Launched, Op, Row, UiAction};
use self::widgets::{key_hint, label, row_widget};
use crate::apps::{self, App};
use crate::currency::Currency;
use crate::icons::{self, Icon};
use crate::theme::Styles;
use crate::{autostart, clipboard, hotkey, layer, markdown, paste, theme};

pub use self::rows::now;

pub struct Options {
    pub hidden: bool,
    pub dev_dir: Option<PathBuf>,
    /// Root search item to open instead of showing the window (`backslash open`).
    pub open: Option<String>,
}

const WIDTH: i32 = 860;
const HEIGHT: i32 = 540;
const LIST_WIDTH: i32 = 450;

pub fn run(options: Options) -> anyhow::Result<()> {
    layer::reexec_if_needed(Config::load().layer_shell);

    let dev = options.dev_dir.is_some();
    let app_id = if dev { "app.backslash.Backslash.Devel" } else { "app.backslash.Backslash" };
    let app = gtk::Application::builder().application_id(app_id).build();

    // `backslash open KEY` asks the running instance, if any, and exits.
    if let Some(key) = &options.open {
        if activate_remote(app_id, key) {
            return Ok(());
        }
    }

    let launcher: Rc<RefCell<Option<Rc<Launcher>>>> = Rc::default();
    let first_activation = Cell::new(true);
    let dev_dir = options.dev_dir.clone();
    let hidden = options.hidden;
    let open = RefCell::new(options.open.clone());

    let open_action = gio::SimpleAction::new("open", Some(glib::VariantTy::STRING));
    open_action.connect_activate({
        let launcher = launcher.clone();
        move |_, key| {
            let (Some(l), Some(key)) = (launcher.borrow().clone(), key.and_then(|k| k.str().map(str::to_owned))) else {
                return;
            };
            l.open_key(&key);
        }
    });
    app.add_action(&open_action);

    app.connect_startup({
        let launcher = launcher.clone();
        move |app| {
            let Some(display) = gdk::Display::default() else { return };
            let styles = Styles::install(&display);
            launcher.replace(Some(Launcher::new(app, styles, dev_dir.clone())));
        }
    });
    // Every later `backslash` invocation reaches the running instance over
    // D-Bus and lands here, which makes `backslash` itself the toggle.
    app.connect_activate(move |_| {
        let Some(l) = launcher.borrow().clone() else { return };
        if first_activation.replace(false) {
            l.start_services();
            if let Some(key) = open.take() {
                return l.open_key(&key);
            }
            if hidden {
                return;
            }
        }
        l.toggle();
    });

    let code = app.run_with_args::<&str>(&[]);
    if code != glib::ExitCode::SUCCESS {
        anyhow::bail!("launcher exited with an error");
    }
    Ok(())
}

/// Calls the `open` action of a running instance over D-Bus (what
/// `GApplication` exports). False when none is running.
fn activate_remote(app_id: &str, key: &str) -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else { return false };
    let path = format!("/{}", app_id.replace('.', "/"));
    let args = glib::Variant::tuple_from_iter([
        "open".to_variant(),
        glib::Variant::array_from_iter_with_type(glib::VariantTy::VARIANT, [key.to_variant().to_variant()]),
        glib::VariantDict::new(None).end(),
    ]);
    bus.call_sync(
        Some(app_id),
        &path,
        "org.gtk.Actions",
        "Activate",
        Some(&args),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        2000,
        None::<&gio::Cancellable>,
    )
    .is_ok()
}

/// Results of the file search, filled from a background thread.
#[derive(Default)]
struct FilesState {
    results: Vec<PathBuf>,
    recent: Vec<PathBuf>,
    generation: u64,
    cancel: Arc<AtomicBool>,
}

pub(super) struct State {
    registry: Registry,
    config: Config,
    apps: Vec<App>,
    commands: Vec<CommandEntry>,
    usage: Usage,
    ranker: Ranker,
    frames: Vec<Frame>,
    rows: Vec<Row>,
    confirm: Option<UiAction>,
    search: Option<(Rc<Session>, u64)>,
    search_timer: Option<glib::SourceId>,
    toast_serial: Rc<Cell<u64>>,
    form: Vec<(proto::Field, FormInput)>,
    /// Keeps the selection on the same item when a view refreshes.
    selected_key: Option<String>,
    filter: Filter,
    clipboard: ClipboardHistory,
    files: FilesState,
    windows: Vec<WindowInfo>,
    window_backend: Option<Backend>,
    currency: Currency,
    autostart: bool,
    catalog: store::CatalogState,
    /// Deno is being downloaded.
    runtime_busy: bool,
    /// Desktop facts shown by the welcome screen.
    desktop: welcome::Desktop,
    /// How to paste into other apps here, if possible.
    paste: Option<paste::Method>,
    /// App id of the window focused before the launcher showed (paste target).
    paste_target: Option<String>,
    /// Script commands from ~/.config/backslash/scripts.
    scripts: Vec<Script>,
    /// Windows come from a KWin script (KDE Plasma on Wayland).
    kwin: bool,
    /// GNOME without a window extension yet.
    gnome_helper_needed: bool,
    /// What commands said about themselves ("3 in progress").
    command_meta: CommandMeta,
    /// View updates that arrived before their view was shown.
    early_updates: Vec<(Rc<Session>, View)>,
    /// When the window last hid, to know whether to restore it as it was.
    hidden_at: Option<std::time::Instant>,
    /// A new version of Backslash itself.
    app_update: appupdate::AppUpdate,
    /// Commands running on their own: intervals and tray menus.
    background: background::Background,
}

/// Subtitles set by commands with `updateCommandMetadata`, kept on disk.
#[derive(Default)]
pub(super) struct CommandMeta(std::collections::BTreeMap<String, String>);

impl CommandMeta {
    fn path() -> PathBuf {
        paths::state_dir().join("command-metadata.json")
    }

    fn load() -> Self {
        Self(backslash_core::store::read_json(&Self::path()))
    }

    pub fn get(&self, command: &str) -> Option<&str> {
        self.0.get(command).map(String::as_str)
    }

    fn set(&mut self, command: &str, subtitle: Option<String>) {
        match subtitle.filter(|s| !s.trim().is_empty()) {
            Some(s) => self.0.insert(command.to_string(), s),
            None => self.0.remove(command),
        };
        if let Err(e) = backslash_core::store::write_json(&Self::path(), &self.0) {
            eprintln!("backslash: cannot save command metadata: {e}");
        }
    }
}

pub struct Launcher {
    window: gtk::ApplicationWindow,
    styles: Styles,
    layer_shell: bool,
    entry: gtk::Entry,
    /// "Linear › My Issues": where you are; clicking goes back.
    back: gtk::Button,
    crumb_icon: gtk::Box,
    crumb_label: gtk::Label,
    search_icon: gtk::Widget,
    chips: gtk::Box,
    /// Chips of an extension list's `filter`, under the search bar.
    filters: gtk::Box,
    filters_divider: gtk::Box,
    spinner: gtk::Spinner,
    pages: gtk::Stack,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    preview: gtk::Box,
    /// Grid layout of extension lists: cells in row order, and the selected one.
    grid_scroller: gtk::ScrolledWindow,
    grid_box: gtk::Box,
    grid_cells: RefCell<Vec<gtk::Widget>>,
    grid_selected: Cell<usize>,
    sections: Rc<RefCell<Vec<Option<String>>>>,
    empty_title: gtk::Label,
    empty_subtitle: gtk::Label,
    detail: gtk::Label,
    detail_stats: gtk::Box,
    metadata: gtk::Box,
    form_box: gtk::Box,
    context: gtk::Label,
    /// "Backslash 0.5.0 is available", after the context line.
    update_pill: gtk::Button,
    toast: gtk::Label,
    primary: gtk::Label,
    primary_key: gtk::Box,
    actions_hint: gtk::Box,
    action_panel: gtk::Box,
    actions_entry: gtk::Entry,
    actions_context: gtk::Label,
    popover_list: gtk::ListBox,
    /// Scrolls the action panel's list (long lists, submenus).
    popover_scroll: gtk::ScrolledWindow,
    /// Actions shown in the panel (after filtering), in list order.
    popover_actions: RefCell<Vec<UiAction>>,
    /// Open submenus: title and actions of each level, the top one shown.
    menu_levels: RefCell<Vec<(String, Vec<UiAction>)>>,
    state: RefCell<State>,
    dev_dir: Option<PathBuf>,
    /// Set while the code (not the user) changes the search text.
    setting_query: Cell<bool>,
    /// `backslash dev`: watches the extension's files for hot reload.
    monitors: RefCell<Vec<gio::FileMonitor>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
    shortcuts: Rc<hotkey::Shortcuts>,
    paster: Rc<paste::Paster>,
    kwin: Option<Rc<crate::kwin::Kwin>>,
    /// The settings window, while open, and its pages.
    settings_window: RefCell<Option<gtk::Window>>,
    settings_stack: RefCell<Option<gtk::Stack>>,
    _hold: gio::ApplicationHoldGuard,
}

impl Launcher {
    fn new(app: &gtk::Application, styles: Styles, dev_dir: Option<PathBuf>) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Backslash")
            .default_width(WIDTH)
            .default_height(HEIGHT)
            .resizable(false)
            .decorated(false)
            .build();
        window.add_css_class("launcher");
        let layer_shell = layer::setup(&window);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_overflow(gtk::Overflow::Hidden);
        panel.set_size_request(WIDTH - 28, HEIGHT - 28);

        // Header: where you are (or the search icon), query, chips, spinner.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("header");
        let back = gtk::Button::new();
        back.add_css_class("crumb");
        back.set_valign(gtk::Align::Center);
        back.set_focusable(false);
        back.set_visible(false);
        let crumb = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let crumb_icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let crumb_label = gtk::Label::new(None);
        crumb_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        crumb_label.set_max_width_chars(36);
        crumb.append(&crumb_icon);
        crumb.append(&crumb_label);
        back.set_child(Some(&crumb));
        back.set_tooltip_text(Some("Back (Esc)"));
        let search_icon: gtk::Widget = crate::glyphs::Glyph::new("bs-search", 18).upcast();
        search_icon.add_css_class("search-icon");
        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_has_frame(false);
        entry.set_hexpand(true);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.set_valign(gtk::Align::Center);
        let spinner = gtk::Spinner::new();
        header.append(&back);
        header.append(&search_icon);
        header.append(&entry);
        header.append(&spinner);
        header.append(&chips);

        let divider = || {
            let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            d.add_css_class("divider");
            d
        };

        // Pages
        let pages = gtk::Stack::new();
        pages.set_vexpand(true);
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_transition_duration(90);

        let list = gtk::ListBox::new();
        list.add_css_class("results");
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.set_activate_on_single_click(true);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .width_request(LIST_WIDTH)
            .build();
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 16);
        preview.add_css_class("preview");
        preview.set_hexpand(true);
        preview.set_vexpand(true);
        // Long panels scroll instead of growing the window.
        let preview_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&preview)
            .hexpand(true)
            .build();
        preview_scroll.add_css_class("preview-scroll");
        let list_page = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        list_page.append(&scroller);
        list_page.append(&divider());
        list_page.append(&preview_scroll);
        pages.add_named(&list_page, Some("list"));

        let grid_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        grid_box.add_css_class("grid");
        let grid_scroller =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&grid_box).build();
        pages.add_named(&grid_scroller, Some("grid"));

        let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
        empty.add_css_class("empty");
        empty.set_valign(gtk::Align::Center);
        let empty_icon = crate::glyphs::Glyph::new("bs-search", 40);
        empty_icon.set_margin_bottom(6);
        let empty_title = label("", "title");
        let empty_subtitle = label("", "subtitle");
        empty_title.set_xalign(0.5);
        empty_subtitle.set_xalign(0.5);
        empty.append(&empty_icon);
        empty.append(&empty_title);
        empty.append(&empty_subtitle);
        pages.add_named(&empty, Some("empty"));

        let detail_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let detail = gtk::Label::new(None);
        detail.add_css_class("detail");
        detail.set_wrap(true);
        detail.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        detail.set_xalign(0.0);
        detail.set_yalign(0.0);
        detail.set_selectable(true);
        detail.set_can_focus(false);
        detail.set_hexpand(true);
        let detail_stats = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail_stats.add_css_class("detail-stats");
        let detail_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail_column.append(&detail_stats);
        detail_column.append(&detail);
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&detail_column)
            .hexpand(true)
            .build();
        let metadata = gtk::Box::new(gtk::Orientation::Vertical, 4);
        metadata.add_css_class("metadata");
        metadata.set_size_request(270, -1);
        metadata.set_hexpand(false);
        let metadata_scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&metadata).build();
        metadata.set_vexpand(true);
        detail_row.append(&detail_scroll);
        detail_row.append(&metadata_scroll);
        pages.add_named(&detail_row, Some("detail"));

        let form_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        form_box.add_css_class("form");
        let form_scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&form_box).build();
        pages.add_named(&form_scroll, Some("form"));

        // Footer: context or toast, primary action, action panel hint.
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.add_css_class("footer");
        let context = label("Backslash", "context");
        context.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let toast = label("", "toast");
        toast.set_ellipsize(gtk::pango::EllipsizeMode::End);
        toast.set_visible(false);
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        left.set_hexpand(true);
        let update_pill = appupdate::pill();
        left.append(&context);
        left.append(&update_pill);
        left.append(&toast);
        let primary = label("", "primary");
        let primary_key = key_hint("", "↵");
        let vsep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        vsep.add_css_class("vsep");
        vsep.set_valign(gtk::Align::Center);
        let actions_hint = key_hint("Actions", "Ctrl K");
        footer.append(&left);
        footer.append(&primary);
        footer.append(&primary_key);
        footer.append(&vsep);
        footer.append(&actions_hint);
        actions_hint.bind_property("visible", &vsep, "visible").sync_create().build();

        // Filter chips of extension lists.
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        filters.add_css_class("filters");
        filters.set_visible(false);
        let filters_divider = divider();
        filters_divider.set_visible(false);

        panel.append(&header);
        panel.append(&divider());
        panel.append(&filters);
        panel.append(&filters_divider);
        panel.append(&pages);
        panel.append(&divider());
        panel.append(&footer);

        // Action panel: an overlay inside the window rather than a popup
        // surface, so focus never leaves the launcher (popups behave
        // differently on every compositor).
        let action_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        action_panel.add_css_class("actions-panel");
        action_panel.set_halign(gtk::Align::End);
        action_panel.set_valign(gtk::Align::End);
        action_panel.set_margin_end(24);
        action_panel.set_margin_bottom(60);
        action_panel.set_size_request(340, -1);
        action_panel.set_visible(false);
        let actions_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions_head.add_css_class("actions-head");
        let actions_context = label("", "actions-context");
        actions_context.set_ellipsize(gtk::pango::EllipsizeMode::End);
        actions_context.set_max_width_chars(18);
        let actions_entry = gtk::Entry::new();
        actions_entry.add_css_class("actions-search");
        actions_entry.set_has_frame(false);
        actions_entry.set_hexpand(true);
        actions_entry.set_placeholder_text(Some("Search actions…"));
        actions_head.append(&actions_context);
        actions_head.append(&actions_entry);
        action_panel.append(&actions_head);
        action_panel.append(&divider());
        let popover_list = gtk::ListBox::new();
        popover_list.set_selection_mode(gtk::SelectionMode::Browse);
        let popover_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(360)
            .child(&popover_list)
            .build();
        action_panel.append(&popover_scroll);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&panel));
        overlay.add_overlay(&action_panel);
        window.set_child(Some(&overlay));

        let config = Config::load();
        styles.apply(&config, &[window.upcast_ref()]);
        let mut registry = Registry::scan(&paths::extension_dirs());
        if let Some(dir) = &dev_dir {
            registry.add_dir(dir);
        }
        let apps = apps::load(&config.hidden_apps);
        let commands = registry.commands(|id| config.is_enabled(id));
        for (dir, err) in &registry.errors {
            eprintln!("backslash: skipping {}: {err}", dir.display());
        }
        let clipboard = ClipboardHistory::load(paths::state_dir().join("clipboard.json"), config.clipboard.max_items);

        let window_backend = Backend::detect();
        let kwin = if window_backend.is_none() { crate::kwin::Kwin::new() } else { None };
        let launcher = Rc::new(Self {
            window,
            styles,
            layer_shell,
            entry,
            back,
            crumb_icon,
            crumb_label,
            search_icon,
            chips,
            filters,
            filters_divider,
            spinner,
            pages,
            list,
            scroller,
            preview,
            grid_scroller,
            grid_box,
            grid_cells: RefCell::default(),
            grid_selected: Cell::new(0),
            sections: Rc::default(),
            empty_title,
            empty_subtitle,
            detail,
            detail_stats,
            metadata,
            form_box,
            context,
            update_pill,
            toast,
            primary,
            primary_key,
            actions_hint,
            action_panel,
            actions_entry,
            actions_context,
            popover_list,
            popover_scroll,
            popover_actions: RefCell::default(),
            menu_levels: RefCell::default(),
            state: RefCell::new(State {
                registry,
                config,
                apps,
                commands,
                usage: Usage::load(paths::state_dir().join("usage.json")),
                ranker: Ranker::default(),
                frames: vec![Frame { kind: FrameKind::Root, query: String::new(), selected: None }],
                rows: Vec::new(),
                confirm: None,
                search: None,
                search_timer: None,
                toast_serial: Rc::default(),
                form: Vec::new(),
                selected_key: None,
                filter: Filter::All,
                clipboard,
                files: FilesState::default(),
                windows: Vec::new(),
                window_backend,
                currency: Currency::new(),
                autostart: autostart::enabled(),
                catalog: store::CatalogState::default(),
                runtime_busy: false,
                desktop: welcome::Desktop { shortcut: hotkey::Status::Disabled, layer_shell: false },
                paste: paste::detect(),
                paste_target: None,
                scripts: scripts::discover(&[scripts::dir()]),
                kwin: kwin.is_some(),
                command_meta: CommandMeta::load(),
                early_updates: Vec::new(),
                hidden_at: None,
                app_update: Default::default(),
                background: Default::default(),
                gnome_helper_needed: window_backend.is_none()
                    && backslash_core::system::windows::is_gnome()
                    && std::env::var_os("WAYLAND_DISPLAY").is_some(),
            }),
            dev_dir,
            setting_query: Cell::new(false),
            monitors: RefCell::default(),
            reload_timer: RefCell::default(),
            shortcuts: Rc::default(),
            paster: Rc::default(),
            kwin,
            settings_window: RefCell::default(),
            settings_stack: RefCell::default(),
            _hold: app.hold(),
        });
        launcher.build_chips();
        launcher.connect_signals();
        launcher.render();
        launcher
    }

    /// `backslash dev`: reload when the extension's manifest or commands change.
    fn watch_dev_dir(self: &Rc<Self>) {
        let Some(dir) = self.dev_dir.clone() else { return };
        for path in [dir.clone(), dir.join("commands")] {
            let Ok(monitor) =
                gio::File::for_path(&path).monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
            else {
                continue;
            };
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, event| {
                if !matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                ) {
                    return;
                }
                let Some(l) = weak.upgrade() else { return };
                // Editors save in several steps: wait for the last one.
                if let Some(timer) = l.reload_timer.borrow_mut().take() {
                    timer.remove();
                }
                let weak = Rc::downgrade(&l);
                let timer = glib::timeout_add_local_once(Duration::from_millis(250), move || {
                    if let Some(l) = weak.upgrade() {
                        l.reload_timer.borrow_mut().take();
                        l.hot_reload();
                    }
                });
                *l.reload_timer.borrow_mut() = Some(timer);
            });
            self.monitors.borrow_mut().push(monitor);
        }
        eprintln!("backslash: watching {} for changes", dir.display());
    }

    /// Reloads the dev extension and restarts its open command, if any.
    fn hot_reload(self: &Rc<Self>) {
        self.reload();
        let Some(dir) = &self.dev_dir else { return };
        let id = match backslash_core::manifest::Manifest::load(dir) {
            Ok(m) => m.extension.id,
            Err(e) => return self.fail(format!("{e:#}")),
        };
        let open = {
            let s = self.state.borrow();
            s.frames
                .iter()
                .position(|f| matches!(&f.kind, FrameKind::Extension { session, .. } if session.extension_id == id))
                .map(|i| match &s.frames[i].kind {
                    FrameKind::Extension { launched, .. } => (i, launched.clone()),
                    _ => unreachable!(),
                })
        };
        match open {
            Some((index, launched)) => {
                {
                    let mut s = self.state.borrow_mut();
                    s.frames.truncate(index);
                    s.search = None;
                    s.selected_key = None;
                }
                let query = self.state.borrow().frames.last().map(|f| f.query.clone()).unwrap_or_default();
                self.set_query(&query);
                self.render();
                self.start_command(&launched.command_id, Some(launched.arguments.clone()));
                eprintln!("backslash: reloaded {}", launched.command_id);
                self.show_toast("Reloaded", proto::ToastStyle::Success);
            }
            None => {
                self.render();
                self.show_toast("Extension reloaded", proto::ToastStyle::Success);
            }
        }
    }

    /// Background services, started once the application is registered.
    fn start_services(self: &Rc<Self>) {
        self.watch_dev_dir();
        self.start_background();
        // Pictures (avatars, favicons) arrive after the rows: redraw them.
        let weak = Rc::downgrade(self);
        crate::images::on_ready(move || {
            let Some(l) = weak.upgrade() else { return };
            if l.window.is_visible() && !l.action_panel.is_visible() && !l.is_form() {
                l.render();
            }
        });
        let clipboard_enabled = self.state.borrow().config.clipboard.enabled;
        self.bind_shortcuts();
        if self.dev_dir.is_none() {
            self.watch_updates();
            self.watch_app_updates();
            self.start_tray();
        }
        if clipboard_enabled {
            let weak = Rc::downgrade(self);
            clipboard::watch(&WidgetExt::display(&self.window), move |text| {
                if let Some(l) = weak.upgrade() {
                    l.on_clipboard_text(text);
                }
            });
        }
        let weak = Rc::downgrade(self);
        theme::watch_system(move || {
            if let Some(l) = weak.upgrade() {
                l.apply_style();
            }
        });
        let weak = Rc::downgrade(self);
        self.state.borrow().currency.connect_update(move || {
            if let Some(l) = weak.upgrade() {
                if l.is_root() && l.window.is_visible() {
                    l.render();
                }
            }
        });
    }

    fn on_clipboard_text(self: &Rc<Self>, text: String) {
        let added = self.state.borrow_mut().clipboard.add(&text);
        let showing =
            matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::Builtin(Builtin::Clipboard)));
        if added && showing {
            self.render();
        }
    }

    fn build_chips(self: &Rc<Self>) {
        for filter in Filter::ALL {
            let chip = gtk::Button::with_label(filter.label());
            chip.add_css_class("chip");
            chip.set_focusable(false);
            let weak = Rc::downgrade(self);
            chip.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.set_filter(filter);
                }
            });
            self.chips.append(&chip);
        }
    }

    fn set_filter(self: &Rc<Self>, filter: Filter) {
        self.state.borrow_mut().filter = filter;
        self.state.borrow_mut().selected_key = None;
        self.render();
    }

    fn cycle_filter(self: &Rc<Self>, delta: i32) {
        let current = self.state.borrow().filter;
        let i = Filter::ALL.iter().position(|f| *f == current).unwrap_or(0) as i32;
        let next = (i + delta).rem_euclid(Filter::ALL.len() as i32) as usize;
        self.set_filter(Filter::ALL[next]);
    }

    fn update_chips(&self) {
        let root = self.is_root();
        self.chips.set_visible(root);
        let filter = self.state.borrow().filter;
        let mut child = self.chips.first_child();
        let mut i = 0;
        while let Some(chip) = child {
            if Filter::ALL[i] == filter {
                chip.add_css_class("active");
            } else {
                chip.remove_css_class("active");
            }
            child = chip.next_sibling();
            i += 1;
        }
    }

    fn connect_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let with = move |f: fn(&Rc<Launcher>)| {
            let weak = weak.clone();
            move || {
                if let Some(l) = weak.upgrade() {
                    f(&l)
                }
            }
        };

        let changed = with(|l| l.on_query_changed());
        self.entry.connect_changed(move |_| changed());

        let back = with(|l| l.pop());
        self.back.connect_clicked(move |_| back());

        let update = with(|l| l.open_app_update());
        self.update_pill.connect_clicked(move |_| update());

        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            if let Some(l) = weak.upgrade() {
                l.run_row(row.index(), 0);
            }
        });
        let weak = Rc::downgrade(self);
        self.list.connect_row_selected(move |_, _| {
            if let Some(l) = weak.upgrade() {
                l.update_footer();
                l.update_preview();
            }
        });

        let sections = self.sections.clone();
        self.list.set_header_func(move |row, before| {
            let sections = sections.borrow();
            let current = sections.get(row.index() as usize).cloned().flatten();
            let previous = before.and_then(|b| sections.get(b.index() as usize).cloned().flatten());
            match current {
                Some(s) if previous.as_ref() != Some(&s) => row.set_header(Some(&label(&s, "section-header"))),
                _ => row.set_header(gtk::Widget::NONE),
            }
        });

        let weak = Rc::downgrade(self);
        self.popover_list.connect_row_activated(move |_, row| {
            if let Some(l) = weak.upgrade() {
                let action = l.popover_actions.borrow().get(row.index() as usize).cloned();
                if let Some(action) = action {
                    l.pick_action(action);
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.actions_entry.connect_changed(move |_| {
            if let Some(l) = weak.upgrade() {
                l.on_actions_query_changed();
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| match weak.upgrade() {
            Some(l) => l.on_key(key, mods),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);

        // Hide when another window takes the focus, like any launcher. Wait a
        // little first: compositors briefly take the keyboard away on layout or
        // keymap changes, which must not close the window.
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |w| {
            if w.is_active() {
                return;
            }
            let weak = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(200), move || {
                let Some(l) = weak.upgrade() else { return };
                let w = &l.window;
                if !w.is_active() && w.is_visible() && !l.action_panel.is_visible() && l.dev_dir.is_none() {
                    l.hide();
                }
            });
        });
        self.window.connect_close_request(move |w| {
            w.set_visible(false);
            glib::Propagation::Stop
        });

        // Keep the application list fresh when packages are installed or removed.
        let weak = Rc::downgrade(self);
        gio::AppInfoMonitor::get().connect_changed(move |_| {
            if let Some(l) = weak.upgrade() {
                let hidden = l.state.borrow().config.hidden_apps.clone();
                l.state.borrow_mut().apps = apps::load(&hidden);
                if l.is_root() {
                    l.render();
                }
            }
        });
    }

    // -----------------------------------------------------------------------
    /// Asks the desktop for the launcher's shortcut and those of items
    /// (`[hotkeys]`). Does nothing when they did not change.
    fn bind_shortcuts(self: &Rc<Self>) {
        if self.dev_dir.is_some() {
            return;
        }
        let bindings = {
            let s = self.state.borrow();
            let mut bindings = Vec::new();
            if !s.config.hotkey.is_empty() {
                bindings.push(hotkey::Binding {
                    id: "toggle".into(),
                    description: "Show or hide Backslash".into(),
                    accel: s.config.hotkey.clone(),
                });
            }
            for (key, accel) in s.config.hotkeys.iter().filter(|(_, a)| !a.trim().is_empty()) {
                let title = rows::row_for_key(&s, key).map_or_else(|| key.clone(), |r| r.title);
                bindings.push(hotkey::Binding {
                    id: key.clone(),
                    description: format!("Backslash: {title}"),
                    accel: accel.clone(),
                });
            }
            bindings
        };
        let weak = Rc::downgrade(self);
        let weak_status = Rc::downgrade(self);
        let app_id = self.window.application().and_then(|a| a.application_id()).unwrap_or_default();
        self.shortcuts.bind(
            &app_id,
            bindings,
            Rc::new(move |id| {
                if let Some(l) = weak.upgrade() {
                    if id == "toggle" {
                        l.toggle();
                    } else {
                        l.open_key(id);
                    }
                }
            }),
            move |status| {
                eprintln!("backslash: {status}");
                // The welcome screen shows whether the shortcut works.
                if let Some(l) = weak_status.upgrade() {
                    let on_welcome = matches!(
                        l.state.borrow().frames.last().map(|f| &f.kind),
                        Some(FrameKind::Builtin(Builtin::Welcome))
                    );
                    if on_welcome && l.window.is_visible() {
                        l.render();
                    }
                }
            },
        );
    }

    /// Runs the main action of a root search item, by key (`app:…`,
    /// `cmd:…`…): what item hotkeys and `backslash open` do.
    pub fn open_key(self: &Rc<Self>, key: &str) {
        self.reload();
        let row = rows::row_for_key(&self.state.borrow(), key);
        let Some(action) = row.and_then(|r| r.actions.into_iter().next()) else {
            eprintln!("backslash: nothing to open for `{key}`");
            return;
        };
        // Apps open directly; everything else shows in the launcher.
        if !matches!(action.op, Op::LaunchApp(..)) {
            // A shortcut opens its item from the root search, not on top of
            // where the user left the launcher.
            if !self.is_root() {
                self.reset_to_root();
            }
            if !self.window.is_visible() {
                self.show();
            }
        }
        self.execute(action);
    }

    // Visibility

    fn toggle(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    fn show(self: &Rc<Self>) {
        self.reload();
        let expired = {
            let s = self.state.borrow();
            match s.config.reopen {
                Reopen::Restore => false,
                Reopen::Minute => s.hidden_at.is_some_and(|t| t.elapsed() > Duration::from_secs(60)),
                Reopen::Root => true,
            }
        };
        if expired {
            self.reset_to_root();
        }
        // config.toml may have been edited by hand.
        self.bind_shortcuts();
        if self.dev_dir.is_none() && self.is_root() && !welcome::seen() {
            welcome::mark_seen();
            self.push(FrameKind::Builtin(Builtin::Welcome));
        }
        self.apply_style();
        if self.layer_shell {
            let height = WidgetExt::display(&self.window)
                .monitors()
                .item(0)
                .and_downcast::<gdk::Monitor>()
                .map_or(1080, |m| m.geometry().height());
            layer::set_top_margin(&self.window, height / 6);
        }
        // Where a paste will go: noted before the launcher takes the focus.
        let target = {
            let s = self.state.borrow();
            s.paste.and_then(|_| paste::focused_app(s.window_backend))
        };
        self.state.borrow_mut().paste_target = target;
        self.render();
        self.window.present();
        self.entry.grab_focus();
        // Text kept from last time is selected: typing replaces it.
        self.entry.select_region(0, -1);
        // Where the clipboard can't be watched in the background (GNOME),
        // at least record what is in it now.
        if self.state.borrow().config.clipboard.enabled {
            let weak = Rc::downgrade(self);
            clipboard::read_now(&WidgetExt::display(&self.window), move |text| {
                if let Some(l) = weak.upgrade() {
                    l.on_clipboard_text(text);
                }
            });
        }
    }

    fn hide(self: &Rc<Self>) {
        self.close_action_panel();
        self.window.set_visible(false);
        let reopen = {
            let mut s = self.state.borrow_mut();
            s.confirm = None;
            s.hidden_at = Some(std::time::Instant::now());
            s.config.reopen
        };
        // Otherwise everything stays as it is (the open command keeps
        // running) and comes back on the next show.
        if reopen == Reopen::Root {
            self.reset_to_root();
        }
    }

    /// Back to the root search; dropping the frames stops extension processes.
    fn reset_to_root(self: &Rc<Self>) {
        {
            let mut s = self.state.borrow_mut();
            s.frames.truncate(1);
            s.confirm = None;
            s.search = None;
            s.filter = Filter::All;
            s.selected_key = None;
            s.frames[0].query.clear();
            s.files.cancel.store(true, Ordering::Relaxed);
        }
        self.set_query("");
        self.render();
    }

    /// Picks up newly installed or edited extensions and config changes.
    fn reload(&self) {
        let mut s = self.state.borrow_mut();
        s.config = Config::load();
        let mut registry = Registry::scan(&paths::extension_dirs());
        if let Some(dir) = &self.dev_dir {
            registry.add_dir(dir);
        }
        for (dir, err) in &registry.errors {
            eprintln!("backslash: skipping {}: {err}", dir.display());
        }
        let config = s.config.clone();
        s.commands = registry.commands(|id| config.is_enabled(id));
        s.registry = registry;
        s.autostart = autostart::enabled();
        s.scripts = scripts::discover(&[scripts::dir()]);
    }

    /// Theme, accent and density, on the launcher and the settings window.
    pub(super) fn apply_style(&self) {
        let config = self.state.borrow().config.clone();
        let mut windows: Vec<gtk::Widget> = vec![self.window.clone().upcast()];
        if let Some(w) = self.settings_window.borrow().as_ref() {
            windows.push(w.clone().upcast());
        }
        let refs: Vec<&gtk::Widget> = windows.iter().collect();
        self.styles.apply(&config, &refs);
    }

    // -----------------------------------------------------------------------
    // Rendering

    fn is_root(&self) -> bool {
        self.state.borrow().frames.len() == 1
    }

    fn render(self: &Rc<Self>) {
        let (depth, title, page) = {
            let s = self.state.borrow();
            let frame = s.frames.last().unwrap();
            let (title, page) = match &frame.kind {
                FrameKind::Root => ("Backslash".to_string(), "list"),
                FrameKind::Builtin(b) => (b.title().to_string(), "list"),
                FrameKind::Extension { view: View::List(_), title, .. } => (title.clone(), "list"),
                // Menus go to the tray and are never pushed; shown empty if one were.
                FrameKind::Extension { view: View::Detail(_) | View::Menu(_), title, .. } => (title.clone(), "detail"),
                FrameKind::Extension { view: View::Form(_), title, .. } => (title.clone(), "form"),
                FrameKind::Form { form, .. } => (form.title.clone().unwrap_or_default(), "form"),
                FrameKind::Approval { .. } => ("Permissions".to_string(), "detail"),
                FrameKind::ConfirmInstall { .. } => ("Extensions".to_string(), "detail"),
                FrameKind::InstallRuntime { .. } => ("Runtime".to_string(), "detail"),
                FrameKind::AppUpdate => ("Update".to_string(), "detail"),
                FrameKind::Script { title, list, .. } => (title.clone(), if *list { "list" } else { "detail" }),
            };
            (s.frames.len(), title, page)
        };
        self.back.set_visible(depth > 1);
        self.search_icon.set_visible(depth == 1);
        self.update_crumb();
        self.context.set_text(&title);
        self.update_chips();
        self.update_filters();
        match page {
            "list" => self.render_list(),
            "detail" => self.render_detail(),
            _ => self.render_form(),
        }
        self.update_footer();
        self.refresh_update_pill();
    }

    /// "Linear › My Issues", or "My Issues › ENG-142" one level deeper.
    fn update_crumb(&self) {
        let s = self.state.borrow();
        let frames = &s.frames;
        let Some(frame) = frames.last() else { return };
        while let Some(child) = self.crumb_icon.first_child() {
            self.crumb_icon.remove(&child);
        }
        let (icon, text) = match &frame.kind {
            FrameKind::Root => return,
            FrameKind::Extension { session, view, title, .. } => {
                let ext = s.registry.find(&session.extension_id).map(|e| &e.manifest.extension);
                let icon = ext.map(|e| e.icon.clone().unwrap_or_else(|| "bs-package".into()));
                let view_title = match view {
                    View::List(l) => l.title.clone(),
                    View::Detail(d) => d.title.clone(),
                    View::Form(f) => f.title.clone(),
                    View::Menu(m) => m.title.clone(),
                };
                let parent = frames.len().checked_sub(2).map(|i| &frames[i].kind);
                let text = match (parent, view_title) {
                    (Some(FrameKind::Extension { session: p, title: pt, .. }), Some(vt))
                        if Rc::ptr_eq(p, session) && vt != *title =>
                    {
                        format!("{pt}  ›  {vt}")
                    }
                    (Some(FrameKind::Extension { session: p, .. }), _) if Rc::ptr_eq(p, session) => title.clone(),
                    _ => format!("{}  ›  {title}", ext.map_or(session.extension_id.as_str(), |e| e.title.as_str())),
                };
                (icon.map(|i| (i, Some(tint_for(&session.extension_id)))), text)
            }
            FrameKind::Builtin(b) => (None, b.title().to_string()),
            FrameKind::Form { form, .. } => (None, form.title.clone().unwrap_or_default()),
            FrameKind::Approval { extension_id, .. } => {
                let title =
                    s.registry.find(extension_id).map_or(extension_id.clone(), |e| e.manifest.extension.title.clone());
                (None, format!("{title}  ›  Permissions"))
            }
            FrameKind::ConfirmInstall { .. } => (None, "Extensions  ›  Install".into()),
            FrameKind::InstallRuntime { .. } => (None, "Runtime".into()),
            FrameKind::AppUpdate => (Some(("bs-download".to_string(), Some("blue"))), "Backslash  ›  Update".into()),
            FrameKind::Script { title, .. } => {
                (Some(("bs-terminal".to_string(), Some("green"))), format!("Scripts  ›  {title}"))
            }
        };
        match icon {
            Some((icon, tint)) => {
                self.crumb_icon.append(&icons::tile(Some(&Icon::Spec(icon)), tint, 18));
                self.crumb_icon.set_visible(true);
            }
            None => {
                self.crumb_icon.append(&crate::glyphs::Glyph::new("bs-back", 14));
                self.crumb_icon.set_visible(true);
            }
        }
        self.crumb_label.set_text(&text);
    }

    /// The chips of an extension list's filter.
    fn update_filters(self: &Rc<Self>) {
        while let Some(child) = self.filters.first_child() {
            self.filters.remove(&child);
        }
        let filter = match &self.state.borrow().frames.last().unwrap().kind {
            FrameKind::Extension { view: View::List(l), .. } => l.filter.clone(),
            _ => None,
        };
        let Some(filter) = filter.filter(|f| f.options.len() > 1) else {
            self.filters.set_visible(false);
            self.filters_divider.set_visible(false);
            return;
        };
        for option in &filter.options {
            let chip = gtk::Button::with_label(&option.title);
            chip.add_css_class("chip");
            chip.set_focusable(false);
            if option.value == filter.value {
                chip.add_css_class("active");
            }
            let weak = Rc::downgrade(self);
            let value = option.value.clone();
            chip.connect_clicked(move |_| {
                if let Some(l) = weak.upgrade() {
                    l.pick_filter(&value);
                }
            });
            self.filters.append(&chip);
        }
        let hint = key_hint("", "Tab");
        hint.set_hexpand(true);
        hint.set_halign(gtk::Align::End);
        self.filters.append(&hint);
        self.filters.set_visible(true);
        self.filters_divider.set_visible(true);
    }

    /// Tab in a list with a filter: the next chip.
    fn cycle_list_filter(self: &Rc<Self>, delta: i32) -> bool {
        let filter = match &self.state.borrow().frames.last().unwrap().kind {
            FrameKind::Extension { view: View::List(l), .. } => l.filter.clone(),
            _ => None,
        };
        let Some(filter) = filter.filter(|f| f.options.len() > 1) else { return false };
        let i = filter.options.iter().position(|o| o.value == filter.value).unwrap_or(0) as i32;
        let next = (i + delta).rem_euclid(filter.options.len() as i32) as usize;
        self.pick_filter(&filter.options[next].value.clone());
        true
    }

    /// Asks the extension for the list with another filter value.
    fn pick_filter(self: &Rc<Self>, value: &str) {
        let (session, view_id) = {
            let s = self.state.borrow();
            match &s.frames.last().unwrap().kind {
                FrameKind::Extension { session, view: View::List(l), .. } => (session.clone(), l.id.clone()),
                _ => return,
            }
        };
        // Show the choice at once; the items follow.
        if let Some(FrameKind::Extension { view: View::List(l), .. }) =
            self.state.borrow_mut().frames.last_mut().map(|f| &mut f.kind)
        {
            if let Some(f) = l.filter.as_mut() {
                f.value = value.to_string();
            }
        }
        self.update_filters();
        let pending = match session.request("filter", json!({ "view": view_id, "value": value })) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };
        self.spinner.set_spinning(true);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            match reply
                .and_then(|v| serde_json::from_value::<View>(v).map_err(|e| RpcError::new(-32700, e.to_string())))
            {
                Ok(view) => {
                    l.state.borrow_mut().selected_key = None;
                    l.replace_top(&session, view);
                }
                Err(e) => l.fail(e),
            }
        });
    }

    fn render_list(self: &Rc<Self>) {
        let query = self.entry.text().to_string();
        self.state.borrow_mut().desktop =
            welcome::Desktop { shortcut: self.shortcuts.status(), layer_shell: self.layer_shell };
        let grid_columns = match &self.state.borrow().frames.last().unwrap().kind {
            FrameKind::Extension { view: View::List(l), .. } if l.layout == proto::Layout::Grid => {
                Some(l.columns.unwrap_or(6).clamp(2, 12))
            }
            _ => None,
        };
        let (rows, placeholder, empty) = {
            let mut guard = self.state.borrow_mut();
            let s = &mut *guard;
            match &s.frames.last().unwrap().kind {
                FrameKind::Root => (rows::root_rows(s, &query), "Search apps, commands, math…".to_string(), None),
                FrameKind::Builtin(b) => {
                    let b = *b;
                    let status = match b {
                        Builtin::Clipboard => Some(format!("{} entries", s.clipboard.entries().len())),
                        Builtin::Windows => Some(format!("{} windows", s.windows.len())),
                        Builtin::Store => {
                            let loc = backslash_core::catalog::Locations::user(&s.config);
                            let installed = s
                                .catalog
                                .entries
                                .iter()
                                .filter(|e| backslash_core::catalog::installed(&loc, &e.id).is_some())
                                .count();
                            Some(if s.catalog.syncing {
                                "Updating the extension list…".to_string()
                            } else if s.catalog.error.is_some() {
                                let age = backslash_core::catalog::age(&loc).map(ago).unwrap_or_default();
                                format!("Couldn't update the list · showing the copy from {age}")
                            } else {
                                format!("{} extensions · {installed} installed", s.catalog.entries.len())
                            })
                        }
                        _ => None,
                    };
                    if let Some(status) = status {
                        self.context.set_text(&status);
                    }
                    let (title, description) = b.empty();
                    let empty =
                        proto::EmptyState { title: title.into(), description: Some(description.into()), icon: None };
                    (rows::builtin_rows(s, b, &query), b.placeholder().to_string(), Some(empty))
                }
                FrameKind::Extension { view: View::List(list), session, .. } => {
                    if s.search.is_none() {
                        self.spinner.set_spinning(list.loading);
                    }
                    if let Some(status) = &list.status {
                        self.context.set_text(status);
                    }
                    let hosts = s
                        .registry
                        .find(&session.extension_id)
                        .map(|e| e.manifest.permissions.net.clone())
                        .unwrap_or_default();
                    let rows = rows::list_rows(&mut s.ranker, list, &query, &hosts);
                    (rows, list.placeholder.clone().unwrap_or_else(|| "Search…".into()), list.empty.clone())
                }
                FrameKind::Script { result, .. } => {
                    self.spinner.set_spinning(result.is_none());
                    let empty = match result {
                        None => proto::EmptyState { title: "Running…".into(), description: None, icon: None },
                        Some(Err(e)) => proto::EmptyState {
                            title: "The script failed".into(),
                            description: Some(e.clone()),
                            icon: None,
                        },
                        Some(Ok(_)) => {
                            proto::EmptyState { title: "It printed nothing".into(), description: None, icon: None }
                        }
                    };
                    (rows::script_lines(s, &query), "Filter…".to_string(), Some(empty))
                }
                _ => unreachable!("render_list on a non-list frame"),
            }
        };
        self.entry.set_editable(true);
        self.entry.set_placeholder_text(Some(&placeholder));

        let previous_key = self.state.borrow().selected_key.clone();
        if let (Some(columns), false) = (grid_columns, rows.is_empty()) {
            let selected = previous_key.and_then(|k| rows.iter().position(|r| r.key == k)).unwrap_or(0);
            self.fill_grid(&rows, columns, selected);
            self.state.borrow_mut().rows = rows;
            self.pages.set_visible_child_name("grid");
            self.update_footer();
            return;
        }
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        *self.sections.borrow_mut() = rows.iter().map(|r| r.section.clone()).collect();
        for row in &rows {
            self.list.append(&row_widget(row));
        }
        let selected = previous_key.and_then(|k| rows.iter().position(|r| r.key == k)).unwrap_or(0);
        let empty_rows = rows.is_empty();
        self.state.borrow_mut().rows = rows;

        if empty_rows {
            let loading = self.spinner.is_spinning();
            let (title, subtitle) = match empty {
                Some(e) if query.is_empty() => (e.title, e.description.unwrap_or_default()),
                _ if loading => ("Loading…".to_string(), String::new()),
                _ if query.is_empty() => ("Nothing here yet".to_string(), String::new()),
                _ => ("No results".to_string(), format!("Nothing matches “{query}”")),
            };
            self.empty_title.set_text(&title);
            self.empty_subtitle.set_text(&subtitle);
            self.pages.set_visible_child_name("empty");
        } else {
            self.pages.set_visible_child_name("list");
            if let Some(row) = self.list.row_at_index(selected as i32) {
                self.list.select_row(Some(&row));
            }
            if selected == 0 {
                self.scroller.vadjustment().set_value(0.0);
            }
        }
        self.update_preview();
    }

    fn is_grid(&self) -> bool {
        self.pages.visible_child_name().as_deref() == Some("grid")
    }

    fn fill_grid(self: &Rc<Self>, rows: &[Row], columns: u32, selected: usize) {
        while let Some(child) = self.grid_box.first_child() {
            self.grid_box.remove(&child);
        }
        let mut cells = Vec::with_capacity(rows.len());
        let mut flow: Option<gtk::FlowBox> = None;
        let mut section: Option<&str> = None;
        for (i, row) in rows.iter().enumerate() {
            if flow.is_none() || row.section.as_deref() != section {
                section = row.section.as_deref();
                if let Some(title) = section {
                    self.grid_box.append(&label(title, "section-header"));
                }
                let f = gtk::FlowBox::builder()
                    .selection_mode(gtk::SelectionMode::None)
                    .homogeneous(true)
                    .min_children_per_line(columns)
                    .max_children_per_line(columns)
                    .column_spacing(6)
                    .row_spacing(6)
                    .build();
                self.grid_box.append(&f);
                flow = Some(f);
            }
            let cell = widgets::grid_cell(row);
            let click = gtk::GestureClick::new();
            let weak = Rc::downgrade(self);
            click.connect_released(move |_, _, _, _| {
                if let Some(l) = weak.upgrade() {
                    l.select_cell(i);
                    l.run_row(i as i32, 0);
                }
            });
            cell.add_controller(click);
            let f = flow.as_ref().expect("created above");
            f.append(&cell);
            if let Some(child) = f.last_child() {
                child.set_focusable(false);
            }
            cells.push(cell.upcast::<gtk::Widget>());
        }
        self.grid_cells.replace(cells);
        self.grid_scroller.vadjustment().set_value(0.0);
        self.select_cell(selected.min(rows.len().saturating_sub(1)));
    }

    fn select_cell(&self, index: usize) {
        let cells = self.grid_cells.borrow();
        if let Some(old) = cells.get(self.grid_selected.get()) {
            old.remove_css_class("selected");
        }
        let Some(cell) = cells.get(index) else { return };
        cell.add_css_class("selected");
        self.grid_selected.set(index);
        let key = self.state.borrow().rows.get(index).map(|r| r.key.clone());
        self.state.borrow_mut().selected_key = key;
        // Keep it visible (after layout, for freshly built grids).
        let (cell, scroller, grid) = (cell.clone(), self.grid_scroller.clone(), self.grid_box.clone());
        glib::idle_add_local_once(move || {
            let Some(b) = cell.compute_bounds(&grid) else { return };
            let adj = scroller.vadjustment();
            let (top, bottom) = (b.y() as f64, (b.y() + b.height()) as f64);
            if top < adj.value() {
                adj.set_value(top - 28.0);
            } else if bottom > adj.value() + adj.page_size() {
                adj.set_value(bottom - adj.page_size() + 6.0);
            }
        });
        drop(cells);
        self.update_footer();
    }

    /// Moves in the grid: `dx` along the line, `dy` lines up or down (to the
    /// tile closest horizontally, across section boundaries).
    fn grid_move(&self, dx: i32, dy: i32) {
        let cells = self.grid_cells.borrow();
        let current = self.grid_selected.get();
        let mut target = (current as i32 + dx).clamp(0, cells.len() as i32 - 1) as usize;
        for _ in 0..dy.abs() {
            let Some(from) = cells.get(target).and_then(|c| c.compute_bounds(&self.grid_box)) else { break };
            let (cx, cy) = (from.x() + from.width() / 2.0, from.y() + from.height() / 2.0);
            let candidates = cells.iter().enumerate().filter_map(|(i, c)| {
                let b = c.compute_bounds(&self.grid_box)?;
                let y = b.y() + b.height() / 2.0;
                let ahead = if dy > 0 { y > cy + 1.0 } else { y < cy - 1.0 };
                ahead.then(|| (i, (y - cy).abs(), (b.x() + b.width() / 2.0 - cx).abs()))
            });
            // Nearest line first, then nearest column.
            let best =
                candidates.min_by(|a, b| (a.1.round() as i64, a.2 as i64).cmp(&(b.1.round() as i64, b.2 as i64)));
            match best {
                Some((i, _, _)) => target = i,
                None => break,
            }
        }
        drop(cells);
        if target != current {
            self.select_cell(target);
        }
    }

    fn update_preview(&self) {
        let Some(row) = self.selected_row() else {
            while let Some(child) = self.preview.first_child() {
                self.preview.remove(&child);
            }
            return;
        };
        widgets::fill_preview(&self.preview, &row.preview());
    }

    fn render_detail(&self) {
        let at_update = matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::AppUpdate));
        let update_detail = at_update.then(|| self.app_update_detail());
        let s = self.state.borrow();
        let approval;
        let d = match s.frames.last().map(|f| &f.kind) {
            Some(FrameKind::AppUpdate) => match &update_detail {
                Some(d) => d,
                None => return,
            },
            Some(FrameKind::Extension { view: View::Detail(d), .. }) => d,
            Some(FrameKind::ConfirmInstall { id }) => {
                let loc = backslash_core::catalog::Locations::user(&s.config);
                let Some(entry) = s.catalog.entries.iter().find(|e| &e.id == id) else { return };
                approval = proto::DetailView {
                    title: Some(entry.title.clone()),
                    markdown: store::confirm_markdown(&loc, entry),
                    ..Default::default()
                };
                &approval
            }
            Some(FrameKind::Script { title, result, .. }) => {
                self.spinner.set_spinning(result.is_none());
                let fence = |t: &str| format!("```\n{}\n```", t.trim_end());
                let markdown = match result {
                    None => "Running…".to_string(),
                    Some(Ok(out)) if out.trim().is_empty() => "_It printed nothing._".to_string(),
                    Some(Ok(out)) => fence(out),
                    Some(Err(e)) => {
                        format!("**{}**", e.lines().next().unwrap_or("Failed"))
                            + &e.split_once('\n').map(|(_, rest)| format!("\n\n{}", fence(rest))).unwrap_or_default()
                    }
                };
                approval = proto::DetailView { title: Some(title.clone()), markdown, ..Default::default() };
                &approval
            }
            Some(FrameKind::InstallRuntime { .. }) => {
                approval = proto::DetailView {
                    title: Some("Deno".into()),
                    markdown: settings::runtime_markdown(s.runtime_busy),
                    ..Default::default()
                };
                &approval
            }
            Some(FrameKind::Approval { extension_id, .. }) => {
                let Some(ext) = s.registry.find(extension_id) else { return };
                approval = proto::DetailView {
                    title: Some(ext.manifest.extension.title.clone()),
                    markdown: settings::approval_markdown(&ext.manifest),
                    ..Default::default()
                };
                &approval
            }
            _ => return,
        };
        if matches!(s.frames.last().map(|f| &f.kind), Some(FrameKind::Extension { .. })) {
            self.spinner.set_spinning(d.loading);
        }
        self.entry.set_editable(false);
        self.entry.set_placeholder_text(d.title.as_deref());
        markdown::set(&self.detail, &d.markdown);
        while let Some(child) = self.metadata.first_child() {
            self.metadata.remove(&child);
        }
        while let Some(child) = self.detail_stats.first_child() {
            self.detail_stats.remove(&child);
        }
        self.detail_stats.set_visible(!d.stats.is_empty());
        if !d.stats.is_empty() {
            self.detail_stats.append(&widgets::stats(&d.stats));
        }
        if let Some(scroll) = self.metadata.parent().and_then(|v| v.parent()) {
            scroll.set_visible(!d.metadata.is_empty());
        }
        if !d.metadata.is_empty() {
            self.metadata.append(&widgets::metadata_grid(&d.metadata, 78));
        }
        if let Some(status) = &d.status {
            self.context.set_text(status);
        }
        self.pages.set_visible_child_name("detail");
    }

    fn render_form(&self) {
        let form = {
            let s = self.state.borrow();
            match &s.frames.last().unwrap().kind {
                FrameKind::Extension { view: View::Form(f), .. } | FrameKind::Form { form: f, .. } => f.clone(),
                _ => return,
            }
        };
        self.entry.set_editable(false);
        self.entry.set_placeholder_text(form.title.as_deref());
        while let Some(child) = self.form_box.first_child() {
            self.form_box.remove(&child);
        }
        let mut inputs = Vec::new();
        for field in &form.fields {
            let title = label(&format!("{}{}", field.title, if field.required { " *" } else { "" }), "field-title");
            let text_value = field.value.as_ref().and_then(Value::as_str).unwrap_or_default();
            let input = match &field.kind {
                FieldKind::Text => {
                    let e = gtk::Entry::new();
                    e.set_text(text_value);
                    e.set_placeholder_text(field.placeholder.as_deref());
                    FormInput::Entry(e)
                }
                FieldKind::Password => {
                    let e = gtk::PasswordEntry::new();
                    e.set_show_peek_icon(true);
                    e.set_text(text_value);
                    FormInput::Password(e)
                }
                FieldKind::TextArea => {
                    let t = gtk::TextView::new();
                    t.set_wrap_mode(gtk::WrapMode::WordChar);
                    t.set_size_request(-1, 90);
                    t.buffer().set_text(text_value);
                    FormInput::Text(t)
                }
                FieldKind::Checkbox => {
                    let c = gtk::CheckButton::with_label(&field.title);
                    c.set_active(field.value.as_ref().and_then(Value::as_bool).unwrap_or(false));
                    FormInput::Check(c)
                }
                FieldKind::Dropdown { options } => {
                    let titles: Vec<&str> = options.iter().map(|o| o.title.as_str()).collect();
                    let d = gtk::DropDown::from_strings(&titles);
                    if let Some(i) = options.iter().position(|o| o.value == text_value) {
                        d.set_selected(i as u32);
                    }
                    FormInput::Drop(d, options.iter().map(|o| o.value.clone()).collect())
                }
            };
            let widget: gtk::Widget = match &input {
                FormInput::Entry(w) => w.clone().upcast(),
                FormInput::Password(w) => w.clone().upcast(),
                FormInput::Text(w) => w.clone().upcast(),
                FormInput::Check(w) => w.clone().upcast(),
                FormInput::Drop(w, _) => w.clone().upcast(),
            };
            if !matches!(input, FormInput::Check(_)) {
                self.form_box.append(&title);
            }
            self.form_box.append(&widget);
            inputs.push((field.clone(), input));
        }
        let first = inputs.first().map(|(_, i)| match i {
            FormInput::Entry(w) => w.clone().upcast::<gtk::Widget>(),
            FormInput::Password(w) => w.clone().upcast(),
            FormInput::Text(w) => w.clone().upcast(),
            FormInput::Check(w) => w.clone().upcast(),
            FormInput::Drop(w, _) => w.clone().upcast(),
        });
        self.state.borrow_mut().form = inputs;
        self.pages.set_visible_child_name("form");
        if let Some(first) = first {
            first.grab_focus();
        }
    }

    fn selected_row(&self) -> Option<Row> {
        let index = if self.is_grid() { self.grid_selected.get() as i32 } else { self.list.selected_row()?.index() };
        self.state.borrow().rows.get(index as usize).cloned()
    }

    /// Actions for the current context: the selected row, or the view itself.
    fn current_actions(&self) -> Vec<UiAction> {
        if matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::AppUpdate)) {
            return self.app_update_actions();
        }
        let view_actions = {
            let s = self.state.borrow();
            let from = |actions: &[proto::Action]| actions.iter().map(UiAction::from_proto).collect::<Vec<_>>();
            match &s.frames.last().unwrap().kind {
                FrameKind::Extension { view: View::Detail(d), .. } => return from(&d.actions),
                FrameKind::Extension { view: View::Form(_) | View::Menu(_), .. } | FrameKind::Form { .. } => {
                    return Vec::new()
                }
                FrameKind::ConfirmInstall { id } => {
                    return vec![
                        UiAction::new("Install", Op::InstallEntry(id.clone())).icon("bs-check"),
                        UiAction::new("Cancel", Op::Back).icon("bs-x"),
                    ]
                }
                FrameKind::Script { id, arguments, result, list, .. } => {
                    let path = s.scripts.iter().find(|x| &x.id == id).map(|x| x.path.display().to_string());
                    let mut actions = Vec::new();
                    if let (false, Some(Ok(out))) = (*list, result) {
                        if s.paste.is_some() {
                            actions.push(
                                UiAction::new("Paste Output", Op::Paste(out.trim_end().to_string())).icon("bs-paste"),
                            );
                        }
                        actions
                            .push(UiAction::new("Copy Output", Op::Copy(out.trim_end().to_string())).icon("bs-copy"));
                    }
                    actions.push(
                        UiAction::new("Run Again", Op::RunScript(id.clone(), Some(arguments.clone())))
                            .icon("bs-refresh")
                            .shortcut("<Ctrl>r"),
                    );
                    if let Some(path) = path {
                        actions.push(UiAction::new("Edit Script", Op::Open(path)).icon("bs-code"));
                    }
                    if *list {
                        let mut row_actions = self.selected_row().map(|r| r.actions).unwrap_or_default();
                        row_actions.extend(actions);
                        return row_actions;
                    }
                    return actions;
                }
                FrameKind::InstallRuntime { .. } => {
                    return vec![
                        UiAction::new("Download and Run", Op::InstallRuntime).icon("bs-download"),
                        UiAction::new("Cancel", Op::Back).icon("bs-x"),
                    ]
                }
                FrameKind::Approval { .. } => {
                    return vec![
                        UiAction::new("Allow and Run", Op::Approve).icon("bs-check"),
                        UiAction::new("Cancel", Op::Back).icon("bs-x"),
                    ]
                }
                FrameKind::Extension { view: View::List(l), .. } => from(&l.actions),
                FrameKind::Builtin(Builtin::Store) => {
                    let mut actions = vec![UiAction::new("Refresh Extensions", Op::RefreshCatalog)
                        .icon("bs-refresh")
                        .shortcut("<Ctrl>r")
                        .section("Extensions")];
                    let lost = store::previous_ids(&s).len();
                    if lost > 0 {
                        actions.push(
                            UiAction::new(format!("Reinstall Previously Installed ({lost})"), Op::ReinstallPrevious)
                                .icon("bs-download")
                                .section("Extensions"),
                        );
                    }
                    actions
                }
                FrameKind::Root | FrameKind::Builtin(_) | FrameKind::AppUpdate => Vec::new(),
            }
        };
        let mut actions = self.selected_row().map(|r| r.actions).unwrap_or_default();
        actions.extend(view_actions);
        actions
    }

    fn is_form(&self) -> bool {
        matches!(
            self.state.borrow().frames.last().unwrap().kind,
            FrameKind::Extension { view: View::Form(_), .. } | FrameKind::Form { .. }
        )
    }

    fn update_footer(&self) {
        if let Some(confirm) = &self.state.borrow().confirm {
            self.primary.add_css_class("confirm");
            self.primary.set_text(&format!(
                "{}   Enter to confirm · Esc to cancel",
                confirm.confirm.as_deref().unwrap_or("Are you sure?")
            ));
            self.primary_key.set_visible(false);
            self.actions_hint.set_visible(false);
            return;
        }
        self.primary.remove_css_class("confirm");
        if self.is_form() {
            let title = match &self.state.borrow().frames.last().unwrap().kind {
                FrameKind::Extension { view: View::Form(f), .. } | FrameKind::Form { form: f, .. } => {
                    f.submit_title.clone()
                }
                _ => None,
            };
            self.primary.set_text(title.as_deref().unwrap_or("Submit"));
            self.primary_key.set_visible(true);
            self.actions_hint.set_visible(false);
            return;
        }
        let actions = self.current_actions();
        self.primary.set_text(actions.first().map_or("", |a| a.title.as_str()));
        self.primary_key.set_visible(!actions.is_empty());
        self.actions_hint.set_visible(actions.len() > 1);
    }

    fn show_toast(&self, text: &str, style: proto::ToastStyle) {
        self.toast.set_text(text);
        for class in ["success", "failure"] {
            self.toast.remove_css_class(class);
        }
        match style {
            proto::ToastStyle::Success => self.toast.add_css_class("success"),
            proto::ToastStyle::Failure => self.toast.add_css_class("failure"),
            proto::ToastStyle::Info => {}
        }
        // The toast takes the place of the context line for a while.
        self.toast.set_visible(true);
        self.context.set_visible(false);
        self.update_pill.set_visible(false);
        // Only the latest toast clears it (removing a timer that already
        // fired would panic, so none is removed).
        let serial = self.state.borrow().toast_serial.clone();
        serial.set(serial.get() + 1);
        let (toast, context, pill, mine) =
            (self.toast.clone(), self.context.clone(), self.update_pill.clone(), serial.get());
        glib::timeout_add_local_once(Duration::from_millis(3500), move || {
            if serial.get() == mine {
                toast.set_text("");
                toast.set_visible(false);
                context.set_visible(true);
                pill.set_visible(pill.has_css_class("available"));
            }
        });
    }

    fn fail(&self, err: impl std::fmt::Display) {
        eprintln!("backslash: {err}");
        self.show_toast(&err.to_string(), proto::ToastStyle::Failure);
    }

    // -----------------------------------------------------------------------
    // Input

    /// Changes the search text without treating it as typing.
    fn set_query(&self, text: &str) {
        self.setting_query.set(true);
        self.entry.set_text(text);
        self.setting_query.set(false);
    }

    fn on_query_changed(self: &Rc<Self>) {
        if self.setting_query.get() {
            return;
        }
        if self.action_panel.is_visible() || !self.entry.is_editable() {
            return;
        }
        self.state.borrow_mut().selected_key = None;
        let files = matches!(self.state.borrow().frames.last().unwrap().kind, FrameKind::Builtin(Builtin::Files));
        if files {
            return self.debounce(150, |l| l.search_files());
        }
        let extension_search = {
            let s = self.state.borrow();
            match &s.frames.last().unwrap().kind {
                FrameKind::Extension { session, view: View::List(l), .. } if l.filtering == Filtering::Extension => {
                    Some((session.clone(), l.id.clone()))
                }
                _ => None,
            }
        };
        match extension_search {
            None => self.render(),
            Some((session, view_id)) => {
                // Debounce, then cancel whatever is still running.
                self.debounce(120, move |l| l.extension_search(session, view_id));
            }
        }
    }

    fn extension_search(self: &Rc<Self>, session: Rc<Session>, view_id: Option<String>) {
        if let Some((old_session, id)) = self.state.borrow_mut().search.take() {
            old_session.cancel(id);
        }
        let query = self.entry.text().to_string();
        let pending = match session.request("search", json!({ "view": view_id, "query": query })) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };
        self.state.borrow_mut().search = Some((session.clone(), pending.id));
        self.spinner.set_spinning(true);
        let weak = Rc::downgrade(self);
        let request_id = pending.id;
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            let current = l.state.borrow().search.as_ref().is_some_and(|(_, id)| *id == request_id);
            if !current {
                return;
            }
            l.state.borrow_mut().search = None;
            l.spinner.set_spinning(false);
            match reply
                .and_then(|v| serde_json::from_value::<View>(v).map_err(|e| RpcError::new(-32700, e.to_string())))
            {
                Ok(view) => l.replace_view(&session, view, true),
                Err(e) if e.code == RpcError::CANCELLED => {}
                Err(e) => l.fail(e),
            }
        });
    }

    fn move_selection(&self, delta: i32) {
        let count = self.state.borrow().rows.len() as i32;
        if count == 0 {
            return;
        }
        let current = self.list.selected_row().map_or(0, |r| r.index());
        let next = (current + delta).clamp(0, count - 1);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
            self.scroll_to(&row);
            let key = self.state.borrow().rows.get(next as usize).map(|r| r.key.clone());
            self.state.borrow_mut().selected_key = key;
        }
    }

    /// Keeps the selected row visible (rows are not focusable, so GTK won't do it).
    fn scroll_to(&self, row: &gtk::ListBoxRow) {
        scroll_into_view(row, &self.list, &self.scroller);
    }

    fn on_key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        if self.action_panel.is_visible() {
            return self.on_popover_key(key, mods);
        }

        if self.state.borrow().confirm.is_some() {
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let action = self.state.borrow_mut().confirm.take();
                    self.update_footer();
                    if let Some(action) = action {
                        self.execute(action);
                    }
                }
                gdk::Key::Escape => {
                    self.state.borrow_mut().confirm = None;
                    self.update_footer();
                }
                _ => {}
            }
            return Stop;
        }

        match key {
            gdk::Key::Tab if self.is_root() => {
                self.cycle_filter(1);
                return Stop;
            }
            gdk::Key::ISO_Left_Tab if self.is_root() => {
                self.cycle_filter(-1);
                return Stop;
            }
            gdk::Key::Tab if !self.is_form() && self.cycle_list_filter(1) => return Stop,
            gdk::Key::ISO_Left_Tab if !self.is_form() && self.cycle_list_filter(-1) => return Stop,
            gdk::Key::Escape => {
                if !self.is_root() {
                    self.pop();
                } else if !self.entry.text().is_empty() {
                    self.entry.set_text("");
                } else if self.state.borrow().filter != Filter::All {
                    self.set_filter(Filter::All);
                } else {
                    self.hide();
                }
                return Stop;
            }
            gdk::Key::BackSpace if self.entry.text().is_empty() && !self.is_root() && !self.is_form() => {
                self.pop();
                return Stop;
            }
            gdk::Key::k | gdk::Key::K if ctrl => {
                self.open_action_panel();
                return Stop;
            }
            gdk::Key::comma if ctrl => {
                self.open_settings(None);
                return Stop;
            }
            _ => {}
        }

        if self.is_form() {
            if matches!(key, gdk::Key::Tab | gdk::Key::ISO_Left_Tab) {
                let back = key == gdk::Key::ISO_Left_Tab || mods.contains(gdk::ModifierType::SHIFT_MASK);
                self.focus_next_field(if back { -1 } else { 1 });
                return Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                let in_textview =
                    gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|w| w.is::<gtk::TextView>());
                if ctrl || !in_textview {
                    self.submit_form();
                    return Stop;
                }
            }
            return Proceed;
        }

        if self.is_grid() {
            let step = match key {
                gdk::Key::Left => Some((-1, 0)),
                gdk::Key::Right => Some((1, 0)),
                gdk::Key::Down => Some((0, 1)),
                gdk::Key::Up => Some((0, -1)),
                gdk::Key::n | gdk::Key::j if ctrl => Some((0, 1)),
                gdk::Key::p if ctrl => Some((0, -1)),
                gdk::Key::Page_Down => Some((0, 3)),
                gdk::Key::Page_Up => Some((0, -3)),
                _ => None,
            };
            if let Some((dx, dy)) = step {
                self.grid_move(dx, dy);
                return Stop;
            }
        }
        match key {
            gdk::Key::Down => self.move_selection(1),
            gdk::Key::Up => self.move_selection(-1),
            gdk::Key::n | gdk::Key::j if ctrl => self.move_selection(1),
            gdk::Key::p if ctrl => self.move_selection(-1),
            gdk::Key::Page_Down => self.move_selection(8),
            gdk::Key::Page_Up => self.move_selection(-8),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let actions = self.current_actions();
                if let Some(action) = actions.get(if ctrl { 1 } else { 0 }).cloned() {
                    self.run_action(action);
                }
            }
            _ => {
                let actions = self.current_actions();
                return match matching_shortcut(&actions, key, mods) {
                    Some(action) => {
                        self.run_action(action);
                        Stop
                    }
                    None => Proceed,
                };
            }
        }
        Stop
    }

    /// Moves the focus between form fields (Tab / Shift+Tab), wrapping around.
    fn focus_next_field(&self, delta: i32) {
        let widgets: Vec<gtk::Widget> = self
            .state
            .borrow()
            .form
            .iter()
            .map(|(_, input)| match input {
                FormInput::Entry(w) => w.clone().upcast(),
                FormInput::Password(w) => w.clone().upcast(),
                FormInput::Text(w) => w.clone().upcast(),
                FormInput::Check(w) => w.clone().upcast(),
                FormInput::Drop(w, _) => w.clone().upcast(),
            })
            .collect();
        if widgets.is_empty() {
            return;
        }
        let focused = gtk::prelude::GtkWindowExt::focus(&self.window);
        let current =
            focused.and_then(|f| widgets.iter().position(|w| &f == w || f.is_ancestor(w))).map_or(-1, |i| i as i32);
        let next = (current + delta).rem_euclid(widgets.len() as i32) as usize;
        widgets[next].grab_focus();
    }

    fn on_popover_key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let actions = self.popover_actions.borrow().clone();
        let selected = self.popover_list.selected_row().map_or(0, |r| r.index());
        let select = |i: i32| {
            if let Some(row) = self.popover_list.row_at_index(i.clamp(0, actions.len() as i32 - 1)) {
                self.popover_list.select_row(Some(&row));
                // The focus stays in the search field, so the scrolled window
                // doesn't follow the selection by itself.
                scroll_into_view(&row, &self.popover_list, &self.popover_scroll);
            }
        };
        match key {
            gdk::Key::Escape => {
                if self.menu_levels.borrow().len() > 1 {
                    self.menu_back();
                } else {
                    self.close_action_panel();
                }
            }
            gdk::Key::BackSpace if self.actions_entry.text().is_empty() && self.menu_levels.borrow().len() > 1 => {
                self.menu_back();
            }
            gdk::Key::Left if self.actions_entry.text().is_empty() && self.menu_levels.borrow().len() > 1 => {
                self.menu_back();
            }
            gdk::Key::k | gdk::Key::K if ctrl => self.close_action_panel(),
            gdk::Key::Down => select(selected + 1),
            gdk::Key::Up => select(selected - 1),
            gdk::Key::n | gdk::Key::j if ctrl => select(selected + 1),
            gdk::Key::p if ctrl => select(selected - 1),
            gdk::Key::Right if self.actions_entry.text().is_empty() => {
                if let Some(action) = actions.get(selected as usize).cloned() {
                    if matches!(action.op, Op::Submenu(_) | Op::LazySubmenu(_)) {
                        self.pick_action(action);
                    }
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(action) = actions.get(selected as usize).cloned() {
                    self.pick_action(action);
                }
            }
            _ => match matching_shortcut(&actions, key, mods) {
                Some(action) => self.pick_action(action),
                None => return Proceed,
            },
        }
        Stop
    }

    /// Runs an action from the panel, or opens its submenu.
    fn pick_action(self: &Rc<Self>, action: UiAction) {
        match &action.op {
            Op::Submenu(children) => self.open_submenu(&action.title, children.clone()),
            Op::LazySubmenu(id) => self.load_submenu(&action.title, id.clone()),
            _ => {
                self.close_action_panel();
                self.run_action(action);
            }
        }
    }

    fn open_action_panel(&self) {
        let actions = self.current_actions();
        if actions.is_empty() {
            return;
        }
        let context = self.selected_row().map(|r| r.title).unwrap_or_default();
        self.menu_levels.replace(vec![(context, actions)]);
        self.show_menu_level();
        self.action_panel.set_visible(true);
        self.actions_entry.grab_focus();
    }

    fn open_submenu(&self, title: &str, actions: Vec<UiAction>) {
        if !self.action_panel.is_visible() {
            // Opened by its shortcut: Escape goes back to all the actions.
            let context = self.selected_row().map(|r| r.title).unwrap_or_default();
            self.menu_levels.replace(vec![(context, self.current_actions())]);
            self.action_panel.set_visible(true);
        }
        self.menu_levels.borrow_mut().push((title.to_string(), actions));
        self.show_menu_level();
        self.actions_entry.grab_focus();
    }

    /// A submenu whose actions the extension gives when asked.
    fn load_submenu(self: &Rc<Self>, title: &str, action_id: String) {
        let Some(session) = self.top_session() else { return };
        self.open_submenu(title, Vec::new());
        self.actions_entry.set_placeholder_text(Some("Loading…"));
        let pending = match session.request("submenu", json!({ "action": action_id })) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };
        let weak = Rc::downgrade(self);
        let title = title.to_string();
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            let actions = reply.and_then(|v| {
                serde_json::from_value::<Vec<proto::Action>>(v).map_err(|e| RpcError::new(-32700, e.to_string()))
            });
            // Still showing that submenu?
            let current = l.menu_levels.borrow().last().is_some_and(|(t, a)| *t == title && a.is_empty());
            match actions {
                Ok(actions) if current => {
                    if let Some(level) = l.menu_levels.borrow_mut().last_mut() {
                        level.1 = actions.iter().map(UiAction::from_proto).collect();
                    }
                    l.show_menu_level();
                }
                Ok(_) => {}
                Err(e) => {
                    l.close_action_panel();
                    l.fail(e);
                }
            }
        });
    }

    fn menu_back(&self) {
        self.menu_levels.borrow_mut().pop();
        self.show_menu_level();
    }

    /// Shows the top menu level, filtered by what is typed in the panel.
    fn show_menu_level(&self) {
        let levels = self.menu_levels.borrow();
        let Some((title, actions)) = levels.last() else { return };
        let path: Vec<&str> = levels.iter().skip(1).map(|(t, _)| t.as_str()).collect();
        let context = if path.is_empty() { title.clone() } else { path.join(" › ") };
        self.actions_context.set_text(&context);
        self.actions_context.set_visible(!context.is_empty());
        let depth = levels.len();
        let actions = actions.clone();
        drop(levels);
        self.setting_query.set(true);
        self.actions_entry.set_text("");
        self.setting_query.set(false);
        self.actions_entry.set_placeholder_text(Some(if depth > 1 { "Search…" } else { "Search actions…" }));
        self.fill_action_list(&actions, "");
    }

    fn fill_action_list(&self, actions: &[UiAction], query: &str) {
        while let Some(child) = self.popover_list.first_child() {
            self.popover_list.remove(&child);
        }
        let q = query.trim().to_lowercase();
        let shown: Vec<UiAction> =
            actions.iter().filter(|a| q.is_empty() || a.title.to_lowercase().contains(&q)).cloned().collect();
        let top_level = self.menu_levels.borrow().len() <= 1;
        let sections: Vec<Option<String>> = shown.iter().map(|a| a.section.clone()).collect();
        for (i, a) in shown.iter().enumerate() {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = a.icon.clone().map(Icon::Spec).or_else(|| Icon::named("bs-dot"));
            let w = icons::widget(icon.as_ref(), 15);
            if a.destructive {
                w.add_css_class("destructive");
            }
            b.append(&w);
            let title = label(&a.title, "title");
            title.set_hexpand(true);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if a.destructive {
                title.add_css_class("destructive");
            }
            b.append(&title);
            if matches!(a.op, Op::Submenu(_) | Op::LazySubmenu(_)) {
                b.append(&crate::glyphs::Glyph::new("bs-chevron-right", 13));
            }
            // Enter and Ctrl+Enter run the first two actions of an item.
            let keys = match (i, &a.shortcut) {
                (_, Some(s)) => Some(widgets::accel_keys(s)),
                (0, None) if top_level && q.is_empty() => Some("↵".to_string()),
                (1, None) if top_level && q.is_empty() => Some("Ctrl ↵".to_string()),
                _ => None,
            };
            if let Some(k) = keys {
                b.append(&key_hint("", &k));
            }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&b));
            row.set_focusable(false);
            self.popover_list.append(&row);
        }
        let sections_for_header = Rc::new(sections);
        self.popover_list.set_header_func(move |row, before| {
            let current = sections_for_header.get(row.index() as usize).cloned().flatten();
            let previous = before.and_then(|b| sections_for_header.get(b.index() as usize).cloned().flatten());
            match current {
                Some(s) if previous.as_ref() != Some(&s) => {
                    let h = label(&s, "section-header");
                    h.set_margin_start(4);
                    row.set_header(Some(&h));
                }
                None if previous.is_some() => {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    line.add_css_class("meta-separator");
                    line.set_margin_top(4);
                    line.set_margin_bottom(4);
                    row.set_header(Some(&line));
                }
                _ => row.set_header(gtk::Widget::NONE),
            }
        });
        *self.popover_actions.borrow_mut() = shown;
        if let Some(first) = self.popover_list.row_at_index(0) {
            self.popover_list.select_row(Some(&first));
        }
        self.popover_scroll.vadjustment().set_value(0.0);
    }

    fn on_actions_query_changed(&self) {
        if self.setting_query.get() {
            return;
        }
        let actions = self.menu_levels.borrow().last().map(|(_, a)| a.clone()).unwrap_or_default();
        self.fill_action_list(&actions, &self.actions_entry.text());
    }

    fn close_action_panel(&self) {
        self.action_panel.set_visible(false);
        self.menu_levels.replace(Vec::new());
        self.entry.grab_focus_without_selecting();
    }

    // -----------------------------------------------------------------------
    // Actions

    fn run_row(self: &Rc<Self>, index: i32, action: usize) {
        let row = self.state.borrow().rows.get(index as usize).cloned();
        if let Some(a) = row.and_then(|r| r.actions.get(action).cloned()) {
            self.run_action(a);
        }
    }

    fn run_action(self: &Rc<Self>, action: UiAction) {
        if action.confirm.is_some() {
            self.state.borrow_mut().confirm = Some(action);
            self.update_footer();
            return;
        }
        self.execute(action);
    }

    fn record(&self, id: String) {
        let query = self.entry.text().to_string();
        self.state.borrow_mut().usage.record(&id, &query);
    }

    fn execute(self: &Rc<Self>, action: UiAction) {
        match action.op {
            Op::OpenSettings => self.open_settings(None),
            Op::Submenu(children) => self.open_submenu(&action.title, children),
            Op::LazySubmenu(id) => self.load_submenu(&action.title, id),
            Op::LaunchApp(id, desktop_action) => {
                let app = self.state.borrow().apps.iter().find(|a| a.id == id).cloned();
                let Some(app) = app else { return };
                self.record(format!("app:{id}"));
                match apps::launch(&app, desktop_action.as_deref()) {
                    Ok(()) => self.hide(),
                    Err(e) => self.fail(format!("Could not open {}: {e}", app.name)),
                }
            }
            Op::HideApp(id) => {
                let mut s = self.state.borrow_mut();
                s.config.hidden_apps.push(id.clone());
                if let Err(e) = s.config.save() {
                    drop(s);
                    return self.fail(e);
                }
                s.apps.retain(|a| a.id != id);
                drop(s);
                self.show_toast(
                    "Hidden. Remove it from hidden_apps in config.toml to restore.",
                    proto::ToastStyle::Info,
                );
                self.render();
            }
            Op::StartCommand(id) => {
                self.record(format!("cmd:{id}"));
                self.start_command(&id, None);
            }
            Op::Copy(text) => {
                self.window.clipboard().set_text(&text);
                self.flash_and_hide("Copied to clipboard");
            }
            Op::CopyConcealed(text) => {
                set_clipboard(&self.window, &text, true);
                self.flash_and_hide("Copied to clipboard");
            }
            Op::PasteConcealed(text) => self.paste(text, true),
            Op::Open(target) => match apps::open(&target) {
                Ok(()) => self.hide(),
                Err(e) => self.fail(e),
            },
            Op::Callback(id) => self.callback(&id, None),
            Op::Snippet(text, paste) => self.copy_snippet(text, paste),
            Op::Paste(text) => self.paste_text(text),
            Op::FillQuery(text) => {
                self.entry.set_text(&text);
                self.entry.set_position(-1);
            }
            Op::OpenBuiltin(builtin) => self.open_builtin(builtin),
            Op::ClipCopy(id) => {
                let text = self.state.borrow().clipboard.get(id).map(|e| e.text.clone());
                if let Some(text) = text {
                    self.window.clipboard().set_text(&text);
                    self.flash_and_hide("Copied to clipboard");
                }
            }
            Op::ClipPin(id) => {
                self.state.borrow_mut().clipboard.toggle_pin(id);
                self.render();
            }
            Op::ClipDelete(id) => {
                self.state.borrow_mut().clipboard.delete(id);
                self.state.borrow_mut().selected_key = None;
                self.render();
            }
            Op::ClipClear => {
                self.state.borrow_mut().clipboard.clear();
                self.render();
            }
            Op::Reveal(path) => {
                reveal(&path);
                self.hide();
            }
            Op::FocusWindow(id) if self.state.borrow().window_backend.is_none() => {
                self.hide();
                if let Some(kwin) = self.kwin.clone() {
                    glib::spawn_future_local(async move {
                        if let Err(e) = kwin.activate(&id).await {
                            eprintln!("backslash: could not switch window: {e}");
                        }
                    });
                }
            }
            Op::InstallGnomeHelper => match backslash_core::system::gnome::install() {
                Ok(()) => {
                    self.show_toast(
                        "Installed. Log out and back in once to switch windows",
                        proto::ToastStyle::Success,
                    );
                    self.render();
                }
                Err(e) => self.fail(format!("Could not install the GNOME helper: {e:#}")),
            },
            Op::FocusWindow(id) => {
                let backend = self.state.borrow().window_backend;
                self.hide();
                if let Some(backend) = backend {
                    // Let the launcher give the focus back first.
                    glib::timeout_add_local_once(Duration::from_millis(60), move || {
                        if let Err(e) = backend.focus(&id) {
                            eprintln!("backslash: {e:#}");
                        }
                    });
                }
            }
            Op::SetTheme(theme) => {
                {
                    let mut s = self.state.borrow_mut();
                    s.config.theme = theme;
                    if let Err(e) = s.config.save() {
                        drop(s);
                        return self.fail(e);
                    }
                }
                self.apply_style();
                self.pop();
            }
            Op::SetAutostart(enabled) => match autostart::set(enabled) {
                Ok(()) => {
                    self.state.borrow_mut().autostart = enabled;
                    self.show_toast(
                        if enabled { "Backslash will start with your session" } else { "Launch at login disabled" },
                        proto::ToastStyle::Success,
                    );
                    self.render();
                }
                Err(e) => self.fail(e),
            },
            Op::SetClipboard(enabled) => {
                self.update_config(|c| c.clipboard.enabled = enabled);
                let note = if enabled {
                    "Clipboard history on (watching starts at the next start)"
                } else {
                    "Clipboard history off"
                };
                self.show_toast(note, proto::ToastStyle::Success);
                self.render();
            }
            Op::EditHotkey => {
                let current = self.state.borrow().config.hotkey.clone();
                self.push(FrameKind::Form { purpose: FormPurpose::Hotkey, form: settings::hotkey_form(&current) });
            }
            Op::EditAlias(key) => {
                let form = {
                    let s = self.state.borrow();
                    let title = rows::row_for_key(&s, &key).map_or_else(|| key.clone(), |r| r.title);
                    settings::alias_form(&title, s.config.aliases.get(&key).map_or("", String::as_str))
                };
                self.push(FrameKind::Form { purpose: FormPurpose::Alias { key }, form });
            }
            Op::EditKeyHotkey(key) => {
                let form = {
                    let s = self.state.borrow();
                    let title = rows::row_for_key(&s, &key).map_or_else(|| key.clone(), |r| r.title);
                    settings::key_hotkey_form(&title, s.config.hotkeys.get(&key).map_or("", String::as_str))
                };
                self.push(FrameKind::Form { purpose: FormPurpose::KeyHotkey { key }, form });
            }
            Op::UnhideApp(id) => {
                self.update_config(|c| c.hidden_apps.retain(|h| *h != id));
                let hidden = self.state.borrow().config.hidden_apps.clone();
                self.state.borrow_mut().apps = apps::load(&hidden);
                self.render();
            }
            Op::Approve => {
                let pending = match &self.state.borrow().frames.last().unwrap().kind {
                    FrameKind::Approval { extension_id, command_id, arguments } => {
                        Some((extension_id.clone(), command_id.clone(), arguments.clone()))
                    }
                    _ => None,
                };
                let Some((extension_id, command_id, arguments)) = pending else { return };
                let approved = {
                    let s = self.state.borrow();
                    s.registry.find(&extension_id).map(|e| approvals::approve(&e.manifest))
                };
                match approved {
                    Some(Ok(())) => {
                        self.pop();
                        self.start_command(&command_id, arguments);
                    }
                    Some(Err(e)) => self.fail(format!("{e:#}")),
                    None => {}
                }
            }
            Op::Back => self.pop(),
            Op::ReviewInstall(id) => self.push(FrameKind::ConfirmInstall { id }),
            Op::InstallEntry(id) => self.install_entry(id),
            Op::UpdateAll => self.update_all(),
            Op::RefreshCatalog => self.refresh_catalog(true, true),
            Op::ReinstallPrevious => {
                let ids = store::previous_ids(&self.state.borrow());
                self.install_entries(ids);
            }
            Op::Spawn(program, args) => {
                if let Err(e) = std::process::Command::new(&program).args(&args).spawn() {
                    self.fail(format!("Could not start {program}: {e}"));
                }
            }
            Op::RunScript(id, arguments) => self.run_script(&id, arguments),
            Op::NewScript => self.push(FrameKind::Form { purpose: FormPurpose::NewScript, form: new_script_form() }),
            Op::OpenScriptsFolder => {
                let dir = scripts::dir();
                let _ = std::fs::create_dir_all(&dir);
                match apps::open(&dir.display().to_string()) {
                    Ok(()) => self.hide(),
                    Err(e) => self.fail(e),
                }
            }
            Op::FinishWelcome => {
                welcome::mark_seen();
                {
                    let mut s = self.state.borrow_mut();
                    s.frames.truncate(1);
                    s.frames[0].query.clear();
                    s.selected_key = None;
                }
                self.set_query("");
                self.render();
                self.show_toast("You're all set. Type to search", proto::ToastStyle::Success);
            }
            Op::InstallRuntime => self.install_runtime(),
            Op::OpenAppUpdate => self.open_app_update(),
            Op::CheckAppUpdate => self.check_app_update(true),
            Op::InstallAppUpdate => self.install_app_update(),
            Op::Quit => self.quit(),
            Op::UninstallEntry(id) => {
                let loc = backslash_core::catalog::Locations::user(&self.state.borrow().config);
                match backslash_core::catalog::uninstall(&loc, &id) {
                    Ok(()) => {
                        self.reload();
                        self.recount_updates();
                        self.show_toast("Uninstalled", proto::ToastStyle::Success);
                        self.render();
                    }
                    Err(e) => self.fail(format!("{e:#}")),
                }
            }
        }
    }

    fn flash_and_hide(self: &Rc<Self>, message: &str) {
        self.show_toast(message, proto::ToastStyle::Success);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(450), move || {
            if let Some(l) = weak.upgrade() {
                l.hide();
            }
        });
    }

    /// Starts `extension/command`. Before running, the launcher may first ask
    /// to approve the extension's permissions, fill missing required
    /// preferences, or fill the command's arguments.
    fn start_command(self: &Rc<Self>, id: &str, arguments: Option<Map<String, Value>>) {
        enum Prepared {
            Started((Session, backslash_core::host::Pending), String, Map<String, Value>),
            Push(FrameKind),
            Failed(String),
            /// A tray menu: refreshed rather than opened.
            Tray,
        }
        let prepared = {
            let s = self.state.borrow();
            match s.registry.resolve(id) {
                None => Prepared::Failed(format!("Unknown command {id}")),
                Some((ext, command)) => {
                    let manifest = &ext.manifest;
                    let ext_id = manifest.extension.id.clone();
                    let (_, missing) = s.config.preferences(manifest);
                    if manifest.runtime == Runtime::Deno && !runtime::is_installed() {
                        Prepared::Push(FrameKind::InstallRuntime { command_id: id.into(), arguments })
                    } else if !approvals::requested(manifest).is_empty() && !approvals::is_approved(manifest) {
                        Prepared::Push(FrameKind::Approval { extension_id: ext_id, command_id: id.into(), arguments })
                    } else if !missing.is_empty() {
                        Prepared::Push(FrameKind::Form {
                            purpose: FormPurpose::Preferences { extension_id: ext_id, then_run: Some(id.into()) },
                            form: settings::preferences_form(manifest, &s.config),
                        })
                    } else if command.mode == backslash_core::manifest::CommandMode::MenuBar {
                        Prepared::Tray
                    } else if arguments.is_none() && !command.arguments.is_empty() {
                        Prepared::Push(FrameKind::Form {
                            purpose: FormPurpose::Arguments { command_id: id.into() },
                            form: arguments_form(command),
                        })
                    } else {
                        let arguments = arguments.unwrap_or_default();
                        match runner::start(ext, command, &s.config, arguments.clone(), "", LaunchType::UserInitiated) {
                            Ok(started) => Prepared::Started(started, command.title.clone(), arguments),
                            Err(e) => Prepared::Failed(format!("{e:#}")),
                        }
                    }
                }
            }
        };
        let ((session, pending), title, arguments) = match prepared {
            Prepared::Started(started, title, arguments) => (started, title, arguments),
            Prepared::Failed(e) => return self.fail(e),
            Prepared::Push(kind) => return self.push(kind),
            Prepared::Tray => {
                self.run_in_background(id);
                return self.flash_and_hide("Shown in the system tray");
            }
        };

        let session = Rc::new(session);
        let launched = Rc::new(Launched { command_id: id.to_string(), arguments });
        self.listen(&session);
        self.spinner.set_spinning(true);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            match reply
                .and_then(|v| serde_json::from_value::<Effect>(v).map_err(|e| RpcError::new(-32700, e.to_string())))
            {
                Ok(Effect::Push { view } | Effect::Replace { view }) => {
                    l.push(FrameKind::Extension { session, view, title, launched });
                }
                Ok(Effect::Close { message }) => match message {
                    Some(m) => l.flash_and_hide(&m),
                    None => l.hide(),
                },
                Ok(_) => {}
                Err(e) => l.fail(format!("{title}: {e}")),
            }
        });
    }

    fn top_session(&self) -> Option<Rc<Session>> {
        match &self.state.borrow().frames.last()?.kind {
            FrameKind::Extension { session, .. } => Some(session.clone()),
            _ => None,
        }
    }

    fn callback(self: &Rc<Self>, action_id: &str, values: Option<Map<String, Value>>) {
        let Some(session) = self.top_session() else { return };
        let pending = match session.request("action", json!({ "action": action_id, "values": values })) {
            Ok(p) => p,
            Err(e) => return self.fail(e),
        };
        self.spinner.set_spinning(true);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            match reply
                .and_then(|v| serde_json::from_value::<Effect>(v).map_err(|e| RpcError::new(-32700, e.to_string())))
            {
                Ok(effect) => l.apply(&session, effect),
                Err(e) => l.fail(e),
            }
        });
    }

    fn apply(self: &Rc<Self>, session: &Rc<Session>, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::Push { view } => {
                let title = self.context.text().to_string();
                let launched = match self.state.borrow().frames.last().map(|f| &f.kind) {
                    Some(FrameKind::Extension { launched, .. }) => launched.clone(),
                    _ => return,
                };
                self.push(FrameKind::Extension { session: session.clone(), view, title, launched });
            }
            Effect::Replace { view } => self.replace_top(session, view),
            Effect::Pop => self.pop(),
            Effect::Close { message } => match message {
                Some(m) => self.flash_and_hide(&m),
                None => self.hide(),
            },
        }
    }

    fn submit_form(self: &Rc<Self>) {
        let mut values = Map::new();
        let mut missing = None;
        for (field, input) in &self.state.borrow().form {
            let value: Value = match input {
                FormInput::Entry(e) => e.text().to_string().into(),
                FormInput::Password(e) => e.text().to_string().into(),
                FormInput::Text(t) => {
                    let b = t.buffer();
                    b.text(&b.start_iter(), &b.end_iter(), false).to_string().into()
                }
                FormInput::Check(c) => c.is_active().into(),
                FormInput::Drop(d, opts) => opts.get(d.selected() as usize).cloned().unwrap_or_default().into(),
            };
            if field.required && missing.is_none() && value.as_str().is_some_and(|v| v.trim().is_empty()) {
                missing = Some(field.title.clone());
            }
            values.insert(field.id.clone(), value);
        }
        if let Some(title) = missing {
            return self.fail(format!("{title} is required"));
        }
        let purpose = match &self.state.borrow().frames.last().unwrap().kind {
            FrameKind::Form { purpose, .. } => Some(purpose.clone()),
            FrameKind::Extension { view: View::Form(f), .. } => {
                let id = format!("{}:submit", f.id.clone().unwrap_or_default());
                return self.callback(&id, Some(values));
            }
            _ => None,
        };
        let Some(purpose) = purpose else { return };
        let text = |k: &str| values.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_string();
        match purpose {
            FormPurpose::Arguments { command_id } => {
                let parsed = {
                    let s = self.state.borrow();
                    let Some((_, command)) = s.registry.resolve(&command_id) else { return };
                    runner::parse_arguments(command, &values)
                };
                match parsed {
                    Ok(args) => {
                        self.pop();
                        self.start_command(&command_id, Some(args));
                    }
                    Err(e) => self.fail(e),
                }
            }
            FormPurpose::ScriptArguments { id } => {
                let names: Vec<String> = {
                    let s = self.state.borrow();
                    let Some(script) = s.scripts.iter().find(|x| x.id == id) else { return };
                    script.arguments.iter().map(|a| a.name.clone()).collect()
                };
                let args = names.iter().map(|n| text(n)).collect();
                self.pop();
                self.run_script(&id, Some(args));
            }
            FormPurpose::NewScript => {
                let title = text("title");
                let mode = match text("mode").as_str() {
                    "output" => ScriptMode::Output,
                    "list" => ScriptMode::List,
                    "paste" => ScriptMode::Paste,
                    _ => ScriptMode::Silent,
                };
                match scripts::create(&scripts::dir(), &title, mode) {
                    Ok(path) => {
                        self.pop();
                        self.reload();
                        let _ = apps::open(&path.display().to_string());
                        self.show_toast(
                            &format!("Created {}: edit it, then search “{title}”", files::tilde(&path)),
                            proto::ToastStyle::Success,
                        );
                        self.render();
                    }
                    Err(e) => self.fail(format!("Could not create the script: {e}")),
                }
            }
            FormPurpose::Preferences { extension_id, then_run } => {
                if let Err(e) = self.save_preferences(&extension_id, &values) {
                    return self.fail(format!("{e:#}"));
                }
                self.pop();
                match then_run {
                    Some(command) => self.start_command(&command, None),
                    None => self.show_toast("Preferences saved", proto::ToastStyle::Success),
                }
            }
            FormPurpose::Hotkey => {
                let hotkey = text("hotkey");
                self.update_config(|c| c.hotkey = hotkey);
                self.pop();
                self.bind_shortcuts();
                self.show_toast("Saved. Confirm the shortcut if your desktop asks", proto::ToastStyle::Info);
            }
            FormPurpose::Alias { key } => {
                let alias = text("alias").trim().to_string();
                let taken = {
                    let s = self.state.borrow();
                    s.config.alias_target(&alias).filter(|k| *k != key).map(str::to_owned)
                };
                if let Some(other) = taken {
                    return self.fail(format!("`{alias}` is already the alias of {other}"));
                }
                self.update_config(|c| {
                    if alias.is_empty() {
                        c.aliases.remove(&key);
                    } else {
                        c.aliases.insert(key, alias);
                    }
                });
                self.pop();
                self.show_toast("Alias saved", proto::ToastStyle::Success);
            }
            FormPurpose::KeyHotkey { key } => {
                let hotkey = text("hotkey").trim().to_string();
                self.update_config(|c| {
                    if hotkey.is_empty() {
                        c.hotkeys.remove(&key);
                    } else {
                        c.hotkeys.insert(key, hotkey);
                    }
                });
                self.pop();
                self.bind_shortcuts();
                self.show_toast("Saved. Confirm the shortcut if your desktop asks", proto::ToastStyle::Info);
            }
        }
    }

    /// Plain preferences go to config.toml, secrets to the keyring. An empty
    /// secret field keeps the saved value.
    fn save_preferences(&self, extension_id: &str, values: &Map<String, Value>) -> anyhow::Result<()> {
        let mut s = self.state.borrow_mut();
        let manifest = s
            .registry
            .find(extension_id)
            .map(|e| e.manifest.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown extension {extension_id}"))?;
        let entry = s.config.extensions.entry(extension_id.to_string()).or_default();
        for pref in &manifest.preferences {
            let value = values.get(&pref.name).cloned().unwrap_or(Value::Null);
            match pref.kind {
                backslash_core::manifest::PreferenceKind::Secret => {
                    let secret = value.as_str().unwrap_or_default();
                    if !secret.is_empty() {
                        secrets::set(extension_id, &pref.name, secret)?;
                    }
                }
                backslash_core::manifest::PreferenceKind::Boolean => {
                    entry.preferences.insert(
                        pref.name.clone(),
                        backslash_core::toml::Value::Boolean(value.as_bool().unwrap_or(false)),
                    );
                }
                backslash_core::manifest::PreferenceKind::Number => {
                    let raw = value.as_str().unwrap_or_default().trim().replace(',', ".");
                    if raw.is_empty() {
                        entry.preferences.remove(&pref.name);
                    } else {
                        let n: f64 = raw.parse().map_err(|_| anyhow::anyhow!("{} must be a number", pref.title))?;
                        entry.preferences.insert(pref.name.clone(), backslash_core::toml::Value::Float(n));
                    }
                }
                backslash_core::manifest::PreferenceKind::Text => {
                    let text = value.as_str().unwrap_or_default().to_string();
                    if text.is_empty() {
                        entry.preferences.remove(&pref.name);
                    } else {
                        entry.preferences.insert(pref.name.clone(), backslash_core::toml::Value::String(text));
                    }
                }
            }
        }
        s.config.save()
    }

    /// Changes the config, saves it, and re-renders.
    fn update_config(self: &Rc<Self>, change: impl FnOnce(&mut Config)) {
        let result = {
            let mut s = self.state.borrow_mut();
            change(&mut s.config);
            let config = s.config.clone();
            let commands = s.registry.commands(|id| config.is_enabled(id));
            s.commands = commands;
            s.config.save()
        };
        if let Err(e) = result {
            self.fail(format!("Could not save config.toml: {e:#}"));
        }
    }

    // -----------------------------------------------------------------------
    // Navigation

    fn push(self: &Rc<Self>, kind: FrameKind) {
        let selected = self.selected_row().map(|r| r.key);
        {
            let mut s = self.state.borrow_mut();
            let query = self.entry.text().to_string();
            let top = s.frames.last_mut().unwrap();
            top.query = query;
            top.selected = selected;
            s.frames.push(Frame { kind, query: String::new(), selected: None });
            s.confirm = None;
            s.selected_key = None;
            // An update may have come before the view it updates.
            let early = match &s.frames.last().unwrap().kind {
                FrameKind::Extension { session, view, .. } => {
                    let id = view_id(view);
                    s.early_updates
                        .iter()
                        .rposition(|(es, v)| Rc::ptr_eq(es, session) && view_id(v) == id)
                        .map(|i| s.early_updates.swap_remove(i).1)
                }
                _ => None,
            };
            s.early_updates.clear();
            if let (Some(update), Some(Frame { kind: FrameKind::Extension { view, .. }, .. })) =
                (early, s.frames.last_mut())
            {
                *view = update;
            }
        }
        self.set_query("");
        self.render();
        // Forms focus their first field themselves.
        if !self.is_form() {
            self.entry.grab_focus();
        }
    }

    fn pop(self: &Rc<Self>) {
        let query = {
            let mut s = self.state.borrow_mut();
            if s.frames.len() <= 1 {
                return;
            }
            s.frames.pop();
            s.confirm = None;
            s.search = None;
            s.selected_key = s.frames.last().unwrap().selected.clone();
            s.frames.last().unwrap().query.clone()
        };
        self.entry.set_editable(true);
        self.set_query(&query);
        self.render();
        self.entry.grab_focus_without_selecting();
        self.entry.set_position(-1);
    }

    /// `replace()` / `refresh()` from an action: swaps the current screen, even
    /// though the re-rendered view has a new id. Keeps the selected item.
    fn replace_top(self: &Rc<Self>, session: &Rc<Session>, view: View) {
        let selected_key = self.selected_row().map(|r| r.key);
        {
            let mut s = self.state.borrow_mut();
            let Some(Frame { kind: FrameKind::Extension { session: fs, view: v, .. }, .. }) = s.frames.last_mut()
            else {
                return;
            };
            if !Rc::ptr_eq(fs, session) {
                return;
            }
            *v = view;
            s.selected_key = selected_key;
        }
        self.render();
    }

    /// Updates the frame showing `view.id` from `session` (streaming updates,
    /// search results). `keep_selection` keeps the highlighted item if it still exists.
    fn replace_view(self: &Rc<Self>, session: &Rc<Session>, view: View, keep_selection: bool) {
        let view_id = view_id(&view);
        let selected_key = keep_selection.then(|| self.selected_row().map(|r| r.key)).flatten();
        let is_top = {
            let mut s = self.state.borrow_mut();
            let len = s.frames.len();
            let position = s.frames.iter().rposition(|f| match &f.kind {
                FrameKind::Extension { session: fs, view: fv, .. } => {
                    Rc::ptr_eq(fs, session) && (view_id.is_none() || view_id_ref(fv) == view_id.as_deref())
                }
                _ => false,
            });
            let Some(i) = position else {
                // Its view is not shown yet (the start reply is on its way).
                s.early_updates.push((session.clone(), view));
                return;
            };
            if let FrameKind::Extension { view: v, .. } = &mut s.frames[i].kind {
                *v = view;
            }
            s.selected_key = selected_key;
            i == len - 1
        };
        if is_top {
            self.render();
        }
    }

    fn listen(self: &Rc<Self>, session: &Rc<Session>) {
        let weak = Rc::downgrade(self);
        let session_weak: Weak<Session> = Rc::downgrade(session);
        let incoming = session.incoming.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = incoming.recv().await {
                let (Some(l), Some(session)) = (weak.upgrade(), session_weak.upgrade()) else { break };
                l.handle_incoming(&session, msg);
            }
        });
    }

    fn handle_incoming(self: &Rc<Self>, session: &Rc<Session>, msg: Incoming) {
        match msg {
            Incoming::Log(line) => eprintln!("[{}] {line}", session.extension_id),
            Incoming::Exited(_) => {}
            Incoming::Notification { method, params } if method == "view/update" => {
                match serde_json::from_value::<View>(params["view"].clone()) {
                    Ok(view) => {
                        if let View::List(l) = &view {
                            self.spinner.set_spinning(l.loading);
                        }
                        self.replace_view(session, view, true)
                    }
                    Err(e) => self.fail(format!("{}: invalid view: {e}", session.extension_id)),
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let result: Result<Value, RpcError> = match method.as_str() {
                    "clipboard/write" => {
                        let concealed = params.get("concealed").and_then(Value::as_bool).unwrap_or(false);
                        set_clipboard(&self.window, &text("text"), concealed);
                        Ok(Value::Null)
                    }
                    "clipboard/paste" => {
                        // Answer first: pasting hides the window.
                        session.respond(id, Ok(Value::Null));
                        let concealed = params.get("concealed").and_then(Value::as_bool).unwrap_or(false);
                        self.paste(text("text"), concealed);
                        return;
                    }
                    "clipboard/read" => {
                        let allowed = self
                            .state
                            .borrow()
                            .registry
                            .find(&session.extension_id)
                            .is_some_and(|e| e.manifest.permissions.clipboard_read);
                        if !allowed {
                            Err(RpcError::new(-32001, "clipboard-read permission not declared"))
                        } else {
                            let clipboard = self.window.clipboard();
                            let session = session.clone();
                            glib::spawn_future_local(async move {
                                let text = clipboard.read_text_future().await.ok().flatten();
                                session.respond(id, Ok(text.map(|t| t.to_string()).into()));
                            });
                            return;
                        }
                    }
                    "open" => apps::open(&text("target")).map(|_| Value::Null).map_err(|e| RpcError::new(-32000, e)),
                    "toast" => {
                        let style = serde_json::from_value(params["style"].clone()).unwrap_or_default();
                        let message = [text("title"), text("message")].join("  ").trim().to_string();
                        self.show_toast(&message, style);
                        Ok(Value::Null)
                    }
                    "close" => {
                        self.hide();
                        Ok(Value::Null)
                    }
                    "oauth/token" | "oauth/signOut" => return self.oauth_request(session, id, &method, params),
                    "command/metadata" => {
                        let command = self.state.borrow().frames.iter().find_map(|f| match &f.kind {
                            FrameKind::Extension { session: s, launched, .. } if Rc::ptr_eq(s, session) => {
                                Some(launched.command_id.clone())
                            }
                            _ => None,
                        });
                        if let Some(command) = command {
                            let subtitle = params.get("subtitle").and_then(Value::as_str).map(str::to_owned);
                            self.state.borrow_mut().command_meta.set(&command, subtitle);
                        }
                        Ok(Value::Null)
                    }
                    _ => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, format!("unknown method {method}"))),
                };
                session.respond(id, result);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Built-in views

    /// Runs `f` after `ms` without new typing; a newer call replaces it.
    fn debounce(self: &Rc<Self>, ms: u64, f: impl FnOnce(&Rc<Self>) + 'static) {
        if let Some(timer) = self.state.borrow_mut().search_timer.take() {
            timer.remove();
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            if let Some(l) = weak.upgrade() {
                l.state.borrow_mut().search_timer = None;
                f(&l);
            }
        });
        self.state.borrow_mut().search_timer = Some(timer);
    }

    fn open_builtin(self: &Rc<Self>, builtin: Builtin) {
        match builtin {
            Builtin::Windows if self.state.borrow().window_backend.is_none() => {
                self.state.borrow_mut().windows.clear();
                if let Some(kwin) = self.kwin.clone() {
                    // KWin answers asynchronously: show the view, then fill it.
                    self.spinner.set_spinning(true);
                    let weak = Rc::downgrade(self);
                    glib::spawn_future_local(async move {
                        let result = kwin.list().await;
                        let Some(l) = weak.upgrade() else { return };
                        l.spinner.set_spinning(false);
                        match result {
                            Ok(mut windows) => {
                                windows.retain(|w| !w.app_id.starts_with("app.backslash.Backslash"));
                                l.state.borrow_mut().windows = windows;
                                l.render();
                            }
                            Err(e) => l.fail(format!("Could not list windows: {e}")),
                        }
                    });
                }
            }
            Builtin::Windows => {
                let backend = self.state.borrow().window_backend;
                let Some(backend) = backend else { return };
                match backend.list() {
                    // The launcher itself is not a place to switch to.
                    Ok(mut windows) => {
                        windows.retain(|w| !w.app_id.starts_with("app.backslash.Backslash"));
                        self.state.borrow_mut().windows = windows;
                    }
                    Err(e) => return self.fail(format!("{e:#}")),
                }
            }
            Builtin::Files => {
                let mut s = self.state.borrow_mut();
                s.files.recent = files::recent(30);
                s.files.results.clear();
            }
            Builtin::Store => {
                // The cached list shows right away; a stale one updates under it.
                self.refresh_catalog(false, false);
                if let Some(e) = self.state.borrow().catalog.error.clone() {
                    self.fail(format!("The extension list may be out of date: {}", first_line(&e)));
                }
            }
            Builtin::Clipboard | Builtin::Theme | Builtin::Welcome => {}
        }
        self.record(format!("builtin:{}", builtin_id(builtin)));
        self.push(FrameKind::Builtin(builtin));
    }

    /// Searches files on a worker thread; only the newest search is shown.
    fn search_files(self: &Rc<Self>) {
        let query = self.entry.text().to_string();
        let (generation, cancel) = {
            let mut s = self.state.borrow_mut();
            s.files.cancel.store(true, Ordering::Relaxed);
            s.files.cancel = Arc::new(AtomicBool::new(false));
            s.files.generation += 1;
            (s.files.generation, s.files.cancel.clone())
        };
        if query.trim().chars().count() < 2 {
            self.state.borrow_mut().files.results.clear();
            return self.render();
        }
        self.spinner.set_spinning(true);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(files::search(&query, &cancel));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(results) = rx.recv().await else { return };
            let Some(l) = weak.upgrade() else { return };
            if l.state.borrow().files.generation != generation {
                return;
            }
            l.spinner.set_spinning(false);
            l.state.borrow_mut().files.results = results;
            let showing =
                matches!(l.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::Builtin(Builtin::Files)));
            if showing {
                l.render();
            }
        });
    }

    /// Shows the cached registry at once, and refreshes it in the background
    /// when it is older than an hour.
    /// Reads the registry copy, then refreshes it in the background when it
    /// is more than an hour old, or right away when `force`d. `announce`
    /// says what changed in a toast ("143 extensions, 45 new").
    fn refresh_catalog(self: &Rc<Self>, force: bool, announce: bool) {
        let loc = backslash_core::catalog::Locations::user(&self.state.borrow().config);
        self.state.borrow_mut().catalog.entries = backslash_core::catalog::entries(&loc);
        self.recount_updates();
        let stale = backslash_core::catalog::age(&loc).is_none_or(|age| age > Duration::from_secs(3600));
        if !(stale || force) || self.state.borrow().catalog.syncing {
            return;
        }
        self.state.borrow_mut().catalog.syncing = true;
        self.spinner.set_spinning(true);
        if self.window.is_visible() && self.in_store() {
            self.render();
        }
        let (tx, rx) = async_channel::bounded(1);
        let thread_loc = loc.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(backslash_core::catalog::sync(&thread_loc).map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("the update stopped".into()));
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            {
                let mut s = l.state.borrow_mut();
                s.catalog.syncing = false;
                s.catalog.entries = backslash_core::catalog::entries(&loc);
                s.catalog.error = result.as_ref().err().cloned();
            }
            l.recount_updates();
            let visible = l.window.is_visible();
            match &result {
                Err(e) if announce || (visible && l.in_store()) => {
                    l.fail(format!("Couldn't update the extension list: {}", first_line(e)))
                }
                Err(e) => eprintln!("backslash: could not update the extension list: {e}"),
                Ok(report) if announce => l.show_toast(&report.summary(), proto::ToastStyle::Success),
                Ok(_) => {}
            }
            if visible && !l.action_panel.is_visible() && !l.is_form() {
                l.render();
            }
        });
    }

    fn in_store(&self) -> bool {
        matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::Builtin(Builtin::Store)))
    }

    /// Recomputes which installed extensions have an update in the registry.
    fn recount_updates(&self) {
        let mut s = self.state.borrow_mut();
        let loc = backslash_core::catalog::Locations::user(&s.config);
        s.catalog.updates = store::pending_updates(&loc, &s.catalog.entries);
    }

    /// Refreshes the registry now and every hour, for the update badge (a
    /// shallow fetch of a small repository: cheap).
    fn watch_updates(self: &Rc<Self>) {
        self.refresh_catalog(false, false);
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(3600, move || {
            let Some(l) = weak.upgrade() else { return glib::ControlFlow::Break };
            l.refresh_catalog(false, false);
            glib::ControlFlow::Continue
        });
    }

    fn install_entry(self: &Rc<Self>, id: String) {
        self.install_entries(vec![id]);
    }

    /// Updates what can be updated without asking: the others ask for more
    /// permissions and go through the review screen.
    fn update_all(self: &Rc<Self>) {
        let (quiet, review) = {
            let s = self.state.borrow();
            let loc = backslash_core::catalog::Locations::user(&s.config);
            s.catalog.updates.iter().cloned().partition::<Vec<_>, _>(|id| {
                let Some(ext) = s.registry.find(id) else { return false };
                matches!(
                    backslash_core::catalog::check_update(&loc, id, &ext.manifest),
                    Ok(backslash_core::catalog::Update::Registry { ref new_permissions, .. }) if new_permissions.is_empty()
                )
            })
        };
        if !review.is_empty() {
            self.show_toast(
                &format!("{} asks for more permissions: review it with Update…", review.join(", ")),
                proto::ToastStyle::Info,
            );
        }
        if !quiet.is_empty() {
            self.install_entries(quiet);
        }
    }

    /// Downloads, verifies and installs registry entries on a worker thread.
    /// Confirming the install screen is the user's consent to its permissions.
    fn install_entries(self: &Rc<Self>, ids: Vec<String>) {
        let (entries, loc) = {
            let s = self.state.borrow();
            let entries: Vec<_> = s.catalog.entries.iter().filter(|e| ids.contains(&e.id)).cloned().collect();
            (entries, backslash_core::catalog::Locations::user(&s.config))
        };
        if entries.is_empty() {
            return;
        }
        if self.state.borrow().catalog.busy.is_some() {
            return self.fail("Another installation is running");
        }
        self.state.borrow_mut().catalog.busy = Some(entries[0].id.clone());
        if matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::ConfirmInstall { .. })) {
            self.pop();
        }
        self.spinner.set_spinning(true);
        self.render();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            for entry in entries {
                let _ = tx.send_blocking((entry.id.clone(), None));
                let result = backslash_core::catalog::install_entry(&loc, &entry)
                    .and_then(|(_, manifest)| approvals::approve(&manifest).map(|_| manifest.extension.title));
                let _ = tx.send_blocking((entry.id, Some(result.map_err(|e| format!("{e:#}")))));
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut done = Vec::new();
            while let Ok((id, result)) = rx.recv().await {
                let Some(l) = weak.upgrade() else { return };
                match result {
                    None => {
                        l.state.borrow_mut().catalog.busy = Some(id);
                        l.render();
                    }
                    Some(Ok(title)) => done.push(title),
                    Some(Err(e)) => l.fail(e),
                }
            }
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            l.state.borrow_mut().catalog.busy = None;
            l.reload();
            l.recount_updates();
            match done.len() {
                0 => {}
                1 => l.show_toast(&format!("{} installed", done[0]), proto::ToastStyle::Success),
                n => l.show_toast(&format!("{n} extensions updated"), proto::ToastStyle::Success),
            }
            l.render();
        });
    }

    /// Downloads Deno on a worker thread, then runs the command that needed it.
    fn install_runtime(self: &Rc<Self>) {
        if self.state.borrow().runtime_busy {
            return;
        }
        self.state.borrow_mut().runtime_busy = true;
        self.spinner.set_spinning(true);
        self.render();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(runtime::install().map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("download thread died".into()));
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            l.state.borrow_mut().runtime_busy = false;
            let pending = match l.state.borrow().frames.last().map(|f| &f.kind) {
                Some(FrameKind::InstallRuntime { command_id, arguments }) => {
                    Some((command_id.clone(), arguments.clone()))
                }
                _ => None,
            };
            match result {
                Ok(_) => {
                    l.show_toast("Deno installed", proto::ToastStyle::Success);
                    if let Some((command_id, arguments)) = pending {
                        l.pop();
                        l.start_command(&command_id, arguments);
                    }
                }
                Err(e) => {
                    l.fail(e);
                    l.render();
                }
            }
        });
    }

    /// Runs a script command: asks for its arguments first, then runs it the
    /// way its mode says.
    fn run_script(self: &Rc<Self>, id: &str, arguments: Option<Vec<String>>) {
        let Some(script) = self.state.borrow().scripts.iter().find(|x| x.id == id).cloned() else {
            return self.fail(format!("No script command {id}"));
        };
        let arguments = match arguments {
            Some(a) => a,
            None if !script.arguments.is_empty() => {
                let form = script_arguments_form(&script);
                return self.push(FrameKind::Form { purpose: FormPurpose::ScriptArguments { id: id.into() }, form });
            }
            None => Vec::new(),
        };
        if !scripts::is_executable(&script.path) {
            return self.fail(format!("{} is not executable: run chmod +x on it", files::tilde(&script.path)));
        }
        self.record(format!("script:{id}"));
        let list = script.mode == ScriptMode::List;
        match script.mode {
            ScriptMode::Output | ScriptMode::List => {
                let kind = FrameKind::Script {
                    id: id.into(),
                    title: script.title.clone(),
                    arguments: arguments.clone(),
                    result: None,
                    list,
                };
                // Run Again replaces the view instead of stacking another.
                let rerun = matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::Script { id: x, .. }) if x == id);
                if rerun {
                    self.state.borrow_mut().frames.last_mut().unwrap().kind = kind;
                    self.render();
                } else {
                    self.push(kind);
                }
            }
            ScriptMode::Silent => self.hide(),
            ScriptMode::Paste => self.spinner.set_spinning(true),
        }
        let (tx, rx) = async_channel::bounded(1);
        let path = script.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(execute_script(&path, &arguments));
        });
        let weak = Rc::downgrade(self);
        let id = id.to_string();
        let title = script.title.clone();
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("the script thread died".into()));
            let Some(l) = weak.upgrade() else { return };
            match script.mode {
                ScriptMode::Output | ScriptMode::List => {
                    let mut s = l.state.borrow_mut();
                    let Some(frame) = s
                        .frames
                        .iter_mut()
                        .rev()
                        .find(|f| matches!(&f.kind, FrameKind::Script { id: x, .. } if *x == id))
                    else {
                        return;
                    };
                    if let FrameKind::Script { result: r, .. } = &mut frame.kind {
                        *r = Some(result);
                    }
                    drop(s);
                    l.render();
                }
                ScriptMode::Paste => {
                    l.spinner.set_spinning(false);
                    match result {
                        Ok(out) => l.paste_text(out.trim_end().to_string()),
                        Err(e) => l.fail(e),
                    }
                }
                ScriptMode::Silent => {
                    if let Err(e) = result {
                        eprintln!("backslash: {title}: {e}");
                        if let Some(app) = l.window.application() {
                            let n = gio::Notification::new(&format!("{title} failed"));
                            n.set_body(Some(&e));
                            app.send_notification(Some("script"), &n);
                        }
                    }
                }
            }
        });
    }

    /// Pastes (or copies) a snippet, expanding `{date}`, `{time}` and `{clipboard}`.
    fn copy_snippet(self: &Rc<Self>, text: String, paste: bool) {
        let now = glib::DateTime::now_local().ok();
        let fmt = |f: &str| now.as_ref().and_then(|d| d.format(f).ok()).map(|s| s.to_string()).unwrap_or_default();
        let text = text.replace("{date}", &fmt("%x")).replace("{time}", &fmt("%H:%M"));
        let deliver = move |l: &Rc<Self>, text: String| {
            if paste {
                l.paste_text(text);
            } else {
                l.window.clipboard().set_text(&text);
                l.flash_and_hide("Snippet copied");
            }
        };
        if !text.contains("{clipboard}") {
            return deliver(self, text);
        }
        let clipboard = self.window.clipboard();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let current = clipboard.read_text_future().await.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
            if let Some(l) = weak.upgrade() {
                deliver(&l, text.replace("{clipboard}", &current));
            }
        });
    }

    /// Puts `text` in the clipboard, hides, and pastes it into the app that
    /// had the focus. Where pasting is not possible, it stays copied.
    fn paste_text(self: &Rc<Self>, text: String) {
        self.paste(text, false);
    }

    fn paste(self: &Rc<Self>, text: String, concealed: bool) {
        set_clipboard(&self.window, &text, concealed);
        let (method, terminal) = {
            let s = self.state.borrow();
            (s.paste, s.paste_target.as_deref().is_some_and(paste::is_terminal))
        };
        let Some(method) = method else {
            return self.flash_and_hide("Copied: press Ctrl+V to paste");
        };
        self.hide();
        let paster = self.paster.clone();
        let app = self.window.application();
        glib::spawn_future_local(async move {
            if let Err(e) = paster.paste(method, terminal).await {
                eprintln!("backslash: could not paste ({e}); the text is in the clipboard");
                if let Some(app) = app {
                    let n = gio::Notification::new("Copied, but could not paste");
                    n.set_body(Some(&format!("Press Ctrl+V. ({e})")));
                    app.send_notification(Some("paste"), &n);
                }
            }
        });
    }
}

/// Puts text on the clipboard. A concealed text carries the
/// `x-kde-passwordManagerHint` type that password managers use, so clipboard
/// histories (ours, Klipper, CopyQ, cliphist with a filter…) skip it.
fn set_clipboard(window: &gtk::ApplicationWindow, text: &str, concealed: bool) {
    if !concealed {
        window.clipboard().set_text(text);
        return;
    }
    let provider = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&text.to_value()),
        gdk::ContentProvider::for_bytes("x-kde-passwordManagerHint", &glib::Bytes::from_static(b"secret")),
    ]);
    if let Err(e) = window.clipboard().set_content(Some(&provider)) {
        eprintln!("backslash: could not copy: {e}");
    }
}

fn builtin_id(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::Clipboard => "clipboard",
        Builtin::Files => "files",
        Builtin::Windows => "windows",
        Builtin::Theme => "theme",
        Builtin::Store => "store",
        Builtin::Welcome => "welcome",
    }
}

/// Shows a file selected in the file manager (FileManager1 D-Bus interface,
/// implemented by Nautilus, Dolphin, Nemo, Thunar…), else opens its folder.
fn reveal(path: &std::path::Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let parent = path.parent().map(|p| p.display().to_string());
    glib::spawn_future_local(async move {
        let shown = match gio::bus_get_future(gio::BusType::Session).await {
            Ok(bus) => bus
                .call_future(
                    Some("org.freedesktop.FileManager1"),
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1",
                    "ShowItems",
                    Some(&(vec![uri], "").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    3000,
                )
                .await
                .is_ok(),
            Err(_) => false,
        };
        if !shown {
            if let Some(parent) = parent {
                let _ = apps::open(&parent);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Helpers

/// Scrolls `scroller` just enough for `row` of `list` to be visible. The first
/// row scrolls to the very top, to show its section header.
fn scroll_into_view(row: &gtk::ListBoxRow, list: &gtk::ListBox, scroller: &gtk::ScrolledWindow) {
    let adj = scroller.vadjustment();
    if row.index() == 0 {
        adj.set_value(0.0);
        return;
    }
    let Some(bounds) = row.compute_bounds(list) else { return };
    let (mut top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
    // A section's first row comes with its header: show both.
    if let Some(header) = row.header().and_then(|h| h.compute_bounds(list)) {
        top = top.min(header.y() as f64);
    }
    if top < adj.value() {
        adj.set_value(top - 6.0);
    } else if bottom > adj.value() + adj.page_size() {
        adj.set_value(bottom - adj.page_size() + 6.0);
    }
}

fn view_id(view: &View) -> Option<String> {
    view_id_ref(view).map(str::to_owned)
}

fn view_id_ref(view: &View) -> Option<&str> {
    match view {
        View::List(v) => v.id.as_deref(),
        View::Detail(v) => v.id.as_deref(),
        View::Form(v) => v.id.as_deref(),
        View::Menu(v) => v.id.as_deref(),
    }
}

fn matching_shortcut(actions: &[UiAction], key: gdk::Key, mods: gdk::ModifierType) -> Option<UiAction> {
    let relevant = gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK;
    let pressed = mods & relevant;
    if pressed.is_empty() {
        return None;
    }
    actions
        .iter()
        .find(|a| {
            a.shortcut
                .as_deref()
                .and_then(gtk::accelerator_parse)
                .is_some_and(|(k, m)| k.to_lower() == key.to_lower() && (m & relevant) == pressed)
        })
        .cloned()
}

/// Runs a script and returns what it printed; an error carries the exit
/// status on its first line, then what it printed.
fn execute_script(path: &std::path::Path, arguments: &[String]) -> Result<String, String> {
    let out = std::process::Command::new(path)
        .args(arguments)
        .current_dir(dirs::home_dir().unwrap_or_else(|| "/".into()))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("Could not run it: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let code = out.status.code().map_or("a signal".to_string(), |c| format!("code {c}"));
        Err(format!("It exited with {code}\n{}{}", stdout, stderr).trim_end().to_string())
    }
}

fn script_arguments_form(script: &Script) -> FormView {
    FormView {
        id: None,
        title: Some(script.title.clone()),
        submit_title: Some("Run".into()),
        fields: script
            .arguments
            .iter()
            .map(|a| proto::Field {
                id: a.name.clone(),
                title: title_case(&a.name),
                placeholder: a.placeholder.clone(),
                required: a.required,
                value: None,
                kind: FieldKind::Text,
            })
            .collect(),
    }
}

fn new_script_form() -> FormView {
    let option = |value: &str, title: &str| proto::DropdownOption { value: value.into(), title: title.into() };
    FormView {
        id: None,
        title: Some("New Script Command".into()),
        submit_title: Some("Create and Edit".into()),
        fields: vec![
            proto::Field {
                id: "title".into(),
                title: "Title".into(),
                placeholder: Some("Restart Wi-Fi".into()),
                required: true,
                value: None,
                kind: FieldKind::Text,
            },
            proto::Field {
                id: "mode".into(),
                title: "When it runs".into(),
                placeholder: None,
                required: true,
                value: None,
                kind: FieldKind::Dropdown {
                    options: vec![
                        option("silent", "Run in the background"),
                        option("output", "Show what it prints"),
                        option("list", "List what it prints, one item per line"),
                        option("paste", "Paste what it prints"),
                    ],
                },
            },
        ],
    }
}

fn arguments_form(command: &backslash_core::manifest::CommandDef) -> FormView {
    FormView {
        id: None,
        title: Some(command.title.clone()),
        submit_title: Some("Run".into()),
        fields: command
            .arguments
            .iter()
            .map(|a| proto::Field {
                id: a.name.clone(),
                title: title_case(&a.name),
                placeholder: a.placeholder.clone(),
                required: a.required,
                value: None,
                kind: if a.options.is_empty() {
                    FieldKind::Text
                } else {
                    FieldKind::Dropdown {
                        options: a
                            .options
                            .iter()
                            .map(|o| proto::DropdownOption { value: o.clone(), title: o.clone() })
                            .collect(),
                    }
                },
            })
            .collect(),
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The first line of an error, for a toast.
fn first_line(e: &str) -> &str {
    e.lines().next().unwrap_or(e)
}

/// "5 minutes ago", "3 hours ago", "2 days ago".
fn ago(d: Duration) -> String {
    let (n, unit) = match d.as_secs() {
        s if s < 3600 => ((s / 60).max(1), "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s => (s / 86_400, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}
