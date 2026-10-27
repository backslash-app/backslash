//! `oauth/*` requests: the launcher signs in on the extension's behalf.
//!
//! Opening the browser usually takes the focus, which hides the launcher and
//! stops the extension. The sign-in carries on regardless; once it succeeds
//! the launcher comes back and starts the command again, which now finds its
//! token.

use std::rc::Rc;

use backslash_core::host::{RpcError, Session};
use backslash_core::oauth::{self, Provider, SignIn};
use backslash_core::protocol as proto;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

use super::model::FrameKind;
use super::Launcher;
use crate::apps;

fn error(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(-32000, e.to_string())
}

/// Runs blocking work off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    gio::spawn_blocking(f).await.unwrap_or_else(|_| Err(anyhow::anyhow!("the sign-in crashed")))
}

impl Launcher {
    pub(super) fn oauth_request(self: &Rc<Self>, session: &Rc<Session>, id: Value, method: &str, params: Value) {
        let extension = session.extension_id.clone();
        let provider = match serde_json::from_value::<Provider>(params["provider"].clone()) {
            Ok(p) => p,
            Err(e) => return session.respond(id, Err(RpcError::new(-32602, format!("invalid provider: {e}")))),
        };
        let hosts = self
            .state
            .borrow()
            .registry
            .find(&extension)
            .map(|e| e.manifest.permissions.net.clone())
            .unwrap_or_default();
        if let Err(e) = provider.check(&hosts) {
            return session.respond(id, Err(error(format!("{e:#}"))));
        }
        if method == "oauth/signOut" {
            let result = oauth::sign_out(&extension, &provider.id).map(|_| Value::Null).map_err(error);
            return session.respond(id, result);
        }
        let interactive = params["interactive"].as_bool().unwrap_or(true);
        // What to start again if the extension stops during the sign-in.
        let launched = self.state.borrow().frames.iter().find_map(|f| match &f.kind {
            FrameKind::Extension { session: s, launched, .. } if Rc::ptr_eq(s, session) => Some(launched.clone()),
            _ => None,
        });
        let weak = Rc::downgrade(self);
        let session_weak = Rc::downgrade(session);
        glib::spawn_future_local(async move {
            let (ext, p) = (extension.clone(), provider.clone());
            let reply = |result: Result<Value, RpcError>| {
                if let Some(s) = session_weak.upgrade() {
                    s.respond(id.clone(), result);
                }
            };
            match blocking(move || oauth::quiet_token(&ext, &p)).await {
                Ok(Some(token)) => return reply(Ok(token.into())),
                Err(e) => return reply(Err(error(format!("{e:#}")))),
                Ok(None) if !interactive => return reply(Ok(Value::Null)),
                Ok(None) => {}
            }
            let sign_in = match SignIn::start(&extension, &provider) {
                Ok(s) => s,
                Err(e) => return reply(Err(error(format!("{e:#}")))),
            };
            let service = provider.title.clone().unwrap_or_else(|| provider.id.clone());
            if let Some(l) = weak.upgrade() {
                l.show_toast(&format!("Sign in to {service} in your browser…"), proto::ToastStyle::Info);
            }
            if let Err(e) = apps::open(&sign_in.url) {
                return reply(Err(error(format!("cannot open the browser: {e}"))));
            }
            let result = blocking(move || sign_in.finish()).await;
            let Some(l) = weak.upgrade() else { return };
            if session_weak.upgrade().is_some() {
                // Still running: hand over the token and come back to it.
                l.window.present();
                if result.is_ok() {
                    l.show_toast(&format!("Signed in to {service}"), proto::ToastStyle::Success);
                }
                return reply(result.map(Value::from).map_err(|e| error(format!("{e:#}"))));
            }
            match (result, launched) {
                (Ok(_), Some(launched)) => {
                    if !l.window.is_visible() {
                        l.show();
                    }
                    l.start_command(&launched.command_id, Some(launched.arguments.clone()));
                }
                (Ok(_), None) => l.flash_and_hide(&format!("Signed in to {service}")),
                (Err(e), _) => {
                    if !l.window.is_visible() {
                        l.show();
                    }
                    l.fail(format!("Sign-in to {service} failed: {e:#}"));
                }
            }
        });
    }
}
