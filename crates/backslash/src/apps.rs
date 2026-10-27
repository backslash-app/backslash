//! Installed applications, straight from GIO: it already implements the XDG
//! rules (data dir precedence, `NoDisplay`, `OnlyShowIn`, `TryExec`, Flatpak and
//! Snap exports, localized names) and launches apps properly (field codes,
//! terminal apps, D-Bus activation, startup notification).

use gtk::gio;
use gtk::prelude::*;

use backslash_core::search::Searchable;

use crate::icons::Icon;

#[derive(Clone)]
pub struct App {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    pub icon: Option<Icon>,
    /// `StartupWMClass`, to match open windows to their application.
    pub wm_class: Option<String>,
    /// Where it was installed from: Flatpak, Snap, local or system.
    pub source: &'static str,
    pub info: gio::AppInfo,
    /// Desktop actions such as "New Private Window": `(action id, label)`.
    pub actions: Vec<(String, String)>,
}

impl Searchable for App {
    fn title(&self) -> &str {
        &self.name
    }
    fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

pub fn load(hidden: &[String]) -> Vec<App> {
    let mut apps: Vec<App> = gio::AppInfo::all()
        .into_iter()
        .filter(|info| info.should_show())
        .filter_map(|info| {
            let id = info.id()?.to_string();
            if hidden.iter().any(|h| *h == id || format!("{h}.desktop") == id) {
                return None;
            }
            let desktop = info.downcast_ref::<gio::DesktopAppInfo>();
            let mut keywords: Vec<String> =
                desktop.map(|d| d.keywords().iter().map(|k| k.to_string()).collect()).unwrap_or_default();
            if let Some(generic) = desktop.and_then(|d| d.generic_name()) {
                keywords.push(generic.to_string());
            }
            // Let "code" find "Visual Studio Code" via its executable, too.
            if let Some(exe) = info.executable().file_name() {
                keywords.push(exe.to_string_lossy().into_owned());
            }
            let actions = desktop
                .map(|d| d.list_actions().iter().map(|a| (a.to_string(), d.action_name(a).to_string())).collect())
                .unwrap_or_default();
            let file = desktop.and_then(|d| d.filename()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let source = if file.contains("/flatpak/") {
                "Flatpak"
            } else if file.contains("/snapd/") {
                "Snap"
            } else if file.starts_with(&*backslash_core::paths::home().to_string_lossy()) {
                "Installed for you"
            } else {
                "System package"
            };
            Some(App {
                id,
                name: info.display_name().to_string(),
                description: info.description().map(|d| d.to_string()),
                generic_name: desktop.and_then(|d| d.generic_name()).map(|g| g.to_string()),
                wm_class: desktop.and_then(|d| d.startup_wm_class()).map(|c| c.to_string()),
                source,
                keywords,
                icon: info.icon().map(Icon::Gio),
                info,
                actions,
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub fn launch(app: &App, action: Option<&str>) -> Result<(), String> {
    let context = gtk::gdk::Display::default().map(|d| d.app_launch_context());
    match (action, app.info.downcast_ref::<gio::DesktopAppInfo>()) {
        (Some(action), Some(desktop)) => {
            desktop.launch_action(action, context.as_ref());
            Ok(())
        }
        _ => app.info.launch(&[], context.as_ref()).map_err(|e| e.to_string()),
    }
}

/// Opens a URL, or a file/folder path, with the default handler.
pub fn open(target: &str) -> Result<(), String> {
    let uri = if target.starts_with('/') || target.starts_with('~') {
        gio::File::for_path(backslash_core::paths::expand(target)).uri().to_string()
    } else {
        target.to_string()
    };
    let context = gtk::gdk::Display::default().map(|d| d.app_launch_context());
    gio::AppInfo::launch_default_for_uri(&uri, context.as_ref()).map_err(|e| e.to_string())
}
