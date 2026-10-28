//! Commands that run without being opened: `no-view` commands with an
//! `interval`, and `menu-bar` commands, whose menu lives in the system tray.
//!
//! Every half minute the launcher looks at the installed commands and starts
//! the ones that are due, with `launchType: "background"`. Only extensions the
//! user enabled and approved run this way (the permission prompt names these
//! commands), with their required preferences set. A tray menu is shown from
//! its first run; clicking one of its items runs the command again, finds the
//! item by its id and calls its action.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backslash_core::host::{Incoming, RpcError, Session};
use backslash_core::manifest::{CommandMode, Runtime};
use backslash_core::protocol::{ActionKind, Effect, LaunchType, MenuView, View};
use backslash_core::{approvals, paths, runner, runtime, sandbox, store};
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{json, Map, Value};

use super::model::{FrameKind, Launched};
use super::{set_clipboard, Launcher};
use crate::{apps, tray_items};

/// How often due commands are looked for.
const TICK: Duration = Duration::from_secs(30);
/// A background run that takes longer is stopped.
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Background {
    /// When each command last started on its own (seconds since the epoch),
    /// kept across restarts so a restart doesn't run everything again.
    last: BTreeMap<String, u64>,
    running: HashSet<String>,
    /// Started at least once since Backslash started (a menu that failed
    /// waits for its interval, or for the user, rather than every tick).
    tried: HashSet<String>,
    /// The menu each `menu-bar` command shows, by command id.
    menus: HashMap<String, MenuView>,
    started: bool,
}

