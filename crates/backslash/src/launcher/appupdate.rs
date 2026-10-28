//! Updating Backslash itself: the background check, the "new version" pill
//! in the footer, and the update screen.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use backslash_core::protocol as proto;
use backslash_core::selfupdate::{self, Install, Release};
use gtk::prelude::*;
use gtk::{gio, glib};

use super::model::{FrameKind, Op, UiAction};
use super::Launcher;

#[derive(Default)]
pub(super) struct AppUpdate {
    /// A newer release than the running one.
    pub release: Option<Release>,
    pub checking: bool,
    pub installing: bool,
    /// A check finished (successfully or not) since the start.
    pub checked: bool,
    pub error: Option<String>,
    install: Option<Install>,
}

impl AppUpdate {
    /// How this copy was installed, worked out once.
    pub fn install(&mut self) -> &Install {
        self.install.get_or_insert_with(selfupdate::detect)
    }
}

/// The pill that replaces nothing: it sits after "Backslash" in the footer
/// while a new version is available.
pub(super) fn pill() -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&crate::glyphs::Glyph::new("bs-download", 12));
    content.append(&gtk::Label::new(None));
    let button = gtk::Button::builder().child(&content).css_classes(["update-pill"]).visible(false).build();
    button.set_tooltip_text(Some("See what's new and update"));
    button.set_focusable(false);
    button.set_valign(gtk::Align::Center);
    button
}

fn markdown(u: &mut AppUpdate) -> String {
    let current = selfupdate::CURRENT;
    if u.installing {
        return "# Updating…\n\nDownloading and checking the new version. Backslash restarts by itself when it's done."
            .into();
    }
    let Some(release) = u.release.clone() else {
        if u.checking || !u.checked {
            return format!("# Checking for updates…\n\nYou have Backslash {current}.");
        }
        if let Some(e) = &u.error {
            return format!("# Couldn't check for updates\n\n{e}\n\nYou have Backslash {current}.");
        }
        return format!("# Backslash is up to date\n\nYou have the latest version, {current}.");
    };
    let how = u.install().describe();
    let notes = release.notes.trim();
    let notes = if notes.is_empty() { String::new() } else { format!("\n\n---\n\n{notes}") };
    format!("# Backslash {} is available\n\nYou have {current}. {how}{notes}", release.version)
}

