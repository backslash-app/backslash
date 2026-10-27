//! Open windows, for the window switcher. Wayland has no common protocol for
//! this, so each compositor with an IPC gets a backend; X11 goes through
//! `wmctrl`. GNOME goes through a small Shell extension (Backslash's own, or
//! "Window Calls"), called with `gdbus`. KDE needs a KWin script and a D-Bus
//! callback, so it lives in the launcher.

use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use super::which;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    /// Backend-specific handle passed back to [`Backend::focus`].
    pub id: String,
    /// Wayland app id or X11 class, used to find the application's icon.
    pub app_id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Hyprland,
    Sway,
    Niri,
    /// GNOME Shell with a window extension.
    Gnome(GnomeApi),
    X11,
}

/// The GNOME Shell extension answering on D-Bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GnomeApi {
    /// `backslash@backslash.app`, shipped with Backslash.
    Backslash,
    /// The "Window Calls" extension, if already installed.
    WindowCalls,
}

impl GnomeApi {
    fn path(self) -> &'static str {
        match self {
            Self::Backslash => "/app/backslash/Windows",
            Self::WindowCalls => "/org/gnome/Shell/Extensions/Windows",
        }
    }
    fn interface(self) -> &'static str {
        match self {
            Self::Backslash => "app.backslash.Windows",
            Self::WindowCalls => "org.gnome.Shell.Extensions.Windows",
        }
    }
    fn call(self, method: &str, args: &[&str]) -> Result<String> {
        let method = format!("{}.{method}", self.interface());
        let mut argv =
            vec!["call", "--session", "--timeout", "2", "--dest", "org.gnome.Shell", "--object-path", self.path()];
        argv.extend(["--method", &method]);
        argv.extend(args);
        run("gdbus", &argv)
    }
}

/// Is this a GNOME session?
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_lowercase().contains("gnome"))
}

/// The GNOME window extension that answers, if any.
pub fn gnome_api() -> Option<GnomeApi> {
    if !is_gnome() || which(&["gdbus"]).is_none() {
        return None;
    }
    [GnomeApi::Backslash, GnomeApi::WindowCalls].into_iter().find(|api| api.call("List", &[]).is_ok())
}

impl Backend {
    /// The backend for the current session, if supported.
    pub fn detect() -> Option<Self> {
        let env = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        if env("HYPRLAND_INSTANCE_SIGNATURE") && which(&["hyprctl"]).is_some() {
            Some(Self::Hyprland)
        } else if env("SWAYSOCK") && which(&["swaymsg"]).is_some() {
            Some(Self::Sway)
        } else if env("NIRI_SOCKET") && which(&["niri"]).is_some() {
            Some(Self::Niri)
        } else if let Some(api) = gnome_api() {
            Some(Self::Gnome(api))
        } else if !env("WAYLAND_DISPLAY") && env("DISPLAY") && which(&["wmctrl"]).is_some() {
            Some(Self::X11)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Hyprland => "Hyprland",
            Self::Sway => "Sway",
            Self::Niri => "niri",
            Self::Gnome(GnomeApi::Backslash) => "GNOME (Backslash extension)",
            Self::Gnome(GnomeApi::WindowCalls) => "GNOME (Window Calls extension)",
            Self::X11 => "X11 (wmctrl)",
        }
    }

    pub fn list(self) -> Result<Vec<WindowInfo>> {
        match self {
            Self::Hyprland => parse_hyprland(&run("hyprctl", &["-j", "clients"])?),
            Self::Sway => parse_sway(&run("swaymsg", &["-t", "get_tree", "-r"])?),
            Self::Niri => parse_niri(&run("niri", &["msg", "-j", "windows"])?),
            Self::Gnome(api) => parse_gnome(&gvariant_string(&api.call("List", &[])?)?),
            Self::X11 => Ok(parse_wmctrl(&run("wmctrl", &["-lx"])?)),
        }
    }