fn last_path() -> PathBuf {
    paths::state_dir().join("background-runs.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A command that may run on its own, and when.
struct Scheduled {
    id: String,
    menu: bool,
    interval: Option<Duration>,
}

impl Launcher {
    /// Starts looking for due commands, now and every [`TICK`].
    pub(super) fn start_background(self: &Rc<Self>) {
        {
            let mut s = self.state.borrow_mut();
            if s.background.started {
                return;
            }
            s.background.started = true;
            s.background.last = store::read_json(&last_path());
        }
        self.background_tick();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(TICK, move || match weak.upgrade() {
            Some(l) => {
                l.background_tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }

    /// The commands allowed to run on their own right now.
    fn scheduled(&self) -> Vec<Scheduled> {
        let s = self.state.borrow();
        let mut out = Vec::new();
        // `backslash dev` runs next to the usual launcher: only its extension here.
        let dev = self.dev_dir.as_ref().and_then(|d| d.canonicalize().ok());
        for ext in &s.registry.extensions {
            let m = &ext.manifest;
            if dev.as_ref().is_some_and(|d| ext.dir.canonicalize().ok().as_ref() != Some(d)) {
                continue;
            }
            if !s.config.is_enabled(&m.extension.id) {
                continue;
            }
            let approved = approvals::requested(m).is_empty() || approvals::is_approved(m);
            let configured = s.config.preferences(m).1.is_empty();
            let can_start = match m.runtime {
                Runtime::Deno => runtime::is_installed(),
                _ => sandbox::is_available(),
            };
            if !(approved && configured && can_start) {
                continue;
            }
            for c in &m.commands {
                let menu = c.mode == CommandMode::MenuBar;
                if menu || c.interval().is_some() {
                    out.push(Scheduled { id: format!("{}/{}", m.extension.id, c.name), menu, interval: c.interval() });
                }
            }
        }
        out
    }

    fn background_tick(self: &Rc<Self>) {
        let scheduled = self.scheduled();
        // Menus of commands that went away (uninstalled, disabled).
        let keep: HashSet<&str> = scheduled.iter().filter(|c| c.menu).map(|c| c.id.as_str()).collect();
        for key in tray_items::shown() {
            if !keep.contains(key.as_str()) {
                tray_items::remove(&key);
                self.state.borrow_mut().background.menus.remove(&key);
            }
        }
        let due: Vec<String> = {
            let s = self.state.borrow();
            let b = &s.background;
            scheduled
                .into_iter()
                .filter(|c| !b.running.contains(&c.id))
                .filter(|c| {
                    let elapsed = now().saturating_sub(b.last.get(&c.id).copied().unwrap_or(0));
                    let interval_passed = c.interval.is_some_and(|i| elapsed >= i.as_secs());
                    // A menu shows from the start, then follows its interval.
                    interval_passed || (c.menu && !b.tried.contains(&c.id))
                })
                .map(|c| c.id)
                .collect()
        };
        for id in due {
            self.run_in_background(&id);
        }
    }

    /// Runs `id` without a window: its menu goes to the tray, a no-view
    /// command's message to the log.
    pub(super) fn run_in_background(self: &Rc<Self>, id: &str) {
        let started = {
            let s = self.state.borrow();
            let Some((ext, command)) = s.registry.resolve(id) else { return };
            runner::start(ext, command, &s.config, Map::new(), "", LaunchType::Background)
        };
        let (session, pending) = match started {
            Ok(started) => started,
            Err(e) => return eprintln!("backslash: {id} in the background: {e:#}"),
        };
        {
            let mut s = self.state.borrow_mut();
            s.background.running.insert(id.to_string());
            s.background.tried.insert(id.to_string());
            s.background.last.insert(id.to_string(), now());
            if let Err(e) = store::write_json(&last_path(), &s.background.last) {
                eprintln!("backslash: cannot save background runs: {e}");
            }
        }
        let session = Rc::new(session);
        self.serve_background(&session, id);
        let weak = Rc::downgrade(self);
        let id = id.to_string();
        glib::spawn_future_local(async move {
            let reply = glib::future_with_timeout(TIMEOUT, pending.get()).await;
            let Some(l) = weak.upgrade() else { return };
            l.state.borrow_mut().background.running.remove(&id);
            let effect = match reply {
                Ok(reply) => reply.and_then(|v| {
                    serde_json::from_value::<Effect>(v).map_err(|e| RpcError::new(-32700, e.to_string()))
                }),
                Err(_) => Err(RpcError::new(RpcError::CANCELLED, "timed out")),
            };
            match effect {
                Ok(Effect::Push { view: View::Menu(menu) }) => l.show_menu(&id, menu),
                Ok(Effect::Close { message: Some(m) }) => eprintln!("[{id}] {m}"),
                Ok(_) => {}
                Err(e) => eprintln!("backslash: {id} in the background: {e}"),
            }
            session.kill();
        });
    }

    /// Answers what a command may ask while nobody looks at it.
    fn serve_background(self: &Rc<Self>, session: &Rc<Session>, command_id: &str) {
        let weak = Rc::downgrade(self);
        let session_weak = Rc::downgrade(session);
        let incoming = session.incoming.clone();
        let command_id = command_id.to_string();
        glib::spawn_future_local(async move {
            while let Ok(msg) = incoming.recv().await {
                let (Some(l), Some(session)) = (weak.upgrade(), session_weak.upgrade()) else { break };
                let Incoming::Request { id, method, mut params } = msg else {
                    if let Incoming::Log(line) = msg {
                        eprintln!("[{}] {line}", session.extension_id);
                    }
                    continue;
                };
                let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let result = match method.as_str() {
                    "command/metadata" => {
                        let subtitle = params.get("subtitle").and_then(Value::as_str).map(str::to_owned);
                        l.state.borrow_mut().command_meta.set(&command_id, subtitle);
                        Ok(Value::Null)
                    }
                    "toast" => {
                        eprintln!("[{command_id}] {} {}", text("title"), text("message"));
                        Ok(Value::Null)
                    }
                    "clipboard/write" => {
                        let concealed = params.get("concealed").and_then(Value::as_bool).unwrap_or(false);
                        set_clipboard(&l.window, &text("text"), concealed);
                        Ok(Value::Null)
                    }
                    "open" => apps::open(&text("target")).map(|_| Value::Null).map_err(|e| RpcError::new(-32000, e)),
                    "close" => Ok(Value::Null),
                    "oauth/token" | "oauth/signOut" => {
                        // Signing in needs the user: a saved token or nothing.
                        params["interactive"] = json!(false);
                        l.oauth_request(&session, id, &method, params);
                        continue;
                    }
                    _ => Err(RpcError::new(-32001, format!("{method} is not available in the background"))),
                };
                session.respond(id, result);
            }
        });
    }

    fn show_menu(self: &Rc<Self>, id: &str, menu: MenuView) {
        let Some(bus) = self.window.application().and_then(|a| a.dbus_connection()) else { return };
        let (title, icon) = {
            let s = self.state.borrow();
            let Some((ext, command)) = s.registry.resolve(id) else { return };
            (command.title.clone(), command.icon.clone().or_else(|| ext.manifest.extension.icon.clone()))
        };
        let weak = Rc::downgrade(self);
        let key = id.to_string();
        tray_items::show(&bus, id, &title, &menu, icon.as_deref(), move |path| {
            if let Some(l) = weak.upgrade() {
                l.menu_clicked(&key, path);
            }
        });
        self.state.borrow_mut().background.menus.insert(id.to_string(), menu);
    }

    /// An item of a tray menu was clicked.
    fn menu_clicked(self: &Rc<Self>, id: &str, path: Vec<String>) {
        let action = {
            let s = self.state.borrow();
            s.background.menus.get(id).and_then(|m| m.find(&path)).and_then(|i| i.action.clone())
        };
        let Some(action) = action else { return };
        match action.kind {
            ActionKind::Open { target } => {
                if let Err(e) = apps::open(&target) {
                    self.notify(&action.title, &e);
                }
            }
            // No app to paste into from the tray: both copy.
            ActionKind::Copy { text, concealed } | ActionKind::Paste { text, concealed } => {
                set_clipboard(&self.window, &text, concealed)
            }
            ActionKind::Callback => self.run_menu_action(id, path),
            ActionKind::Submenu { .. } => {}
        }
    }

    /// Runs the command again, as the user asked for it, and calls the action
    /// of the item at `path` in the fresh menu. A view it returns opens in
    /// the launcher; then the menu is refreshed.
    fn run_menu_action(self: &Rc<Self>, id: &str, path: Vec<String>) {
        let started = {
            let s = self.state.borrow();
            let Some((ext, command)) = s.registry.resolve(id) else { return };
            runner::start(ext, command, &s.config, Map::new(), "", LaunchType::UserInitiated)
                .map(|started| (started, command.title.clone()))
        };
        let ((session, pending), title) = match started {
            Ok(started) => started,
            Err(e) => return self.notify(id, &format!("{e:#}")),
        };
        let session = Rc::new(session);
        self.listen(&session);
        let weak = Rc::downgrade(self);
        let id = id.to_string();
        glib::spawn_future_local(async move {
            let parse = |v: Result<Value, RpcError>| {
                v.and_then(|v| serde_json::from_value::<Effect>(v).map_err(|e| RpcError::new(-32700, e.to_string())))
            };
            let menu = match parse(pending.get().await) {
                Ok(Effect::Push { view: View::Menu(menu) }) => menu,
                Ok(_) => return,
                Err(e) => {
                    if let Some(l) = weak.upgrade() {
                        l.notify(&title, &e.message);
                    }
                    return;
                }
            };
            let Some(l) = weak.upgrade() else { return };
            let action = menu.find(&path).and_then(|i| i.action.clone());
            l.show_menu(&id, menu);
            let Some(action) = action else { return };
            let pending = match session.request("action", json!({ "action": action.id })) {
                Ok(p) => p,
                Err(e) => return l.notify(&title, &e.to_string()),
            };
            let effect = parse(pending.get().await);
            let Some(l) = weak.upgrade() else { return };
            match effect {
                Ok(Effect::Push { view } | Effect::Replace { view }) if !matches!(view, View::Menu(_)) => {
                    l.show();
                    l.reset_to_root();
                    let launched = Rc::new(Launched { command_id: id.clone(), arguments: Map::new() });
                    l.push(FrameKind::Extension { session: session.clone(), view, title, launched });
                }
                Ok(Effect::Close { message: Some(m) }) => l.notify(&title, &m),
                Ok(_) => {}
                Err(e) => l.notify(&title, &e.message),
            }
            l.run_in_background(&id);
        });
    }

    /// A desktop notification, for what happens while the window is hidden.
    fn notify(&self, title: &str, body: &str) {
        let Some(app) = self.window.application() else { return };
        let n = gio::Notification::new(title);
        n.set_body(Some(body));
        app.send_notification(None, &n);
    }
}
