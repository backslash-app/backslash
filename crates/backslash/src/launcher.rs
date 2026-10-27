//! The launcher window: a search bar, a navigation stack of views, a footer
//! with the primary action, and an action panel (Ctrl+K).
//!
//! The root view searches applications and extension commands; every other
//! view comes from an extension process (see `backslash_core::host`).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use backslash_core::config::Config;
use backslash_core::host::{Incoming, RpcError, Session};
use backslash_core::protocol::{self as proto, Accessory, ActionKind, Effect, FieldKind, Filtering, FormView, View};
use backslash_core::registry::{CommandEntry, Registry};
use backslash_core::search::{Ranker, Searchable, Usage};
use backslash_core::{calc, paths, runner};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{json, Map, Value};

use crate::apps::{self, App};
use crate::icons::{self, Icon};
use crate::{markdown, theme};

pub struct Options {
    pub hidden: bool,
    pub dev_dir: Option<PathBuf>,
}

const WIDTH: i32 = 780;
const HEIGHT: i32 = 500;
const MAX_ROWS: usize = 60;

pub fn run(options: Options) -> anyhow::Result<()> {
    let dev = options.dev_dir.is_some();
    let app = gtk::Application::builder()
        .application_id(if dev { "app.backslash.Backslash.Devel" } else { "app.backslash.Backslash" })
        .build();

    let launcher: Rc<RefCell<Option<Rc<Launcher>>>> = Rc::default();
    let first_activation = Cell::new(true);
    let dev_dir = options.dev_dir.clone();
    let hidden = options.hidden;

    app.connect_startup({
        let launcher = launcher.clone();
        move |app| {
            if let Some(display) = gdk::Display::default() {
                theme::install(&display);
            }
            launcher.replace(Some(Launcher::new(app, dev_dir.clone())));
        }
    });
    // Every later `backslash` invocation reaches the running instance over
    // D-Bus and lands here, which makes `backslash` itself the toggle.
    app.connect_activate(move |_| {
        let Some(l) = launcher.borrow().clone() else { return };
        if first_activation.replace(false) && hidden {
            return;
        }
        l.toggle();
    });

    let code = app.run_with_args::<&str>(&[]);
    if code != glib::ExitCode::SUCCESS {
        anyhow::bail!("launcher exited with an error");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Model

#[derive(Clone)]
enum Op {
    LaunchApp(String, Option<String>),
    HideApp(String),
    StartCommand(String),
    Copy(String),
    Open(String),
    Callback(String),
}

#[derive(Clone)]
struct UiAction {
    title: String,
    icon: Option<String>,
    shortcut: Option<String>,
    destructive: bool,
    confirm: Option<String>,
    op: Op,
}

impl UiAction {
    fn new(title: impl Into<String>, op: Op) -> Self {
        Self { title: title.into(), icon: None, shortcut: None, destructive: false, confirm: None, op }
    }

    fn icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.into());
        self
    }

    fn from_proto(a: &proto::Action) -> Self {
        let op = match &a.kind {
            ActionKind::Open { target } => Op::Open(target.clone()),
            ActionKind::Copy { text } => Op::Copy(text.clone()),
            ActionKind::Callback => Op::Callback(a.id.clone()),
        };
        Self {
            title: a.title.clone(),
            icon: a.icon.clone(),
            shortcut: a.shortcut.clone(),
            destructive: a.destructive,
            confirm: a.confirm.clone(),
            op,
        }
    }
}

#[derive(Clone, Default, PartialEq)]
enum RowStyle {
    #[default]
    Normal,
    Calculator,
}

#[derive(Clone, Default)]
struct Row {
    key: String,
    title: String,
    subtitle: Option<String>,
    icon: Option<Icon>,
    accessories: Vec<Accessory>,
    kind: Option<String>,
    section: Option<String>,
    actions: Vec<UiAction>,
    style: RowStyle,
}

struct ItemRef<'a>(&'a proto::Item);
impl Searchable for ItemRef<'_> {
    fn title(&self) -> &str {
        &self.0.title
    }
    fn keywords(&self) -> &[String] {
        &self.0.keywords
    }
    fn subtitle(&self) -> Option<&str> {
        self.0.subtitle.as_deref()
    }
}

