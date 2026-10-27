//! The launcher window: a search bar with filters, a navigation stack of
//! views, a results list with a preview panel, a footer with the primary
//! action, and an action panel (Ctrl+K).
//!
//! The root view searches applications, extension commands and built-in
//! commands; extension views come from extension processes (see
//! `backslash_core::host`); built-in views (clipboard, files, windows, theme)
//! are implemented here.

mod model;
mod rows;
mod settings;
mod store;
mod widgets;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use backslash_core::config::Config;
use backslash_core::host::{Incoming, RpcError, Session};
use backslash_core::protocol::{self as proto, Effect, FieldKind, Filtering, FormView, View};
use backslash_core::registry::{CommandEntry, Registry};
use backslash_core::search::{Ranker, Usage};
use backslash_core::system::clipboard::ClipboardHistory;
use backslash_core::system::files;
use backslash_core::system::windows::{Backend, WindowInfo};
use backslash_core::{approvals, paths, runner, secrets};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{json, Map, Value};

use self::model::{Builtin, Filter, FormInput, FormPurpose, Frame, FrameKind, Launched, Op, Row, UiAction};
use self::widgets::{key_hint, label, pretty_accel, row_widget};
use crate::apps::{self, App};
use crate::currency::Currency;
use crate::icons::{self, Icon};
use crate::theme::Styles;
use crate::{autostart, clipboard, hotkey, layer, markdown, theme};

pub use self::rows::now;

pub struct Options {
    pub hidden: bool,
    pub dev_dir: Option<PathBuf>,
}

const WIDTH: i32 = 860;
const HEIGHT: i32 = 540;
const LIST_WIDTH: i32 = 400;

pub fn run(options: Options) -> anyhow::Result<()> {
    layer::reexec_if_needed(Config::load().layer_shell);

    let dev = options.dev_dir.is_some();
    let app_id = if dev { "app.backslash.Backslash.Devel" } else { "app.backslash.Backslash" };
    let app = gtk::Application::builder().application_id(app_id).build();

    let launcher: Rc<RefCell<Option<Rc<Launcher>>>> = Rc::default();
    let first_activation = Cell::new(true);
    let dev_dir = options.dev_dir.clone();
    let hidden = options.hidden;

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
    toast_timer: Option<glib::SourceId>,
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
}

pub struct Launcher {
    window: gtk::ApplicationWindow,
    styles: Styles,
    layer_shell: bool,
    entry: gtk::Entry,
    back: gtk::Button,
    search_icon: gtk::Image,
    chips: gtk::Box,
    spinner: gtk::Spinner,
    pages: gtk::Stack,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    preview: gtk::Box,
    sections: Rc<RefCell<Vec<Option<String>>>>,
    empty_title: gtk::Label,
    empty_subtitle: gtk::Label,
    detail: gtk::Label,
    metadata: gtk::Box,
    form_box: gtk::Box,
    context: gtk::Label,
    toast: gtk::Label,
    primary: gtk::Label,
    primary_key: gtk::Box,
    actions_hint: gtk::Box,
    action_panel: gtk::Box,
    popover_list: gtk::ListBox,
    popover_actions: RefCell<Vec<UiAction>>,
    state: RefCell<State>,
    dev_dir: Option<PathBuf>,
    /// Set while the code (not the user) changes the search text.
    setting_query: Cell<bool>,
    /// `backslash dev`: watches the extension's files for hot reload.
    monitors: RefCell<Vec<gio::FileMonitor>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
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

        // Header: back or search icon, query, filters, spinner.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("header");
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.add_css_class("back");
        back.set_valign(gtk::Align::Center);
        back.set_focusable(false);
        back.set_visible(false);
        let search_icon = gtk::Image::from_icon_name("system-search-symbolic");
        search_icon.add_css_class("search-icon");
        search_icon.set_pixel_size(18);
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
        let list_page = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        list_page.append(&scroller);
        list_page.append(&divider());
        list_page.append(&preview);
        pages.add_named(&list_page, Some("list"));