impl Launcher {
    /// Checks GitHub for a new version on a worker thread. `manual` checks
    /// run even when automatic checks are off.
    pub(super) fn check_app_update(self: &Rc<Self>, manual: bool) {
        {
            let mut s = self.state.borrow_mut();
            if s.app_update.checking || s.app_update.installing || (!manual && !s.config.check_updates) {
                return;
            }
            s.app_update.checking = true;
        }
        if manual {
            self.render();
        }
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(selfupdate::check().map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("the check stopped".into()));
            let Some(l) = weak.upgrade() else { return };
            {
                let mut s = l.state.borrow_mut();
                let u = &mut s.app_update;
                u.checking = false;
                u.checked = true;
                match result {
                    Ok(release) => {
                        u.release = release;
                        u.error = None;
                    }
                    Err(e) => u.error = Some(e),
                }
            }
            l.refresh_update_pill();
            crate::tray::set_update(l.state.borrow().app_update.release.as_ref().map(|r| r.version.clone()));
            if l.window.is_visible() && !l.action_panel.is_visible() && !l.is_form() {
                l.render();
            }
        });
    }

    /// Checks now, then every six hours.
    pub(super) fn watch_app_updates(self: &Rc<Self>) {
        self.check_app_update(false);
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(6 * 3600, move || {
            let Some(l) = weak.upgrade() else { return glib::ControlFlow::Break };
            l.check_app_update(false);
            glib::ControlFlow::Continue
        });
    }

    pub(super) fn open_app_update(self: &Rc<Self>) {
        let at_update = matches!(self.state.borrow().frames.last().map(|f| &f.kind), Some(FrameKind::AppUpdate));
        if !at_update {
            self.push(FrameKind::AppUpdate);
        }
        let stale = {
            let s = self.state.borrow();
            s.app_update.release.is_none() && !s.app_update.checking
        };
        if stale {
            self.check_app_update(true);
        }
    }

    /// Shows or hides the footer pill.
    pub(super) fn refresh_update_pill(&self) {
        let version = self.state.borrow().app_update.release.as_ref().map(|r| r.version.clone());
        let show = version.is_some() && self.is_root();
        if let Some(text) = self.update_pill.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
            text.set_text(&format!("Backslash {} is available", version.unwrap_or_default()));
        }
        if show {
            self.update_pill.add_css_class("available");
        } else {
            self.update_pill.remove_css_class("available");
        }
        self.update_pill.set_visible(show && !self.toast.is_visible());
    }

    pub(super) fn app_update_detail(&self) -> proto::DetailView {
        let mut s = self.state.borrow_mut();
        proto::DetailView { title: Some("Update".into()), markdown: markdown(&mut s.app_update), ..Default::default() }
    }

    pub(super) fn app_update_actions(&self) -> Vec<UiAction> {
        let mut s = self.state.borrow_mut();
        let u = &mut s.app_update;
        if u.installing || u.checking {
            return Vec::new();
        }
        let Some(release) = u.release.clone() else {
            return vec![
                UiAction::new("Check Again", Op::CheckAppUpdate).icon("bs-refresh"),
                UiAction::new("Open Release Notes", Op::Open(selfupdate::RELEASES_PAGE.into())).icon("bs-external"),
            ];
        };
        let mut actions = Vec::new();
        if u.install().can_update() {
            actions.push(UiAction::new("Update and Restart", Op::InstallAppUpdate).icon("bs-download"));
        }
        actions.push(UiAction::new("Open Release Page", Op::Open(release.url)).icon("bs-external"));
        actions.push(UiAction::new("Later", Op::Back).icon("bs-x"));
        actions
    }

    /// Downloads, verifies and installs the new version, then restarts.
    pub(super) fn install_app_update(self: &Rc<Self>) {
        let (release, install) = {
            let mut s = self.state.borrow_mut();
            let u = &mut s.app_update;
            let Some(release) = u.release.clone() else { return };
            if u.installing || !u.install().can_update() {
                return;
            }
            u.installing = true;
            (release, u.install().clone())
        };
        // Its path before the update replaces the file.
        let exe = std::env::current_exe().ok();
        self.spinner.set_spinning(true);
        self.render();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = selfupdate::download_verified(&release, &install)
                .and_then(|file| selfupdate::apply(&install, &file).map(|_| file))
                .map(|file| {
                    let _ = std::fs::remove_file(file);
                    release.version
                });
            let _ = tx.send_blocking(result.map_err(|e| format!("{e:#}")));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("the update stopped".into()));
            let Some(l) = weak.upgrade() else { return };
            l.spinner.set_spinning(false);
            l.state.borrow_mut().app_update.installing = false;
            match result {
                Ok(version) => {
                    l.show_toast(&format!("Updated to {version}. Restarting…"), proto::ToastStyle::Success);
                    let weak = Rc::downgrade(&l);
                    glib::timeout_add_local_once(Duration::from_millis(900), move || {
                        if let Some(l) = weak.upgrade() {
                            l.restart(exe);
                        }
                    });
                }
                Err(e) => {
                    l.fail(e);
                    l.render();
                }
            }
        });
    }

    /// Starts the (new) binary once this process has exited, then quits.
    fn restart(&self, exe: Option<PathBuf>) {
        let exe = exe.map(|p| {
            // After a package update, /proc/self/exe points at the deleted file.
            let s = p.to_string_lossy().trim_end_matches(" (deleted)").to_string();
            PathBuf::from(s)
        });
        if let Some(exe) = exe {
            use std::os::unix::process::CommandExt;
            let _ = std::process::Command::new("sh")
                .args(["-c", "sleep 1; exec \"$0\" --hidden"])
                .arg(exe)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn();
        }
        self.quit();
    }

    pub(super) fn quit(&self) {
        crate::tray::stop();
        if let Some(app) = self.window.application() {
            app.quit();
        } else if let Some(app) = gio::Application::default() {
            app.quit();
        }
    }
}

impl Launcher {
    /// The tray icon, unless turned off in the settings.
    pub(super) fn start_tray(self: &Rc<Self>) {
        if !self.state.borrow().config.tray {
            return;
        }
        let Some(bus) = self.window.application().and_then(|a| a.dbus_connection()) else { return };
        let weak = Rc::downgrade(self);
        crate::tray::start(&bus, move |event| {
            let Some(l) = weak.upgrade() else { return };
            match event {
                crate::tray::Event::Toggle => l.toggle(),
                crate::tray::Event::Settings => l.open_settings(None),
                crate::tray::Event::Update => {
                    if !l.window.is_visible() {
                        l.show();
                    }
                    l.open_app_update();
                }
                crate::tray::Event::Quit => l.quit(),
            }
        });
        let version = self.state.borrow().app_update.release.as_ref().map(|r| r.version.clone());
        crate::tray::set_update(version);
    }
}
