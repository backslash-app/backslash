//! UI tests: the real launcher in a virtual display (Xvfb), driven with
//! keystrokes (xdotool), checked through what it reports with
//! `BACKSLASH_UI_TRACE` (see `launcher/trace.rs`).
//!
//! They need Xvfb, xdotool and dbus-run-session, and run when
//! `BACKSLASH_UI_TESTS=1` (`make ui-test`, and CI).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn enabled() -> bool {
    if std::env::var("BACKSLASH_UI_TESTS").is_ok_and(|v| v == "1") {
        return true;
    }
    eprintln!("UI tests skipped: set BACKSLASH_UI_TESTS=1 (needs Xvfb, xdotool, dbus-run-session)");
    false
}

struct Ui {
    xvfb: Child,
    app: Child,
    display: String,
    dir: tempfile::TempDir,
}

impl Drop for Ui {
    fn drop(&mut self) {
        let _ = self.app.kill();
        let _ = self.app.wait();
        let _ = self.xvfb.kill();
        let _ = self.xvfb.wait();
    }
}

impl Ui {
    /// Starts a display and the launcher, with an empty home, `config` as
    /// `config.toml` and the extensions in `extensions` (a folder of them).
    fn start(config: &str, extensions: Option<&Path>) -> Self {
        let mut xvfb = Command::new("Xvfb")
            .args(["-displayfd", "1", "-screen", "0", "1280x900x24", "-nolisten", "tcp"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Xvfb");
        let mut line = String::new();
        BufReader::new(xvfb.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let display = format!(":{}", line.trim());

        let dir = tempfile::tempdir().unwrap();
        let p = |name: &str| dir.path().join(name);
        std::fs::create_dir_all(p("config/backslash")).unwrap();
        std::fs::write(p("config/backslash/config.toml"), config).unwrap();
        // The welcome screen was seen already.
        std::fs::create_dir_all(p("state/backslash")).unwrap();
        std::fs::write(p("state/backslash/welcomed"), "").unwrap();

        let mut cmd = Command::new("dbus-run-session");
        cmd.arg("--").arg(env!("CARGO_BIN_EXE_backslash"));
        cmd.env("DISPLAY", &display)
            .env("GDK_BACKEND", "x11")
            .env("GTK_A11Y", "none")
            .env("HOME", p("home"))
            .env("XDG_CONFIG_HOME", p("config"))
            .env("XDG_STATE_HOME", p("state"))
            .env("XDG_DATA_HOME", p("data"))
            .env("XDG_CACHE_HOME", p("cache"))
            .env("BACKSLASH_UI_TRACE", p("trace.json"))
            .env_remove("WAYLAND_DISPLAY")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match extensions {
            Some(e) => cmd.env("BACKSLASH_EXTENSIONS_PATH", e),
            None => cmd.env("BACKSLASH_EXTENSIONS_PATH", p("no-extensions")),
        };
        let app = cmd.spawn().expect("dbus-run-session");
        let ui = Self { xvfb, app, display, dir };
        ui.wait_for("the window", |s| s["visible"] == true);
        ui
    }

    fn trace(&self) -> PathBuf {
        self.dir.path().join("trace.json")
    }

    fn state(&self) -> Value {
        std::fs::read_to_string(self.trace()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
    }

    /// Waits until `check` holds, or fails with the last state.
    fn wait_for(&self, what: &str, check: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let state = self.state();
            if !state.is_null() && check(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}; last state: {state:#}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn xdotool(&self, args: &[&str]) {
        let ok = Command::new("xdotool").args(args).env("DISPLAY", &self.display).status().unwrap();
        assert!(ok.success(), "xdotool {args:?}");
    }

    fn type_text(&self, text: &str) {
        self.xdotool(&["type", "--delay", "30", text]);
    }

    fn key(&self, key: &str) {
        self.xdotool(&["key", key]);
    }

    fn keys(&self, key: &str, times: usize) {
        for _ in 0..times {
            self.key(key);
        }
    }
}

fn rows(state: &Value) -> Vec<String> {
    state["rows"]
        .as_array()
        .map(|r| r.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

#[test]
fn calculator_answers_as_you_type() {
    if !enabled() {
        return;
    }
    let ui = Ui::start("", None);
    ui.type_text("2+3*4");
    ui.wait_for("the result", |s| rows(s).first().is_some_and(|r| r == "14"));
    // Escape clears the search, then hides the window.
    ui.key("Escape");
    ui.wait_for("an empty search", |s| s["query"] == "");
    ui.key("Escape");
    ui.wait_for("the window to hide", |s| s["visible"] == false);
}

#[test]
fn window_size_follows_the_config() {
    if !enabled() {
        return;
    }
    let ui = Ui::start("window_size = \"small\"\ntheme = \"nord\"\n", None);
    let state = ui.wait_for("the window", |s| s["size"][0].as_i64() > Some(0));
    // The small size is 740×460 with the shadow; the panel, a bit less (its border).
    let (w, h) = (state["size"][0].as_i64().unwrap(), state["size"][1].as_i64().unwrap());
    assert!((700..=712).contains(&w) && (420..=432).contains(&h), "{w}×{h}");
    assert_eq!(state["theme"], "Nord");
}

#[test]
fn change_theme_from_the_search() {
    if !enabled() {
        return;
    }
    let ui = Ui::start("", None);
    ui.type_text("change theme");
    ui.wait_for("the command", |s| rows(s).first().is_some_and(|r| r == "Change Theme"));
    ui.key("Return");
    ui.wait_for("the themes", |s| s["depth"] == 2 && rows(s).iter().any(|r| r == "Dracula"));
    ui.type_text("dracula");
    ui.wait_for("Dracula first", |s| rows(s).first().is_some_and(|r| r == "Dracula"));
    ui.key("Return");
    ui.wait_for("the theme to change", |s| s["theme"] == "Dracula");
    let config = std::fs::read_to_string(ui.dir.path().join("config/backslash/config.toml")).unwrap();
    assert!(config.contains("theme = \"dracula\""), "{config}");
}

/// The action panel follows the selection in long lists (it used not to).
#[test]
fn action_panel_scrolls_with_the_keyboard() {
    if !enabled() || Command::new("deno").arg("--version").output().is_err() {
        return;
    }
    let extensions = tempfile::tempdir().unwrap();
    let ext = extensions.path().join("many");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(
        ext.join("backslash.toml"),
        "[extension]\nid = \"many\"\ntitle = \"Many\"\n[[commands]]\nname = \"acts\"\ntitle = \"Many Actions\"\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("commands/acts.ts"),
        "import { copy, list } from \"@backslash/api\";\n\
         const actions = Array.from({ length: 40 }, (_, i) => copy(`a${i}`, `Action ${i}`));\n\
         export default list({ items: [{ title: \"Item\", actions }] });\n",
    )
    .unwrap();
    let ui = Ui::start("", Some(extensions.path()));
    ui.type_text("many actions");
    ui.wait_for("the command", |s| rows(s).first().is_some_and(|r| r == "Many Actions"));
    ui.key("Return");
    ui.wait_for("the list", |s| s["depth"] == 2 && rows(s) == ["Item"]);
    ui.key("ctrl+k");
    ui.wait_for("the action panel", |s| s["actions"]["items"].as_array().is_some_and(|a| a.len() == 40));
    ui.keys("Down", 30);
    let state = ui.wait_for("the 31st action", |s| s["actions"]["selected"] == 30);
    let scroll = &state["actions"]["scroll"];
    let (value, page) = (scroll["value"].as_f64().unwrap(), scroll["page"].as_f64().unwrap());
    assert!(value > 0.0 && page > 0.0, "the panel did not scroll: {scroll}");
    ui.keys("Up", 30);
    ui.wait_for("the panel back at the top", |s| {
        s["actions"]["selected"] == 0 && s["actions"]["scroll"]["value"] == 0.0
    });
}

/// An extension with one command whose items take `delay_ms` to load.
fn slow_extension(delay_ms: u32) -> tempfile::TempDir {
    let extensions = tempfile::tempdir().unwrap();
    let ext = extensions.path().join("slow");
    std::fs::create_dir_all(ext.join("commands")).unwrap();
    std::fs::write(
        ext.join("backslash.toml"),
        "[extension]\nid = \"slow\"\ntitle = \"Slow\"\n[[commands]]\nname = \"media\"\ntitle = \"Slow Media\"\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("commands/media.ts"),
        format!(
            "import {{ list }} from \"@backslash/api\";\n\
             export default list({{\n\
               items: async () => {{\n\
                 await new Promise((r) => setTimeout(r, {delay_ms}));\n\
                 return [{{ title: \"Movie\" }}, {{ title: \"Show\" }}];\n\
               }},\n\
             }});\n"
        ),
    )
    .unwrap();
    extensions
}

/// A command that takes a while shows a skeleton of its list, not the
/// screen it was started from.
#[test]
fn slow_commands_show_a_skeleton_until_their_items_come() {
    if !enabled() || Command::new("deno").arg("--version").output().is_err() {
        return;
    }
    let extensions = slow_extension(1500);
    let ui = Ui::start("", Some(extensions.path()));
    ui.type_text("slow media");
    ui.wait_for("the command", |s| rows(s).first().is_some_and(|r| r == "Slow Media"));
    ui.key("Return");
    let state = ui.wait_for("the skeleton", |s| s["page"] == "skeleton");
    assert_eq!(state["depth"], 2, "{state:#}");
    assert_eq!(state["context"], "Slow Media");
    ui.wait_for("the items", |s| s["depth"] == 2 && s["page"] == "list" && rows(s) == ["Movie", "Show"]);
    // Back to the search it was started from.
    ui.key("Escape");
    ui.wait_for("the root search", |s| s["depth"] == 1 && s["query"] == "slow media");
}

/// Going back from the skeleton gives up on the command: its view doesn't
/// open later on its own.
#[test]
fn leaving_the_skeleton_drops_the_command() {
    if !enabled() || Command::new("deno").arg("--version").output().is_err() {
        return;
    }
    let extensions = slow_extension(1500);
    let ui = Ui::start("", Some(extensions.path()));
    ui.type_text("slow media");
    ui.wait_for("the command", |s| rows(s).first().is_some_and(|r| r == "Slow Media"));
    ui.key("Return");
    ui.wait_for("the skeleton", |s| s["page"] == "skeleton");
    ui.key("Escape");
    ui.wait_for("the root search", |s| s["depth"] == 1);
    std::thread::sleep(Duration::from_millis(2500));
    let state = ui.state();
    assert_eq!(state["depth"], 1, "the view opened after leaving: {state:#}");
}

/// A 3×2 red PNG.
const RED_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x41, 0xd2, 0x05, 0xfb,
    0x87, 0xf0, 0xb9, 0x48, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn copied_images_show_in_the_history() {
    if !enabled() || Command::new("xclip").arg("-version").output().is_err() {
        return;
    }
    let ui = Ui::start("", None);
    let png = ui.dir.path().join("red.png");
    std::fs::write(&png, RED_PNG).unwrap();
    // xclip keeps serving the selection until something else takes it.
    let mut xclip = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-i"])
        .arg(&png)
        .env("DISPLAY", &ui.display)
        .spawn()
        .unwrap();
    ui.type_text("clipboard history");
    ui.wait_for("the command", |s| rows(s).first().is_some_and(|r| r == "Clipboard History"));
    ui.key("Return");
    let state = ui.wait_for("the image", |s| s["depth"] == 2 && rows(s).first().is_some_and(|r| r == "Image 3×2"));
    assert_eq!(rows(&state).len(), 1, "the image only, not its bytes as text: {state:#}");
    let stored = std::fs::read_dir(ui.dir.path().join("state/backslash/clipboard-images")).unwrap().count();
    assert_eq!(stored, 1);
    let _ = xclip.kill();
    let _ = xclip.wait();
}