        let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
        empty.add_css_class("empty");
        empty.set_valign(gtk::Align::Center);
        let empty_icon = gtk::Image::from_icon_name("system-search-symbolic");
        empty_icon.set_pixel_size(42);
        empty_icon.set_opacity(0.4);
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
        detail.set_xalign(0.0);
        detail.set_yalign(0.0);
        detail.set_selectable(true);
        detail.set_hexpand(true);
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&detail)
            .hexpand(true)
            .build();
        let metadata = gtk::Box::new(gtk::Orientation::Vertical, 4);
        metadata.add_css_class("metadata");
        detail_row.append(&detail_scroll);
        detail_row.append(&metadata);
        pages.add_named(&detail_row, Some("detail"));

        let form_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        form_box.add_css_class("form");
        let form_scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&form_box).build();
        pages.add_named(&form_scroll, Some("form"));

        // Footer: context, toast, primary action, action panel hint.
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.add_css_class("footer");
        let context = label("Backslash", "context");
        let toast = label("", "toast");
        toast.set_hexpand(true);
        toast.set_xalign(1.0);
        toast.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let primary = label("", "primary");
        let primary_key = key_hint("", "↵");
        let actions_hint = key_hint("Actions", "Ctrl K");
        actions_hint.set_margin_start(6);
        footer.append(&context);
        footer.append(&toast);
        footer.append(&primary);
        footer.append(&primary_key);
        footer.append(&actions_hint);

        panel.append(&header);
        panel.append(&divider());
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
        action_panel.set_margin_bottom(58);
        action_panel.set_size_request(280, -1);
        action_panel.set_visible(false);
        let popover_list = gtk::ListBox::new();
        popover_list.set_selection_mode(gtk::SelectionMode::Browse);
        action_panel.append(&popover_list);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&panel));
        overlay.add_overlay(&action_panel);
        window.set_child(Some(&overlay));

        let config = Config::load();
        styles.apply(config.theme, &window);
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

        let launcher = Rc::new(Self {
            window,
            styles,
            layer_shell,
            entry,
            back,
            search_icon,
            chips,
            spinner,
            pages,
            list,
            scroller,
            preview,
            sections: Rc::default(),
            empty_title,
            empty_subtitle,
            detail,
            metadata,
            form_box,
            context,
            toast,
            primary,
            primary_key,
            actions_hint,
            action_panel,
            popover_list,
            popover_actions: RefCell::default(),
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
                toast_timer: None,
                form: Vec::new(),
                selected_key: None,
                filter: Filter::All,
                clipboard,
                files: FilesState::default(),
                windows: Vec::new(),
                window_backend: Backend::detect(),
                currency: Currency::new(),
                autostart: autostart::enabled(),
                catalog: store::CatalogState::default(),
            }),
            dev_dir,
            setting_query: Cell::new(false),
            monitors: RefCell::default(),
            reload_timer: RefCell::default(),
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
        let (hotkey, clipboard_enabled) = {
            let s = self.state.borrow();
            (s.config.hotkey.clone(), s.config.clipboard.enabled)
        };
        if !hotkey.is_empty() && self.dev_dir.is_none() {
            let weak = Rc::downgrade(self);
            let app_id = self.window.application().and_then(|a| a.application_id()).unwrap_or_default();
            hotkey::register(
                &app_id,
                &hotkey,
                move || {
                    if let Some(l) = weak.upgrade() {
                        l.toggle();
                    }
                },
                |status| eprintln!("backslash: {status}"),
            );
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
                let theme = l.state.borrow().config.theme;
                l.styles.apply(theme, &l.window);
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
                Some(s) if previous.as_ref() != Some(&s) => {
                    row.set_header(Some(&label(&s.to_uppercase(), "section-header")))
                }
                _ => row.set_header(gtk::Widget::NONE),
            }
        });

        let weak = Rc::downgrade(self);
        self.popover_list.connect_row_activated(move |_, row| {
            if let Some(l) = weak.upgrade() {
                let action = l.popover_actions.borrow().get(row.index() as usize).cloned();
                l.close_action_panel();
                if let Some(action) = action {
                    l.run_action(action);
                }
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
        let theme = self.state.borrow().config.theme;
        self.styles.apply(theme, &self.window);
        if self.layer_shell {
            let height = WidgetExt::display(&self.window)
                .monitors()
                .item(0)
                .and_downcast::<gdk::Monitor>()
                .map_or(1080, |m| m.geometry().height());
            layer::set_top_margin(&self.window, height / 6);
        }
        self.render();
        self.window.present();
        self.entry.grab_focus();
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
        // Start fresh next time; dropping the frames stops extension processes.
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
                FrameKind::Extension { view: View::Detail(_), title, .. } => (title.clone(), "detail"),
                FrameKind::Extension { view: View::Form(_), title, .. } => (title.clone(), "form"),
                FrameKind::Form { form, .. } => (form.title.clone().unwrap_or_default(), "form"),
                FrameKind::Approval { .. } => ("Permissions".to_string(), "detail"),
                FrameKind::ConfirmInstall { .. } => ("Extensions".to_string(), "detail"),
            };
            (s.frames.len(), title, page)
        };
        self.back.set_visible(depth > 1);
        self.search_icon.set_visible(depth == 1);
        self.context.set_text(&title);
        self.update_chips();
        match page {
            "list" => self.render_list(),
            "detail" => self.render_detail(),
            _ => self.render_form(),
        }
        self.update_footer();
    }

    fn render_list(self: &Rc<Self>) {
        let query = self.entry.text().to_string();
        let (rows, placeholder, empty) = {
            let mut guard = self.state.borrow_mut();
            let s = &mut *guard;
            match &s.frames.last().unwrap().kind {
                FrameKind::Root => (rows::root_rows(s, &query), "Search apps, commands, math…".to_string(), None),
                FrameKind::Builtin(b) => {
                    let b = *b;
                    let (title, description) = b.empty();
                    let empty =
                        proto::EmptyState { title: title.into(), description: Some(description.into()), icon: None };
                    (rows::builtin_rows(s, b, &query), b.placeholder().to_string(), Some(empty))
                }
                FrameKind::Extension { view: View::List(list), session, .. } => {
                    if s.search.is_none() {
                        self.spinner.set_spinning(list.loading);
                    }
                    let rows = rows::list_rows(&mut s.ranker, list, &query, &session.extension_id);
                    (rows, list.placeholder.clone().unwrap_or_else(|| "Search…".into()), list.empty.clone())
                }
                _ => unreachable!("render_list on a non-list frame"),
            }
        };
        self.entry.set_editable(true);
        self.entry.set_placeholder_text(Some(&placeholder));

        let previous_key = self.state.borrow().selected_key.clone();
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

    fn update_preview(&self) {
        let Some(row) = self.selected_row() else {
            while let Some(child) = self.preview.first_child() {
                self.preview.remove(&child);
            }
            return;
        };
        widgets::fill_preview(&self.preview, &row.preview(), &self.current_actions());
    }

    fn render_detail(&self) {
        let s = self.state.borrow();
        let approval;
        let d = match s.frames.last().map(|f| &f.kind) {
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
        self.entry.set_editable(false);
        self.entry.set_placeholder_text(d.title.as_deref());
        self.detail.set_markup(&markdown::to_pango(&d.markdown));
        while let Some(child) = self.metadata.first_child() {
            self.metadata.remove(&child);
        }
        self.metadata.set_visible(!d.metadata.is_empty());
        for m in &d.metadata {
            self.metadata.append(&label(&m.label, "label"));
            let value = label(&m.value, "value");
            value.set_selectable(true);
            value.set_wrap(true);
            value.set_margin_bottom(8);
            self.metadata.append(&value);
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
        let index = self.list.selected_row()?.index();
        self.state.borrow().rows.get(index as usize).cloned()
    }

    /// Actions for the current context: the selected row, or the view itself.
    fn current_actions(&self) -> Vec<UiAction> {
        let view_actions = {
            let s = self.state.borrow();
            let from = |actions: &[proto::Action]| actions.iter().map(UiAction::from_proto).collect::<Vec<_>>();
            match &s.frames.last().unwrap().kind {
                FrameKind::Extension { view: View::Detail(d), .. } => return from(&d.actions),
                FrameKind::Extension { view: View::Form(_), .. } | FrameKind::Form { .. } => return Vec::new(),
                FrameKind::ConfirmInstall { id } => {
                    return vec![
                        UiAction::new("Install", Op::InstallEntry(id.clone())).icon("object-select"),
                        UiAction::new("Cancel", Op::Back).icon("window-close"),
                    ]
                }
                FrameKind::Approval { .. } => {
                    return vec![
                        UiAction::new("Allow and Run", Op::Approve).icon("object-select"),
                        UiAction::new("Cancel", Op::Back).icon("window-close"),
                    ]
                }
                FrameKind::Extension { view: View::List(l), .. } => from(&l.actions),
                FrameKind::Root | FrameKind::Builtin(_) => Vec::new(),
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
            self.primary.set_text("Submit");
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
        let toast = self.toast.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(3500), move || toast.set_text(""));
        if let Some(old) = self.state.borrow_mut().toast_timer.replace(id) {
            old.remove();
        }
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
        let Some(bounds) = row.compute_bounds(&self.list) else { return };
        let adj = self.scroller.vadjustment();
        let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
        if top < adj.value() {
            adj.set_value(top - 6.0);
        } else if bottom > adj.value() + adj.page_size() {
            adj.set_value(bottom - adj.page_size() + 6.0);
        }
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
            }
        };
        match key {
            gdk::Key::Escape => self.close_action_panel(),
            gdk::Key::k | gdk::Key::K if ctrl => self.close_action_panel(),
            gdk::Key::Down => select(selected + 1),
            gdk::Key::Up => select(selected - 1),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                self.close_action_panel();
                if let Some(action) = actions.get(selected as usize).cloned() {
                    self.run_action(action);
                }
            }
            _ => match matching_shortcut(&actions, key, mods) {
                Some(action) => {
                    self.close_action_panel();
                    self.run_action(action);
                }
                None => return Proceed,
            },
        }
        Stop
    }

    fn open_action_panel(&self) {
        let actions = self.current_actions();
        if actions.is_empty() {
            return;
        }
        while let Some(child) = self.popover_list.first_child() {
            self.popover_list.remove(&child);
        }
        for (i, a) in actions.iter().enumerate() {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.append(&icons::widget(a.icon.clone().map(Icon::Spec).as_ref(), 16));
            let title = label(&a.title, "title");
            title.set_hexpand(true);
            if a.destructive {
                title.add_css_class("destructive");
            }
            b.append(&title);
            let shortcut = match (i, &a.shortcut) {
                (0, _) => Some("↵".to_string()),
                (1, None) => Some("Ctrl ↵".to_string()),
                (_, Some(s)) => Some(pretty_accel(s)),
                _ => None,
            };
            if let Some(s) = shortcut {
                b.append(&label(&s, "kind"));
            }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&b));
            self.popover_list.append(&row);
        }
        *self.popover_actions.borrow_mut() = actions;
        if let Some(first) = self.popover_list.row_at_index(0) {
            self.popover_list.select_row(Some(&first));
        }
        self.action_panel.set_visible(true);
    }

    fn close_action_panel(&self) {
        self.action_panel.set_visible(false);
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
            Op::Open(target) => match apps::open(&target) {
                Ok(()) => self.hide(),
                Err(e) => self.fail(e),
            },
            Op::Callback(id) => self.callback(&id, None),
            Op::Snippet(text) => self.copy_snippet(text),
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
                self.styles.apply(theme, &self.window);
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
            Op::ToggleExtension(id) => {
                let enabled = self.state.borrow().config.is_enabled(&id);
                self.update_config(|c| c.extensions.entry(id.clone()).or_default().enabled = !enabled);
                self.render();
            }
            Op::ConfigureExtension(id) => {
                let form = {
                    let s = self.state.borrow();
                    let Some(ext) = s.registry.find(&id) else { return };
                    settings::preferences_form(&ext.manifest, &s.config)
                };
                let purpose = FormPurpose::Preferences { extension_id: id, then_run: None };
                self.push(FrameKind::Form { purpose, form });
            }
            Op::EditQuicklink(index) => {
                let form = settings::quicklink_form(
                    index.and_then(|i| self.state.borrow().config.quicklinks.get(i).cloned()).as_ref(),
                );
                self.push(FrameKind::Form { purpose: FormPurpose::Quicklink { index }, form });
            }
            Op::DeleteQuicklink(index) => {
                self.update_config(|c| {
                    if index < c.quicklinks.len() {
                        c.quicklinks.remove(index);
                    }
                });
                self.render();
            }
            Op::EditSnippet(index) => {
                let form = settings::snippet_form(
                    index.and_then(|i| self.state.borrow().config.snippets.get(i).cloned()).as_ref(),
                );
                self.push(FrameKind::Form { purpose: FormPurpose::Snippet { index }, form });
            }
            Op::DeleteSnippet(index) => {
                self.update_config(|c| {
                    if index < c.snippets.len() {
                        c.snippets.remove(index);
                    }
                });
                self.render();
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
            Op::UninstallEntry(id) => {
                let loc = backslash_core::catalog::Locations::user(&self.state.borrow().config);
                match backslash_core::catalog::uninstall(&loc, &id) {
                    Ok(()) => {
                        self.reload();
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
        }
        let prepared = {
            let s = self.state.borrow();
            match s.registry.resolve(id) {
                None => Prepared::Failed(format!("Unknown command {id}")),
                Some((ext, command)) => {
                    let manifest = &ext.manifest;
                    let ext_id = manifest.extension.id.clone();
                    let (_, missing) = s.config.preferences(manifest);
                    if !approvals::requested(manifest).is_empty() && !approvals::is_approved(manifest) {
                        Prepared::Push(FrameKind::Approval { extension_id: ext_id, command_id: id.into(), arguments })
                    } else if !missing.is_empty() {
                        Prepared::Push(FrameKind::Form {
                            purpose: FormPurpose::Preferences { extension_id: ext_id, then_run: Some(id.into()) },
                            form: settings::preferences_form(manifest, &s.config),
                        })
                    } else if arguments.is_none() && !command.arguments.is_empty() {
                        Prepared::Push(FrameKind::Form {
                            purpose: FormPurpose::Arguments { command_id: id.into() },
                            form: arguments_form(command),
                        })
                    } else {
                        let arguments = arguments.unwrap_or_default();
                        match runner::start(ext, command, &s.config, arguments.clone(), "") {
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
        let optional = |k: &str| Some(text(k)).filter(|t| !t.is_empty());
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
            FormPurpose::Quicklink { index } => {
                let link = backslash_core::config::Quicklink {
                    name: text("name"),
                    url: text("url"),
                    keyword: optional("keyword"),
                    icon: index.and_then(|i| self.state.borrow().config.quicklinks.get(i).and_then(|l| l.icon.clone())),
                };
                if !link.url.contains("://") {
                    return self.fail("The URL must start with https:// (or another scheme)");
                }
                self.update_config(|c| match index {
                    Some(i) if i < c.quicklinks.len() => c.quicklinks[i] = link,
                    _ => c.quicklinks.push(link),
                });
                self.pop();
            }
            FormPurpose::Snippet { index } => {
                let snippet = backslash_core::config::Snippet {
                    name: text("name"),
                    text: values.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
                    keyword: optional("keyword"),
                };
                self.update_config(|c| match index {
                    Some(i) if i < c.snippets.len() => c.snippets[i] = snippet,
                    _ => c.snippets.push(snippet),
                });
                self.pop();
            }
            FormPurpose::Hotkey => {
                let hotkey = text("hotkey");
                self.update_config(|c| c.hotkey = hotkey);
                self.pop();
                self.show_toast("Saved. The new shortcut applies at the next start", proto::ToastStyle::Info);
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
            let Some(i) = position else { return };
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
                        self.window.clipboard().set_text(&text("text"));
                        Ok(Value::Null)
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
            Builtin::Store => self.refresh_catalog(),
            Builtin::Clipboard | Builtin::Theme | Builtin::Settings => {}
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
    fn refresh_catalog(self: &Rc<Self>) {
        let loc = backslash_core::catalog::Locations::user(&self.state.borrow().config);
        self.state.borrow_mut().catalog.entries = backslash_core::catalog::entries(&loc);
        let stale = backslash_core::catalog::age(&loc).is_none_or(|age| age > Duration::from_secs(3600));
        if !stale || self.state.borrow().catalog.syncing {
            return;
        }
        self.state.borrow_mut().catalog.syncing = true;
        self.spinner.set_spinning(true);
        let (tx, rx) = async_channel::bounded(1);
        let thread_loc = loc.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(backslash_core::catalog::sync(&thread_loc).map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("sync thread died".into()));
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            {
                let mut s = l.state.borrow_mut();
                s.catalog.syncing = false;
                s.catalog.entries = backslash_core::catalog::entries(&loc);
                s.catalog.error = result.as_ref().err().cloned();
            }
            if let Err(e) = result {
                l.fail(format!("Could not update the extension list: {e}"));
            }
            if matches!(l.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::Builtin(Builtin::Store))) {
                l.render();
            }
        });
    }

    /// Downloads, verifies and installs a registry entry on a worker thread.
    /// Confirming the install screen is the user's consent to its permissions.
    fn install_entry(self: &Rc<Self>, id: String) {
        let (entry, loc) = {
            let s = self.state.borrow();
            let Some(entry) = s.catalog.entries.iter().find(|e| e.id == id).cloned() else { return };
            (entry, backslash_core::catalog::Locations::user(&s.config))
        };
        if self.state.borrow().catalog.busy.is_some() {
            return self.fail("Another installation is running");
        }
        self.state.borrow_mut().catalog.busy = Some(id.clone());
        if matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::ConfirmInstall { .. })) {
            self.pop();
        }
        self.spinner.set_spinning(true);
        self.render();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = backslash_core::catalog::install_entry(&loc, &entry)
                .and_then(|(_, manifest)| approvals::approve(&manifest).map(|_| manifest.extension.title));
            let _ = tx.send_blocking(result.map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("install thread died".into()));
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            l.state.borrow_mut().catalog.busy = None;
            l.reload();
            match result {
                Ok(title) => l.show_toast(&format!("{title} installed"), proto::ToastStyle::Success),
                Err(e) => l.fail(e),
            }
            l.render();
        });
    }

    /// Copies a snippet, expanding `{date}`, `{time}` and `{clipboard}`.
    fn copy_snippet(self: &Rc<Self>, text: String) {
        let now = glib::DateTime::now_local().ok();
        let fmt = |f: &str| now.as_ref().and_then(|d| d.format(f).ok()).map(|s| s.to_string()).unwrap_or_default();
        let text = text.replace("{date}", &fmt("%x")).replace("{time}", &fmt("%H:%M"));
        if !text.contains("{clipboard}") {
            self.window.clipboard().set_text(&text);
            return self.flash_and_hide("Snippet copied");
        }
        let clipboard = self.window.clipboard();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let current = clipboard.read_text_future().await.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
            clipboard.set_text(&text.replace("{clipboard}", &current));
            if let Some(l) = weak.upgrade() {
                l.flash_and_hide("Snippet copied");
            }
        });
    }
}

fn builtin_id(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::Clipboard => "clipboard",
        Builtin::Files => "files",
        Builtin::Windows => "windows",
        Builtin::Theme => "theme",
        Builtin::Settings => "settings",
        Builtin::Store => "store",
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

fn view_id(view: &View) -> Option<String> {
    view_id_ref(view).map(str::to_owned)
}

fn view_id_ref(view: &View) -> Option<&str> {
    match view {
        View::List(v) => v.id.as_deref(),
        View::Detail(v) => v.id.as_deref(),
        View::Form(v) => v.id.as_deref(),
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
