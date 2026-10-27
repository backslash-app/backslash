//! Global shortcut through the XDG desktop portal (`GlobalShortcuts`, supported
//! by KDE, GNOME 48+, Hyprland and others). The desktop owns the key: it
//! asks the user to confirm once, then calls us when it is pressed.
//!
//! Where the portal is missing, binding `backslash` to a key in the desktop
//! settings keeps working: a second launch toggles the running instance.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use gtk::gio;
use gtk::glib::{self, Variant, VariantTy};
use gtk::prelude::*;

use crate::portal::{dict, request, DEST, PATH};

const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// `Super+Space` → `LOGO+space`, the format of the XDG shortcuts specification.
pub fn to_portal_trigger(accel: &str) -> String {
    accel
        .split('+')
        .map(|part| match part.trim().to_lowercase().as_str() {
            "super" | "meta" | "logo" | "win" => "LOGO".to_string(),
            "ctrl" | "control" => "CTRL".to_string(),
            "alt" => "ALT".to_string(),
            "shift" => "SHIFT".to_string(),
            key => key.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// A shortcut to ask the desktop for.
#[derive(Clone)]
pub struct Binding {
    /// Stable id: the desktop remembers the user's choice under it.
    pub id: String,
    pub description: String,
    /// `Super+Space` style.
    pub accel: String,
}

/// Same shortcut, whatever its description says.
impl PartialEq for Binding {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.accel == other.accel
    }
}

struct Active {
    bus: gio::DBusConnection,
    session: String,
    subscription: gio::SignalSubscriptionId,
}

/// Where the shortcuts stand, for the welcome screen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Nothing to register (no shortcut set, or a development instance).
    Disabled,
    Pending,
    Active,
    /// The desktop has no GlobalShortcuts portal, or refused.
    Unavailable,
}

/// The shortcuts registered with the portal. Binding again replaces them
/// (a new portal session, as a session's shortcuts can only be bound once).
#[derive(Default)]
pub struct Shortcuts {
    active: RefCell<Option<Active>>,
    /// Bumped on each `bind`, so a slow, outdated setup gives up.
    generation: Cell<u64>,
    bound: RefCell<Vec<Binding>>,
    failed: Cell<bool>,
}

impl Shortcuts {
    pub fn status(&self) -> Status {
        if self.active.borrow().is_some() {
            Status::Active
        } else if self.failed.get() {
            Status::Unavailable
        } else if self.bound.borrow().is_empty() {
            Status::Disabled
        } else {
            Status::Pending
        }
    }

    /// Registers `bindings`, calling `on_activate(id)` when one is pressed.
    /// `on_status` receives a short human-readable outcome (for logs).
    pub fn bind(
        self: &Rc<Self>,
        app_id: &str,
        bindings: Vec<Binding>,
        on_activate: Rc<dyn Fn(&str)>,
        on_status: impl Fn(String) + 'static,
    ) {
        // Unchanged, or the portal already failed for these.
        if *self.bound.borrow() == bindings {
            return;
        }
        self.bound.replace(bindings.clone());
        self.failed.set(false);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if let Some(old) = self.active.take() {
            close(old);
        }
        if bindings.is_empty() {
            return;
        }
        let this = self.clone();
        let app_id = app_id.to_string();
        glib::spawn_future_local(async move {
            let summary = bindings.iter().map(|b| to_portal_trigger(&b.accel)).collect::<Vec<_>>().join(", ");
            match setup(&app_id, &bindings, on_activate).await {
                Ok(active) if this.generation.get() == generation => {
                    this.active.replace(Some(active));
                    this.failed.set(false);
                    on_status(format!("global shortcuts {summary} registered through the desktop portal"));
                }
                Ok(stale) => close(stale),
                Err(e) => {
                    this.failed.set(this.generation.get() == generation);
                    on_status(format!(
                        "global shortcut portal unavailable ({e}); bind `backslash` to a key in your desktop settings"
                    ))
                }
            }
        });
    }
}

fn close(active: Active) {
    #[allow(deprecated)]
    active.bus.signal_unsubscribe(active.subscription);
    active.bus.call(
        Some(DEST),
        &active.session,
        "org.freedesktop.portal.Session",
        "Close",
        None,
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
        |result| {
            if let Err(e) = result {
                eprintln!("backslash: could not close the previous shortcuts session: {e}");
            }
        },
    );
}

async fn setup(app_id: &str, bindings: &[Binding], on_activate: Rc<dyn Fn(&str)>) -> Result<Active, String> {
    let bus = gio::bus_get_future(gio::BusType::Session).await.map_err(|e| e.to_string())?;

    // Host (non-Flatpak) apps tell the portal who they are; older portals
    // lack this method, which is fine.
    let _ = bus
        .call_future(
            Some(DEST),
            PATH,
            "org.freedesktop.host.portal.Registry",
            "Register",
            Some(&Variant::tuple_from_iter([app_id.to_variant(), dict(&[])])),
            None,
            gio::DBusCallFlags::NONE,
            2000,
        )
        .await;

    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let token = format!("backslash{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let session_token = format!("{token}s");
    let created = request(
        &bus,
        IFACE,
        "CreateSession",
        vec![dict(&[("handle_token", token.to_variant()), ("session_handle_token", session_token.to_variant())])],
        &token,
    )
    .await?;
    let session = created
        .lookup_value("session_handle", None)
        .and_then(|v| v.str().map(str::to_owned))
        .ok_or("no session handle")?;

    let shortcuts = Variant::array_from_iter_with_type(
        VariantTy::new("(sa{sv})").expect("valid type"),
        bindings.iter().map(|b| {
            Variant::tuple_from_iter([
                b.id.to_variant(),
                dict(&[
                    ("description", b.description.to_variant()),
                    ("preferred_trigger", to_portal_trigger(&b.accel).to_variant()),
                ]),
            ])
        }),
    );
    let session_path = glib::variant::ObjectPath::try_from(session.clone()).map_err(|e| e.to_string())?;
    let bind_token = format!("{token}b");
    request(
        &bus,
        IFACE,
        "BindShortcuts",
        vec![session_path.to_variant(), shortcuts, "".to_variant(), dict(&[("handle_token", bind_token.to_variant())])],
        &bind_token,
    )
    .await?;

    // Debounce: some portals send Activated twice (press and repeat).
    let last = Rc::new(Cell::new(0i64));
    let ours = session.clone();
    #[allow(deprecated)]
    let subscription = bus.signal_subscribe(
        Some(DEST),
        Some(IFACE),
        Some("Activated"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            if params.child_value(0).str() != Some(ours.as_str()) {
                return;
            }
            let id = params.child_value(1).str().map(str::to_owned).unwrap_or_default();
            let now = glib::monotonic_time();
            if now - last.get() > 250_000 {
                last.set(now);
                on_activate(&id);
            }
        },
    );
    Ok(Active { bus, session, subscription })
}

#[cfg(test)]
mod tests {
    #[test]
    fn triggers() {
        assert_eq!(super::to_portal_trigger("Super+Space"), "LOGO+space");
        assert_eq!(super::to_portal_trigger("Ctrl+Alt+K"), "CTRL+ALT+k");
    }
}
