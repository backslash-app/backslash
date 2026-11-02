//! Answers in the root search: `mode = "answer"` commands (a calculator, a
//! conversion) get each query and say what to show on top of the results.
//!
//! Each one runs in its own process for as long as the window is open: it is
//! started when the window shows, so the first keystroke is answered quickly,
//! and stopped when it hides. A new query cancels the previous one. Only
//! extensions the user enabled and approved answer (the permission prompt
//! says that they see what is typed), with their required preferences set.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use backslash_core::host::{Incoming, RpcError, Session};
use backslash_core::manifest::CommandMode;
use backslash_core::protocol as proto;
use backslash_core::runner;
use gtk::glib;
use gtk::prelude::*;

use super::background::can_run_unattended;
use super::model::Row;
use super::{rows, Launcher, State};

#[derive(Default)]
pub(super) struct Answers {
    /// Running answer commands, by command id.
    sessions: HashMap<String, Rc<Session>>,
    /// The request each command is working on.
    pending: HashMap<String, u64>,
    /// Commands whose process died: not started again until the window
    /// shows again, rather than on every keystroke.
    broken: HashSet<String>,
    /// The latest rows of each command, with the query they answer.
    results: HashMap<String, (String, Vec<Row>)>,
}

/// Ids (`extension/command`) of the answer commands allowed to run, in the
/// order their rows are shown.
fn allowed(s: &State) -> Vec<String> {
    let mut out = Vec::new();
    for ext in &s.registry.extensions {
        let m = &ext.manifest;
        if !m.commands.iter().any(|c| c.mode == CommandMode::Answer) || !can_run_unattended(&s.config, m) {
            continue;
        }
        for c in m.commands.iter().filter(|c| c.mode == CommandMode::Answer) {
            out.push(format!("{}/{}", m.extension.id, c.name));
        }
    }
    out
}

/// The rows answering `query`, in command order.
pub(super) fn current_rows(s: &State, query: &str) -> Vec<Row> {
    let query = query.trim();
    allowed(s)
        .iter()
        .filter_map(|id| s.answers.results.get(id))
        .filter(|(asked, _)| asked == query)
        .flat_map(|(_, rows)| rows.iter().cloned())
        .collect()
}

impl Launcher {
    /// Starts the answer commands that are not running yet (window shown).
    pub(super) fn warm_answers(self: &Rc<Self>) {
        let ids = self.prune_answers();
        for id in ids {
            self.answer_session(&id);
        }
    }

    /// Stops them all (window hidden). Their last rows stay for when the
    /// window comes back as it was.
    pub(super) fn stop_answers(&self) {
        let mut s = self.state.borrow_mut();
        s.answers.sessions.clear();
        s.answers.pending.clear();
        s.answers.broken.clear();
    }

    /// Sends the root search query to every answer command.
    pub(super) fn ask_answers(self: &Rc<Self>, query: &str) {
        let query = query.trim();
        for id in self.prune_answers() {
            self.ask_answer(&id, query);
        }
    }

    /// Forgets the commands that may no longer answer (disabled, removed);
    /// returns the others.
    fn prune_answers(&self) -> Vec<String> {
        let mut s = self.state.borrow_mut();
        let ids = allowed(&s);
        let a = &mut s.answers;
        a.sessions.retain(|id, _| ids.contains(id));
        a.pending.retain(|id, _| ids.contains(id));
        a.results.retain(|id, _| ids.contains(id));
        ids
    }

    /// The running process of an answer command, started if needed.
    fn answer_session(self: &Rc<Self>, id: &str) -> Option<Rc<Session>> {
        if let Some(session) = self.state.borrow().answers.sessions.get(id) {
            return Some(session.clone());
        }
        if self.state.borrow().answers.broken.contains(id) {
            return None;
        }
        let spawned = {
            let s = self.state.borrow();
            let (ext, command) = s.registry.resolve(id)?;
            runner::spawn_answers(ext, command)
        };
        let session = match spawned {
            Ok(session) => Rc::new(session),
            Err(e) => {
                eprintln!("backslash: {id}: {e:#}");
                self.state.borrow_mut().answers.broken.insert(id.to_string());
                return None;
            }
        };
        self.serve_answers(&session);
        self.state.borrow_mut().answers.sessions.insert(id.to_string(), session.clone());
        Some(session)
    }

