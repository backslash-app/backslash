//! Calls to the XDG desktop portal (D-Bus), shared by the global shortcut and
//! paste-into-app features.

use gtk::gio;
use gtk::glib::{self, Variant};

pub const DEST: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";

pub fn dict(entries: &[(&str, Variant)]) -> Variant {
    let d = glib::VariantDict::new(None);
    for (k, v) in entries {
        d.insert_value(k, v);
    }
    d.end()
}

/// Calls a portal method returning a Request object and waits for its
/// `Response` signal. Returns the results dictionary.
pub async fn request(
    bus: &gio::DBusConnection,
    iface: &str,
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
            iface,
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
