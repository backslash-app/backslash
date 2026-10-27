//! Windows on KDE Plasma (Wayland): KWin lets apps run scripts, not list
//! windows. Backslash loads a tiny script that reports the windows back over
//! D-Bus, or activates one, then unloads it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use backslash_core::system::windows::{parse_kwin, WindowInfo};
use gtk::gio;
use gtk::glib::{self, Variant};
use gtk::prelude::*;

const PATH: &str = "/app/backslash/KWin";
const IFACE: &str = "app.backslash.KWin";
const XML: &str = r#"<node><interface name="app.backslash.KWin">
  <method name="Windows"><arg type="s" direction="in" name="json"/></method>
</interface></node>"#;

/// Lists windows as JSON and sends it to SERVICE (replaced before loading).
const LIST_SCRIPT: &str = r#"
const all = workspace.windowList ? workspace.windowList() : workspace.clientList();
const out = all
  .filter(w => !w.skipTaskbar && (w.normalWindow === undefined || w.normalWindow))
  .map(w => ({
    id: String(w.internalId),
    app_id: w.desktopFileName || String(w.resourceClass || ""),
    title: w.caption || "",
    focus: !!w.active,
    workspace: (w.desktops && w.desktops.length) ? w.desktops[0].name : (w.desktop > 0 ? "Desktop " + w.desktop : ""),
  }));
callDBus("SERVICE", "/app/backslash/KWin", "app.backslash.KWin", "Windows", JSON.stringify(out));
"#;

const ACTIVATE_SCRIPT: &str = r#"
const all = workspace.windowList ? workspace.windowList() : workspace.clientList();
const target = all.find(w => String(w.internalId) === ID);
if (target) {
  if ("activeWindow" in workspace) workspace.activeWindow = target;
  else workspace.activeClient = target;
}
"#;

pub struct Kwin {
    bus: gio::DBusConnection,
    reply: Rc<RefCell<Option<async_channel::Sender<String>>>>,
}

impl Kwin {
    /// Ready on KDE Plasma under Wayland; `None` elsewhere.
    pub fn new() -> Option<Rc<Self>> {
        if !backslash_core::system::windows::is_kde_wayland() {
            return None;
        }
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
        let info = gio::DBusNodeInfo::for_xml(XML).ok()?.lookup_interface(IFACE)?;
        let reply: Rc<RefCell<Option<async_channel::Sender<String>>>> = Rc::default();
        let sink = reply.clone();
        bus.register_object(PATH, &info)
            .method_call(move |_, _, _, _, method, params, invocation| {
                if method == "Windows" {
                    if let (Some(tx), Some(json)) = (sink.borrow_mut().take(), params.child_value(0).str()) {
                        let _ = tx.try_send(json.to_string());
                    }
                }
                invocation.return_value(None);
            })
            .build()
            .ok()?;
        Some(Rc::new(Self { bus, reply }))
    }

    pub async fn list(&self) -> Result<Vec<WindowInfo>, String> {
        let service = self.bus.unique_name().ok_or("no D-Bus name")?.to_string();
        let (tx, rx) = async_channel::bounded(1);
        self.reply.replace(Some(tx));
        self.run(&LIST_SCRIPT.replace("SERVICE", &service)).await?;
        let json = glib::future_with_timeout(Duration::from_secs(2), rx.recv())
            .await
            .map_err(|_| "KWin did not answer".to_string())?
            .map_err(|e| e.to_string())?;
        parse_kwin(&json).map_err(|e| e.to_string())
    }

    pub async fn activate(&self, id: &str) -> Result<(), String> {
        let id = serde_json::to_string(id).map_err(|e| e.to_string())?;
        self.run(&ACTIVATE_SCRIPT.replace("ID", &id)).await
    }

    /// Loads a script into KWin, runs it, and unloads it.
    async fn run(&self, source: &str) -> Result<(), String> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let name = format!("backslash-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let dir = glib::user_runtime_dir();
        let file = dir.join(format!("{name}.js"));
        std::fs::write(&file, source).map_err(|e| e.to_string())?;
        let result = self.load_and_run(&file.display().to_string(), &name).await;
        let _ = self
            .call("/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(name.as_str(),).to_variant()))
            .await;
        let _ = std::fs::remove_file(&file);
        result
    }

    async fn load_and_run(&self, file: &str, name: &str) -> Result<(), String> {
        let loaded =
            self.call("/Scripting", "org.kde.kwin.Scripting", "loadScript", Some(&(file, name).to_variant())).await?;
        let id = loaded.child_value(0).get::<i32>().ok_or("unexpected answer from KWin")?;
        if id < 0 {
            return Err("KWin refused the script".into());
        }
        // Plasma 6 and Plasma 5 put loaded scripts at different paths.
        if self.call(&format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", None).await.is_ok() {
            return Ok(());
        }
        self.call(&format!("/{id}"), "org.kde.kwin.Script", "run", None).await.map(|_| ())
    }

    async fn call(&self, path: &str, iface: &str, method: &str, args: Option<&Variant>) -> Result<Variant, String> {
        self.bus
            .call_future(Some("org.kde.KWin"), path, iface, method, args, None, gio::DBusCallFlags::NONE, 2000)
            .await
            .map_err(|e| e.to_string())
    }
}
