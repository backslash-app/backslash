//! What the window shows: rows, actions, previews, and the navigation stack.

use std::path::PathBuf;
use std::rc::Rc;

use backslash_core::config::Theme;
use backslash_core::host::Session;
use backslash_core::protocol::{self as proto, Accessory, ActionKind, FormView, View};
use backslash_core::search::Searchable;
use serde_json::{Map, Value};

use crate::icons::Icon;

/// Views implemented by the launcher itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Clipboard,
    Files,
    Windows,
    Theme,
    Settings,
    Store,
}

impl Builtin {
    pub fn title(self) -> &'static str {
        match self {
            Self::Clipboard => "Clipboard History",
            Self::Files => "Search Files",
            Self::Windows => "Switch Windows",
            Self::Theme => "Theme",
            Self::Settings => "Settings",
            Self::Store => "Extensions",
        }
    }

    /// Shown when the view has no rows and nothing is typed.
    pub fn empty(self) -> (&'static str, &'static str) {
        match self {
            Self::Clipboard => ("Nothing copied yet", "Text you copy will show up here."),
            Self::Files => ("Search your files", "Type at least two letters. Recent files appear here."),
            Self::Windows => ("No open windows", ""),
            Self::Theme | Self::Settings => ("", ""),
            Self::Store => ("No extensions found", "Check your connection, then open the store again."),
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Clipboard => "Search copied text…",
            Self::Files => "Search files in your home folder…",
            Self::Windows => "Search open windows…",
            Self::Theme => "Choose a theme…",
            Self::Settings => "Search settings…",
            Self::Store => "Search extensions…",
        }
    }
}

#[derive(Clone)]
pub enum Op {
    LaunchApp(String, Option<String>),
    HideApp(String),
    StartCommand(String),
    Copy(String),
    /// Copies a snippet after expanding its placeholders.
    Snippet(String),
    Open(String),
    Callback(String),
    /// Replaces the search text (quicklink keywords).
    FillQuery(String),
    OpenBuiltin(Builtin),
    ClipCopy(u64),
    ClipDelete(u64),
    ClipPin(u64),
    ClipClear,
    Reveal(PathBuf),
    FocusWindow(String),
    SetTheme(Theme),
    SetAutostart(bool),
    SetClipboard(bool),
    EditHotkey,
    ToggleExtension(String),
    ConfigureExtension(String),
    EditQuicklink(Option<usize>),
    DeleteQuicklink(usize),
    EditSnippet(Option<usize>),
    DeleteSnippet(usize),
    UnhideApp(String),
    /// Allow the extension of the approval screen, then run its command.
    Approve,
    /// Store: show what an extension will be allowed to do, then install.
    ReviewInstall(String),
    InstallEntry(String),
    UninstallEntry(String),
    Back,
}

#[derive(Clone)]
pub struct UiAction {
    pub title: String,
    pub icon: Option<String>,
    pub shortcut: Option<String>,
    pub destructive: bool,
    pub confirm: Option<String>,
    pub op: Op,
}

impl UiAction {
    pub fn new(title: impl Into<String>, op: Op) -> Self {
        Self { title: title.into(), icon: None, shortcut: None, destructive: false, confirm: None, op }
    }

    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn shortcut(mut self, accel: &str) -> Self {
        self.shortcut = Some(accel.into());
        self
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn confirm(mut self, question: &str) -> Self {
        self.confirm = Some(question.into());
        self
    }

    pub fn from_proto(a: &proto::Action) -> Self {
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
pub enum RowStyle {
    #[default]
    Normal,
    /// Big result card (calculator, conversions).
    Answer,
}

/// The right-hand panel describing the selected row.
#[derive(Clone, Default)]
pub struct Preview {
    pub icon: Option<Icon>,
    pub title: String,
    pub subtitle: Option<String>,
    pub facts: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Clone, Default)]
pub struct Row {
    pub key: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    /// Accent of the icon tile: blue, orange, green, purple, yellow, red, teal.
    pub tint: Option<&'static str>,
    pub accessories: Vec<Accessory>,
    pub section: Option<String>,
    pub actions: Vec<UiAction>,
    pub style: RowStyle,
    pub preview: Option<Preview>,
}

impl Row {
    /// The preview to show: the explicit one, or one derived from the row.
    pub fn preview(&self) -> Preview {
        if let Some(p) = &self.preview {
            return p.clone();
        }
        Preview {
            icon: self.icon.clone(),
            title: self.title.clone(),
            subtitle: self.subtitle.clone(),
            facts: self
                .accessories
                .iter()
                .filter_map(|a| a.text.clone().or_else(|| a.tag.clone()))
                .map(|v| (String::new(), v))
                .collect(),
            body: None,
        }
    }
}

const TINTS: [&str; 7] = ["blue", "orange", "green", "purple", "yellow", "red", "teal"];

/// A stable tint for a name, so an extension keeps its color.
pub fn tint_for(name: &str) -> &'static str {
    let hash = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    TINTS[hash as usize % TINTS.len()]
}

pub struct ItemRef<'a>(pub &'a proto::Item);
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

/// Root view filter, cycled with Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Apps,
    Commands,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::All, Filter::Apps, Filter::Commands];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Apps => "Apps",
            Self::Commands => "Commands",
        }
    }
}

/// What a form built by the launcher itself is for.
#[derive(Clone)]
pub enum FormPurpose {
    /// Arguments of a command, asked before starting it.
    Arguments {
        command_id: String,
    },
    /// Preferences of an extension; `then_run` starts a command after saving.
    Preferences {
        extension_id: String,
        then_run: Option<String>,
    },
    Quicklink {
        index: Option<usize>,
    },
    Snippet {
        index: Option<usize>,
    },
    Hotkey,
}

/// The command an extension view belongs to, to restart it (hot reload).
pub struct Launched {
    pub command_id: String,
    pub arguments: Map<String, Value>,
}

pub enum FrameKind {
    Root,
    /// A view served by an extension process.
    Extension {
        session: Rc<Session>,
        view: View,
        title: String,
        launched: Rc<Launched>,
    },
    /// A form handled by the launcher.
    Form {
        purpose: FormPurpose,
        form: FormView,
    },
    /// "This extension wants to…": shown before its first run.
    Approval {
        extension_id: String,
        command_id: String,
        arguments: Option<Map<String, Value>>,
    },
    Builtin(Builtin),
    /// Store: "Install X? It will be allowed to…".
    ConfirmInstall {
        id: String,
    },
}

pub struct Frame {
    pub kind: FrameKind,
    /// Search text and selected item to restore when navigating back.
    pub query: String,
    pub selected: Option<String>,
}

pub enum FormInput {
    Entry(gtk::Entry),
    Password(gtk::PasswordEntry),
    Text(gtk::TextView),
    Check(gtk::CheckButton),
    Drop(gtk::DropDown, Vec<String>),
}