    fn ask_answer(self: &Rc<Self>, id: &str, query: &str) {
        let previous = self.state.borrow_mut().answers.pending.remove(id);
        if let (Some(request), Some(session)) = (previous, self.state.borrow().answers.sessions.get(id)) {
            session.cancel(request);
        }
        if query.is_empty() {
            self.state.borrow_mut().answers.results.remove(id);
            return;
        }
        let Some(session) = self.answer_session(id) else { return };
        let asked = {
            let s = self.state.borrow();
            let Some((ext, command)) = s.registry.resolve(id) else { return };
            runner::ask(&session, command, &s.config, ext, query)
        };
        let pending = match asked {
            Ok(pending) => pending,
            Err(e) => return self.answer_failed(id, &e.to_string()),
        };
        let request = pending.id;
        self.state.borrow_mut().answers.pending.insert(id.to_string(), request);
        let weak = Rc::downgrade(self);
        let id = id.to_string();
        let query = query.to_string();
        glib::spawn_future_local(async move {
            let reply = pending.get().await;
            let Some(l) = weak.upgrade() else { return };
            if l.state.borrow().answers.pending.get(&id) != Some(&request) {
                return;
            }
            l.state.borrow_mut().answers.pending.remove(&id);
            let items = reply.and_then(|v| {
                serde_json::from_value::<Vec<proto::Item>>(v).map_err(|e| RpcError::new(-32700, e.to_string()))
            });
            match items {
                Ok(items) => {
                    let rows = {
                        let s = l.state.borrow();
                        let Some((ext, command)) = s.registry.resolve(&id) else { return };
                        let icon = command.icon.clone().or_else(|| ext.manifest.extension.icon.clone());
                        rows::answer_rows(&items, &id, &command.title, icon, &ext.manifest.permissions.net)
                    };
                    l.state.borrow_mut().answers.results.insert(id, (query.clone(), rows));
                    let showing = l.is_root() && l.window.is_visible() && l.entry.text().trim() == query;
                    if showing {
                        l.render();
                    }
                }
                Err(e) if e.code == RpcError::CANCELLED => {}
                Err(e) => l.answer_failed(&id, &e.message),
            }
        });
    }

    /// An answer that failed shows nothing; a dead process is not restarted
    /// before the window shows again.
    fn answer_failed(&self, id: &str, error: &str) {
        eprintln!("backslash: {id}: {error}");
        let mut s = self.state.borrow_mut();
        s.answers.results.remove(id);
        let dead = s.answers.sessions.get(id).is_some_and(|session| session.exit_status().is_some());
        if dead {
            s.answers.sessions.remove(id);
            s.answers.broken.insert(id.to_string());
        }
    }

    /// Answers what an answer command may ask: nothing but storage (served
    /// by the host) and logs, since it runs behind the user's typing.
    fn serve_answers(self: &Rc<Self>, session: &Rc<Session>) {
        let weak = Rc::downgrade(session);
        let incoming = session.incoming.clone();
        glib::spawn_future_local(async move {
            while let Ok(msg) = incoming.recv().await {
                let Some(session) = weak.upgrade() else { break };
                match msg {
                    Incoming::Log(line) => eprintln!("[{}] {line}", session.extension_id),
                    Incoming::Request { id, method, .. } => session
                        .respond(id, Err(RpcError::new(-32001, format!("{method} is not available while answering")))),
                    Incoming::Notification { .. } | Incoming::Exited(_) => {}
                }
            }
        });
    }
}