    pub fn focus(self, id: &str) -> Result<()> {
        match self {
            Self::Hyprland => run("hyprctl", &["dispatch", "focuswindow", &format!("address:{id}")]),
            Self::Sway => run("swaymsg", &[&format!("[con_id={id}]"), "focus"]),
            Self::Niri => run("niri", &["msg", "action", "focus-window", "--id", id]),
            Self::Gnome(api) => api.call("Activate", &[id]),
            Self::X11 => run("wmctrl", &["-ia", id]),
        }
        .map(|_| ())
    }
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program).args(args).output().with_context(|| format!("cannot run {program}"))?;
    if !out.status.success() {
        bail!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn parse_hyprland(json: &str) -> Result<Vec<WindowInfo>> {
    let clients: Vec<Value> = serde_json::from_str(json)?;
    let mut windows: Vec<(i64, WindowInfo)> = clients
        .iter()
        .filter(|c| c.get("mapped").and_then(Value::as_bool).unwrap_or(true))
        .map(|c| {
            let history = c.get("focusHistoryID").and_then(Value::as_i64).unwrap_or(i64::MAX);
            let info = WindowInfo {
                id: str_of(c, "address"),
                app_id: str_of(c, "class"),
                title: str_of(c, "title"),
                workspace: c.get("workspace").map(|w| str_of(w, "name")).filter(|n| !n.is_empty()),
                focused: history == 0,
            };
            (history, info)
        })
        .collect();
    windows.sort_by_key(|(h, _)| *h);
    Ok(windows.into_iter().map(|(_, w)| w).collect())
}

pub fn parse_sway(json: &str) -> Result<Vec<WindowInfo>> {
    fn visit(node: &Value, workspace: Option<&str>, out: &mut Vec<WindowInfo>) {
        let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
        let workspace = if kind == "workspace" { node.get("name").and_then(Value::as_str) } else { workspace };
        let is_window = matches!(kind, "con" | "floating_con") && node.get("pid").is_some();
        if is_window {
            let app_id = node
                .get("app_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| node.pointer("/window_properties/class").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default();
            out.push(WindowInfo {
                id: node.get("id").map(|v| v.to_string()).unwrap_or_default(),
                app_id,
                title: str_of(node, "name"),
                workspace: workspace.filter(|w| *w != "__i3_scratch").map(str::to_owned),
                focused: node.get("focused").and_then(Value::as_bool).unwrap_or(false),
            });
        }
        for key in ["nodes", "floating_nodes"] {
            for child in node.get(key).and_then(Value::as_array).into_iter().flatten() {
                visit(child, workspace, out);
            }
        }
    }
    let tree: Value = serde_json::from_str(json)?;
    let mut out = Vec::new();
    visit(&tree, None, &mut out);
    Ok(out)
}

pub fn parse_niri(json: &str) -> Result<Vec<WindowInfo>> {
    let windows: Vec<Value> = serde_json::from_str(json)?;
    Ok(windows
        .iter()
        .map(|w| WindowInfo {
            id: w.get("id").map(|v| v.to_string()).unwrap_or_default(),
            app_id: str_of(w, "app_id"),
            title: str_of(w, "title"),
            workspace: w.get("workspace_id").filter(|v| !v.is_null()).map(|v| v.to_string()),
            focused: w.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect())
}

/// The string in gdbus's output `('…',)`, unescaped.
pub fn gvariant_string(out: &str) -> Result<String> {
    let t = out.trim();
    let inner = t
        .strip_prefix("('")
        .and_then(|r| r.strip_suffix("',)"))
        .or_else(|| t.strip_prefix("(\"").and_then(|r| r.strip_suffix("\",)")))
        .with_context(|| format!("unexpected gdbus output: {}", t.chars().take(80).collect::<String>()))?;
    let mut s = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            s.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => s.push('\n'),
            Some('t') => s.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                s.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(other) => s.push(other),
            None => {}
        }
    }
    Ok(s)
}

/// The JSON of the Backslash or Window Calls GNOME extension.
pub fn parse_gnome(json: &str) -> Result<Vec<WindowInfo>> {
    let windows: Vec<Value> = serde_json::from_str(json)?;
    Ok(windows
        .iter()
        // Window Calls lists every window; keep the normal ones.
        .filter(|w| w.get("window_type").and_then(Value::as_i64).is_none_or(|t| t == 0))
        .map(|w| {
            let app_id = [str_of(w, "app_id"), str_of(w, "wm_class"), str_of(w, "class")]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            WindowInfo {
                id: w.get("id").map(|v| v.to_string()).unwrap_or_default(),
                app_id,
                title: str_of(w, "title"),
                workspace: w.get("workspace").and_then(Value::as_i64).filter(|n| *n >= 0).map(|n| (n + 1).to_string()),
                focused: w.get("focus").and_then(Value::as_bool).unwrap_or(false),
            }
        })
        .collect())
}

/// What Backslash's KWin script reports (KDE Plasma).
pub fn parse_kwin(json: &str) -> Result<Vec<WindowInfo>> {
    let windows: Vec<Value> = serde_json::from_str(json)?;
    Ok(windows
        .iter()
        .map(|w| WindowInfo {
            id: str_of(w, "id"),
            app_id: str_of(w, "app_id"),
            title: str_of(w, "title"),
            workspace: Some(str_of(w, "workspace")).filter(|s| !s.is_empty()),
            focused: w.get("focus").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect())
}

/// Is this KDE Plasma on Wayland (where wmctrl can't see the windows)?
pub fn is_kde_wayland() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_lowercase().contains("kde"))
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// `wmctrl -lx`: `0x03a00003  0 firefox.Firefox  host  Title with spaces`.
pub fn parse_wmctrl(text: &str) -> Vec<WindowInfo> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let id = parts.next()?.to_string();
            let desktop = parts.next()?;
            let class = parts.next()?;
            let _host = parts.next()?;
            let title = parts.collect::<Vec<_>>().join(" ");
            // Desktop -1 holds panels and docks.
            (desktop != "-1").then(|| WindowInfo {
                id,
                app_id: class.split('.').next_back().unwrap_or(class).to_string(),
                title,
                workspace: Some((desktop.parse::<i64>().unwrap_or(0) + 1).to_string()),
                focused: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod gnome_tests {
    use super::*;

    #[test]
    fn gdbus_strings() {
        assert_eq!(gvariant_string("('[{\"id\":1}]',)\n").unwrap(), "[{\"id\":1}]");
        assert_eq!(gvariant_string(r"('It\'s \u00e9t\u00e9',)").unwrap(), "It's été");
        assert!(gvariant_string("()").is_err());
    }

    #[test]
    fn kwin_windows() {
        let w = parse_kwin(
            r#"[{"id":"{5a1}","app_id":"org.kde.dolphin","title":"Home","focus":true,"workspace":"Desktop 1"}]"#,
        )
        .unwrap();
        assert_eq!(
            (w[0].id.as_str(), w[0].app_id.as_str(), w[0].workspace.as_deref()),
            ("{5a1}", "org.kde.dolphin", Some("Desktop 1"))
        );
    }

    #[test]
    fn gnome_windows() {
        let json = r#"[{"id":12,"app_id":"org.gnome.Nautilus","wm_class":"org.gnome.Nautilus","title":"Home","focus":true,"workspace":0},
                       {"id":13,"app_id":"","wm_class":"firefox","title":"Mozilla","focus":false,"workspace":1},
                       {"id":14,"wm_class":"gnome-shell","window_type":3,"title":"popup"}]"#;
        let w = parse_gnome(json).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].id.as_str(), w[0].app_id.as_str(), w[0].focused), ("12", "org.gnome.Nautilus", true));
        assert_eq!((w[1].app_id.as_str(), w[1].workspace.as_deref()), ("firefox", Some("2")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_sorted_by_focus_history() {
        let json = r#"[
          {"address":"0xa","mapped":true,"class":"kitty","title":"zsh","workspace":{"id":2,"name":"2"},"focusHistoryID":1},
          {"address":"0xb","mapped":true,"class":"firefox","title":"Docs","workspace":{"id":1,"name":"1"},"focusHistoryID":0},
          {"address":"0xc","mapped":false,"class":"ghost","title":"","workspace":{"id":1,"name":"1"},"focusHistoryID":2}
        ]"#;
        let w = parse_hyprland(json).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].app_id, "firefox");
        assert!(w[0].focused);
        assert_eq!(w[1].workspace.as_deref(), Some("2"));
    }

    #[test]
    fn sway_tree_walk() {
        let json = r#"{"type":"root","nodes":[{"type":"output","nodes":[
          {"type":"workspace","name":"1","nodes":[
            {"type":"con","id":7,"pid":10,"app_id":"foot","name":"~","focused":true,"nodes":[]},
            {"type":"con","id":8,"nodes":[{"type":"con","id":9,"pid":11,"app_id":null,
               "window_properties":{"class":"Gimp"},"name":"GIMP","nodes":[]}]}
          ],"floating_nodes":[{"type":"floating_con","id":12,"pid":12,"app_id":"pavucontrol","name":"Volume","nodes":[]}]}
        ]}]}"#;
        let w = parse_sway(json).unwrap();
        let ids: Vec<&str> = w.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["7", "9", "12"]);
        assert_eq!(w[1].app_id, "Gimp");
        assert!(w[0].focused);
        assert_eq!(w[2].workspace.as_deref(), Some("1"));
    }

    #[test]
    fn niri_and_wmctrl() {
        let w = parse_niri(r#"[{"id":3,"title":"Mail","app_id":"thunderbird","workspace_id":1,"is_focused":false}]"#)
            .unwrap();
        assert_eq!(w[0].id, "3");
        let w =
            parse_wmctrl("0x01  0 Navigator.firefox  host  Docs — Mozilla Firefox\n0x02 -1 panel.Panel host panel\n");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].app_id, "firefox");
        assert_eq!(w[0].title, "Docs — Mozilla Firefox");
        assert_eq!(w[0].workspace.as_deref(), Some("1"));
    }
}