enum FrameKind {
    Root,
    /// A view served by an extension process.
    Extension {
        session: Rc<Session>,
        view: View,
        title: String,
    },
    /// Arguments of a command, asked before starting it.
    Arguments {
        command_id: String,
        form: FormView,
    },
}

struct Frame {
    kind: FrameKind,
    /// Search text and selected item to restore when navigating back.
    query: String,
    selected: Option<String>,
}

enum FormInput {
    Entry(gtk::Entry),
    Password(gtk::PasswordEntry),
    Text(gtk::TextView),
    Check(gtk::CheckButton),
    Drop(gtk::DropDown, Vec<String>),
}

struct State {
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
}

pub struct Launcher {
    window: gtk::ApplicationWindow,
    entry: gtk::Entry,
    back: gtk::Button,
    spinner: gtk::Spinner,
    pages: gtk::Stack,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    sections: Rc<RefCell<Vec<Option<String>>>>,
    empty_title: gtk::Label,
    empty_subtitle: gtk::Label,
    detail: gtk::Label,
    metadata: gtk::Box,
    form_box: gtk::Box,
    context: gtk::Label,
    toast: gtk::Label,
    primary: gtk::Label,
    actions_hint: gtk::Box,
    action_panel: gtk::Box,
    popover_list: gtk::ListBox,
    popover_actions: RefCell<Vec<UiAction>>,
    state: RefCell<State>,
    dev_dir: Option<PathBuf>,
    /// Set while the code (not the user) changes the search text.
    setting_query: Cell<bool>,
    _hold: gio::ApplicationHoldGuard,
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class(class);
    l.set_xalign(0.0);
    l
}

fn key_hint(text: &str, key: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    b.add_css_class("hint");
    b.append(&gtk::Label::new(Some(text)));
    for k in key.split(' ') {
        b.append(&label(k, "key"));
    }
    b
}

impl Launcher {
    fn new(app: &gtk::Application, dev_dir: Option<PathBuf>) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Backslash")
            .default_width(WIDTH)
            .default_height(HEIGHT)
            .resizable(false)
            .decorated(false)
            .build();
        window.add_css_class("launcher");

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("panel");
        panel.set_overflow(gtk::Overflow::Hidden);

        // Header
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("header");
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.add_css_class("back");
        back.set_valign(gtk::Align::Center);
        back.set_focusable(false);
        back.set_visible(false);
        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_has_frame(false);
        entry.set_hexpand(true);
        entry.set_placeholder_text(Some("Search apps and commands…"));
        let spinner = gtk::Spinner::new();
        header.append(&back);
        header.append(&entry);
        header.append(&spinner);

        let separator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        separator.add_css_class("separator");

        // Pages
        let pages = gtk::Stack::new();
        pages.set_vexpand(true);
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_transition_duration(90);

