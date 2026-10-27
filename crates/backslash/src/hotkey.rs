//! Global shortcut through the XDG desktop portal (`GlobalShortcuts`, supported
//! by KDE, GNOME 48+, Hyprland and others). The desktop owns the key: it
//! asks the user to confirm once, then calls us when it is pressed.
//!
//! Where the portal is missing, binding `backslash` to a key in the desktop
//! settings keeps working: a second launch toggles the running instance.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib::{self, Variant, VariantTy};
use gtk::prelude::*;

const DEST: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const SHORTCUT_ID: &str = "toggle";

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

fn dict(entries: &[(&str, Variant)]) -> Variant {
    let d = glib::VariantDict::new(None);
    for (k, v) in entries {
        d.insert_value(k, v);
    }
    d.end()
}

/// Calls a portal method returning a Request object and waits for its
/// `Response` signal. Returns the results dictionary.
async fn request(
    bus: &gio::DBusConnection,
    method: &str,
    args: Vec<Variant>,
    token: &str,
) -> Result<glib::VariantDict, String> {
    let sender = bus.unique_name().ok_or("no D-Bus name")?;
    let sender = sender.trim_start_matches(':').replace('.', "_");
    let request_path = format!("{PATH}/request/{sender}/{token}");

    let (tx, rx) = async_channel::bounded::<(u32, Variant)>(1);
    // Subscribe before calling, or a fast portal could answer first.
    #[allow(deprecated)]
    let subscription = bus.signal_subscribe(
        Some(DEST),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&request_path),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            let code = params.child_value(0).get::<u32>().unwrap_or(2);
            let _ = tx.try_send((code, params.child_value(1)));
        },
    );
    let call = bus
        .call_future(
            Some(DEST),
            PATH,
            IFACE,
            method,
            Some(&Variant::tuple_from_iter(args)),
            None,
            gio::DBusCallFlags::NONE,
            -1,
        )
        .await;
    let result = match call {
        Err(e) => Err(e.to_string()),
        Ok(_) => match rx.recv().await {
            Ok((0, results)) => Ok(glib::VariantDict::new(Some(&results))),
            Ok((1, _)) => Err("cancelled by the user".into()),
            Ok((_, _)) => Err("refused by the desktop".into()),
            Err(_) => Err("no answer".into()),
        },
    };
    #[allow(deprecated)]
    bus.signal_unsubscribe(subscription);
    result
}

/// Registers the shortcut and calls `on_activate` each time it is pressed.
/// `on_status` receives a short human-readable outcome (for logs).
pub fn register(app_id: &str, accel: &str, on_activate: impl Fn() + 'static, on_status: impl Fn(String) + 'static) {
    let app_id = app_id.to_string();
    let trigger = to_portal_trigger(accel);
    glib::spawn_future_local(async move {
        match setup(&app_id, &trigger, on_activate).await {
            Ok(()) => on_status(format!("global shortcut {trigger} registered through the desktop portal")),
            Err(e) => on_status(format!(
                "global shortcut portal unavailable ({e}); bind `backslash` to a key in your desktop settings"
            )),
        }
    });
}

async fn setup(app_id: &str, trigger: &str, on_activate: impl Fn() + 'static) -> Result<(), String> {
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

    let token = format!("backslash{}", std::process::id());
    let session_token = format!("{token}s");
    let created = request(
        &bus,
        "CreateSession",
        vec![dict(&[("handle_token", token.to_variant()), ("session_handle_token", session_token.to_variant())])],
        &token,
    )
    .await?;
    let session = created
        .lookup_value("session_handle", None)
        .and_then(|v| v.str().map(str::to_owned))
        .ok_or("no session handle")?;

    let shortcut = Variant::tuple_from_iter([
        SHORTCUT_ID.to_variant(),
        dict(&[("description", "Show or hide Backslash".to_variant()), ("preferred_trigger", trigger.to_variant())]),
    ]);
    let shortcuts = Variant::array_from_iter_with_type(VariantTy::new("(sa{sv})").expect("valid type"), [shortcut]);
    let session_path = glib::variant::ObjectPath::try_from(session.clone()).map_err(|e| e.to_string())?;
    let bind_token = format!("{token}b");
    request(
        &bus,
        "BindShortcuts",
        vec![session_path.to_variant(), shortcuts, "".to_variant(), dict(&[("handle_token", bind_token.to_variant())])],
        &bind_token,
    )
    .await?;

    // Debounce: some portals send Activated twice (press and repeat).
    let last = Rc::new(Cell::new(0i64));
    #[allow(deprecated)]
    bus.signal_subscribe(
        Some(DEST),
        Some(IFACE),
        Some("Activated"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |_, _, _, _, _, params| {
            let id = params.child_value(1).str().map(str::to_owned).unwrap_or_default();
            let now = glib::monotonic_time();
            if id == SHORTCUT_ID && now - last.get() > 250_000 {
                last.set(now);
                on_activate();
            }
        },
    );
    // Keep the connection (and the subscription) alive for the process lifetime.
    std::mem::forget(bus);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn triggers() {
        assert_eq!(super::to_portal_trigger("Super+Space"), "LOGO+space");
        assert_eq!(super::to_portal_trigger("Ctrl+Alt+K"), "CTRL+ALT+k");
    }
}