        let list = gtk::ListBox::new();
        list.add_css_class("results");
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.set_activate_on_single_click(true);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&list).build();
        pages.add_named(&scroller, Some("list"));

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

        let form_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        form_box.add_css_class("form");
        let form_scroll =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&form_box).build();
        pages.add_named(&form_scroll, Some("form"));

        // Footer
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        footer.add_css_class("footer");
        let context = label("Backslash", "context");
        let toast = label("", "toast");
        toast.set_hexpand(true);
        toast.set_xalign(1.0);
        toast.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let primary = label("", "primary");
        let actions_hint = key_hint("Actions", "Ctrl K");
        footer.append(&context);
        footer.append(&toast);
        footer.append(&primary);
        footer.append(&actions_hint);

        panel.append(&header);
        panel.append(&separator);
        panel.append(&pages);
        panel.append(&footer);

        // Action panel: an overlay inside the window rather than a popup
        // surface, so focus never leaves the launcher (popups behave
        // differently on every compositor).
        let action_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        action_panel.add_css_class("actions-panel");
        action_panel.set_halign(gtk::Align::End);
        action_panel.set_valign(gtk::Align::End);
        action_panel.set_margin_end(20);
        action_panel.set_margin_bottom(54);
        action_panel.set_visible(false);
        let popover_list = gtk::ListBox::new();
        popover_list.set_selection_mode(gtk::SelectionMode::Browse);
        action_panel.append(&popover_list);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&panel));
        overlay.add_overlay(&action_panel);
        window.set_child(Some(&overlay));

        let config = Config::load();
        let mut registry = Registry::scan(&paths::extension_dirs());
        if let Some(dir) = &dev_dir {
            registry.add_dir(dir);
        }
        let apps = apps::load(&config.hidden_apps);
        let commands = registry.commands(|id| config.is_enabled(id));
        for (dir, err) in &registry.errors {
            eprintln!("backslash: skipping {}: {err}", dir.display());
        }

        let launcher = Rc::new(Self {
            window,
            entry,
            back,
            spinner,
            pages,
            list,
            scroller,
            sections: Rc::default(),
            empty_title,
            empty_subtitle,
            detail,
            metadata,
            form_box,
            context,
            toast,
            primary,
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
            }),
            dev_dir,
            setting_query: Cell::new(false),
            _hold: app.hold(),
        });
        launcher.connect_signals();
        launcher.render();
        launcher
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

        // Hide when another window takes the focus, like any launcher.
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |w| {
            let Some(l) = weak.upgrade() else { return };
            if !w.is_active() && w.is_visible() && !l.action_panel.is_visible() && l.dev_dir.is_none() {
                l.hide();
            }
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
        theme::apply_scheme(&self.window);
        self.window.present();
        self.entry.grab_focus();
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
            s.frames[0].query.clear();
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
                FrameKind::Extension { view: View::List(_), title, .. } => (title.clone(), "list"),
                FrameKind::Extension { view: View::Detail(_), title, .. } => (title.clone(), "detail"),
                FrameKind::Extension { view: View::Form(_), title, .. } => (title.clone(), "form"),
                FrameKind::Arguments { form, .. } => (form.title.clone().unwrap_or_default(), "form"),
            };
            (s.frames.len(), title, page)
        };
        self.back.set_visible(depth > 1);
        self.context.set_text(&title);
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
                FrameKind::Root => {
                    let rows = root_rows(s, &query);
                    (rows, "Search apps and commands…".to_string(), None)
                }
                FrameKind::Extension { view: View::List(list), .. } => {
                    if s.search.is_none() {
                        self.spinner.set_spinning(list.loading);
                    }
                    let rows = list_rows(&mut s.ranker, list, &query);
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
    }

    fn render_detail(&self) {
        let s = self.state.borrow();
        let Some(Frame { kind: FrameKind::Extension { view: View::Detail(d), .. }, .. }) = s.frames.last() else {
            return;
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
                FrameKind::Extension { view: View::Form(f), .. } | FrameKind::Arguments { form: f, .. } => f.clone(),
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
                FrameKind::Extension { view: View::Form(_), .. } | FrameKind::Arguments { .. } => return Vec::new(),
                FrameKind::Extension { view: View::List(l), .. } => from(&l.actions),
                FrameKind::Root => Vec::new(),
            }
        };
        let mut actions = self.selected_row().map(|r| r.actions).unwrap_or_default();
        actions.extend(view_actions);
        actions
    }

    fn is_form(&self) -> bool {
        matches!(
            self.state.borrow().frames.last().unwrap().kind,
            FrameKind::Extension { view: View::Form(_), .. } | FrameKind::Arguments { .. }
        )
    }

    fn update_footer(&self) {
        if let Some(confirm) = &self.state.borrow().confirm {
            self.primary.add_css_class("confirm");
            self.primary.set_text(&format!(
                "{}   Enter to confirm · Esc to cancel",
                confirm.confirm.as_deref().unwrap_or("Are you sure?")
            ));
            self.actions_hint.set_visible(false);
            return;
        }
        self.primary.remove_css_class("confirm");
        if self.is_form() {
            self.primary.set_text("Submit  Ctrl ↵");
            self.actions_hint.set_visible(false);
            return;
        }
        let actions = self.current_actions();
        self.primary.set_text(&actions.first().map(|a| format!("{}  ↵", a.title)).unwrap_or_default());
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
                if let Some(timer) = self.state.borrow_mut().search_timer.take() {
                    timer.remove();
                }
                let weak = Rc::downgrade(self);
                let timer = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                    if let Some(l) = weak.upgrade() {
                        l.state.borrow_mut().search_timer = None;
                        l.extension_search(session, view_id);
                    }
                });
                self.state.borrow_mut().search_timer = Some(timer);
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
            gdk::Key::Escape => {
                if !self.is_root() {
                    self.pop();
                } else if !self.entry.text().is_empty() {
                    self.entry.set_text("");
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

    /// Starts `extension/command`. Without `arguments`, commands that declare
    /// arguments first show a small form to fill them.
    fn start_command(self: &Rc<Self>, id: &str, arguments: Option<Map<String, Value>>) {
        enum Prepared {
            Started((Session, backslash_core::host::Pending), String),
            NeedArguments(FormView),
            Failed(String),
        }
        let prepared = {
            let s = self.state.borrow();
            match s.registry.resolve(id) {
                None => Prepared::Failed(format!("Unknown command {id}")),
                Some((_, command)) if arguments.is_none() && !command.arguments.is_empty() => {
                    Prepared::NeedArguments(arguments_form(command))
                }
                Some((ext, command)) => {
                    match runner::start(ext, command, &s.config, arguments.unwrap_or_default(), "") {
                        Ok(started) => Prepared::Started(started, command.title.clone()),
                        Err(e) => Prepared::Failed(format!("{e:#}")),
                    }
                }
            }
        };
        let ((session, pending), title) = match prepared {
            Prepared::Started(started, title) => (started, title),
            Prepared::Failed(e) => return self.fail(e),
            Prepared::NeedArguments(form) => {
                return self.push(FrameKind::Arguments { command_id: id.to_string(), form });
            }
        };

        let session = Rc::new(session);
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
                    l.push(FrameKind::Extension { session, view, title });
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
                self.push(FrameKind::Extension { session: session.clone(), view, title });
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
        enum Target {
            Arguments(String),
            Extension(Option<String>),
        }
        let target = match &self.state.borrow().frames.last().unwrap().kind {
            FrameKind::Arguments { command_id, .. } => Target::Arguments(command_id.clone()),
            FrameKind::Extension { view: View::Form(f), .. } => Target::Extension(f.id.clone()),
            _ => return,
        };
        match target {
            Target::Arguments(id) => {
                let parsed = {
                    let s = self.state.borrow();
                    let Some((_, command)) = s.registry.resolve(&id) else { return };
                    runner::parse_arguments(command, &values)
                };
                match parsed {
                    Ok(args) => {
                        self.pop();
                        self.start_command(&id, Some(args));
                    }
                    Err(e) => self.fail(e),
                }
            }
            Target::Extension(view_id) => {
                let id = format!("{}:submit", view_id.unwrap_or_default());
                self.callback(&id, Some(values));
            }
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
        self.entry.grab_focus();
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
}

// ---------------------------------------------------------------------------
// Row builders

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

fn root_rows(s: &mut State, query: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    if let Some(value) = calc::evaluate(query) {
        let result = calc::format(value);
        rows.push(Row {
            key: "calc".into(),
            title: result.clone(),
            subtitle: Some(query.trim().to_string()),
            icon: Some(Icon::Spec("accessories-calculator".into())),
            kind: Some("Calculator".into()),
            actions: vec![
                UiAction::new("Copy Result", Op::Copy(result.clone())).icon("edit-copy"),
                UiAction::new("Copy Expression", Op::Copy(format!("{} = {result}", query.trim()))).icon("edit-copy"),
            ],
            style: RowStyle::Calculator,
            ..Default::default()
        });
    }

    let usage = &s.usage;
    let app_id = |a: &App| format!("app:{}", a.id);
    let cmd_id = |c: &CommandEntry| format!("cmd:{}", c.id);
    let apps = s.ranker.rank(query, &s.apps, Some((usage, &app_id)));
    let commands = s.ranker.rank(query, &s.commands, Some((usage, &cmd_id)));

    let mut scored: Vec<(u32, Row)> = Vec::new();
    for (i, score) in commands {
        let c = &s.commands[i];
        scored.push((
            score,
            Row {
                key: format!("cmd:{}", c.id),
                title: c.title.clone(),
                subtitle: Some(c.subtitle.clone()),
                icon: Some(Icon::Spec(c.icon.clone().unwrap_or_else(|| "system-run".into()))),
                kind: Some("Command".into()),
                section: Some("Commands".into()),
                actions: vec![UiAction::new("Open Command", Op::StartCommand(c.id.clone())).icon("system-run")],
                ..Default::default()
            },
        ));
    }
    for (i, score) in apps {
        let a = &s.apps[i];
        let mut actions =
            vec![UiAction::new("Open Application", Op::LaunchApp(a.id.clone(), None)).icon("document-open")];
        actions.extend(
            a.actions
                .iter()
                .map(|(id, name)| UiAction::new(name.clone(), Op::LaunchApp(a.id.clone(), Some(id.clone())))),
        );
        actions.push(UiAction::new("Hide from Results", Op::HideApp(a.id.clone())).icon("view-conceal"));
        scored.push((
            score,
            Row {
                key: format!("app:{}", a.id),
                title: a.name.clone(),
                subtitle: a.description.clone().filter(|_| !query.is_empty()),
                icon: a.icon.clone(),
                kind: Some("Application".into()),
                section: Some("Applications".into()),
                actions,
                ..Default::default()
            },
        ));
    }
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));

    if query.trim().is_empty() {
        // Suggestions first (what you use), then everything grouped by kind.
        let mut suggestions: Vec<Row> = Vec::new();
        let mut rest: Vec<Row> = Vec::new();
        for (score, mut row) in scored {
            if score > 0 && suggestions.len() < 6 {
                row.section = Some("Suggestions".into());
                suggestions.push(row);
            } else {
                rest.push(row);
            }
        }
        rest.sort_by_key(|r| (r.section.clone(), r.title.to_lowercase()));
        rows.extend(suggestions);
        rows.extend(rest);
    } else {
        rows.extend(scored.into_iter().map(|(_, mut r)| {
            r.section = Some("Results".into());
            r
        }));
    }
    rows.truncate(MAX_ROWS);
    rows
}

fn list_rows(ranker: &mut Ranker, list: &proto::ListView, query: &str) -> Vec<Row> {
    let order: Vec<usize> = if list.filtering == Filtering::Host && !query.trim().is_empty() {
        let refs: Vec<ItemRef> = list.items.iter().map(ItemRef).collect();
        ranker.rank(query, &refs, None).into_iter().map(|(i, _)| i).collect()
    } else {
        (0..list.items.len()).collect()
    };
    let ranked = list.filtering == Filtering::Host && !query.trim().is_empty();
    order
        .into_iter()
        .take(500)
        .map(|i| {
            let item = &list.items[i];
            Row {
                key: item.id.clone(),
                title: item.title.clone(),
                subtitle: item.subtitle.clone(),
                icon: item.icon.clone().map(Icon::Spec),
                accessories: item.accessories.clone(),
                kind: None,
                // Sections only make sense in the extension's own order.
                section: if ranked { None } else { item.section.clone() },
                actions: item.actions.iter().map(UiAction::from_proto).collect(),
                style: RowStyle::Normal,
            }
        })
        .collect()
}

fn row_widget(row: &Row) -> gtk::ListBoxRow {
    let widget = gtk::ListBoxRow::new();
    widget.set_focusable(false);
    if row.style == RowStyle::Calculator {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        b.add_css_class("calculator");
        b.append(&label(&row.title, "result"));
        b.append(&label(row.subtitle.as_deref().unwrap_or_default(), "expression"));
        widget.set_child(Some(&b));
        return widget;
    }
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.append(&icons::widget(row.icon.as_ref(), 28));
    let title = label(&row.title, "title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(48);
    b.append(&title);
    if let Some(subtitle) = &row.subtitle {
        let s = label(subtitle, "subtitle");
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_hexpand(true);
        b.append(&s);
    } else {
        title.set_hexpand(true);
    }
    for a in &row.accessories {
        b.append(&icons::accessory(a));
    }
    if let Some(kind) = &row.kind {
        b.append(&label(kind, "kind"));
    }
    widget.set_child(Some(&b));
    widget
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

fn pretty_accel(accel: &str) -> String {
    gtk::accelerator_parse(accel)
        .map(|(k, m)| gtk::accelerator_get_label(k, m).to_string())
        .unwrap_or_else(|| accel.to_string())
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
