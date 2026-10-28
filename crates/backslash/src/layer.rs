//! Overlay window through `wlr-layer-shell` (Sway, Hyprland, KDE, niri,
//! river…), using the gtk4-layer-shell library when it is installed.
//!
//! The library must be loaded before libwayland-client to work, which a
//! program linked against GTK cannot do by itself. So at startup the launcher
//! re-executes itself once with `LD_PRELOAD` (the method documented by
//! gtk4-layer-shell), then looks the functions up at runtime: no build-time
//! dependency, and nothing changes where the library is absent.

use std::ffi::{c_int, c_void, CString};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;

use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;

const LIB: &str = "libgtk4-layer-shell.so.0";
const CHILD_ENV: &str = "BACKSLASH_LAYER_SHELL";

const LAYER_OVERLAY: c_int = 3;
const EDGE_TOP: c_int = 2;
const KEYBOARD_EXCLUSIVE: c_int = 1;

/// Where gtk4-layer-shell is installed, if it is. `$BACKSLASH_LAYER_SHELL_LIB`
/// names it where it is in no usual place (Nix), without `LD_LIBRARY_PATH`,
/// which would reach the extensions too.
pub fn library_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BACKSLASH_LAYER_SHELL_LIB").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let dirs = [
        "/usr/lib",
        "/usr/lib64",
        "/usr/local/lib",
        "/usr/local/lib64",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/run/current-system/sw/lib",
    ];
    let extra = std::env::var_os("LD_LIBRARY_PATH").map(|v| std::env::split_paths(&v).collect::<Vec<_>>());
    extra.into_iter().flatten().chain(dirs.iter().map(PathBuf::from)).map(|d| d.join(LIB)).find(|p| p.is_file())
}

/// Called first thing for the GUI. Re-executes the process with the library
/// preloaded when we are on Wayland and it is available. Returns normally
/// otherwise (and in the re-executed child).
pub fn reexec_if_needed(enabled: bool) {
    if std::env::var_os(CHILD_ENV).is_some() {
        // In the child: don't leak the preload into apps we launch.
        restore_ld_preload();
        return;
    }
    if !enabled || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let Some(lib) = library_path() else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    let previous = std::env::var("LD_PRELOAD").unwrap_or_default();
    let preload = if previous.is_empty() { lib.display().to_string() } else { format!("{}:{previous}", lib.display()) };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("LD_PRELOAD", preload)
        .env(CHILD_ENV, previous)
        .exec();
    // exec only returns on failure; carry on without the overlay.
    eprintln!("backslash: could not enable layer-shell: {err}");
}

fn restore_ld_preload() {
    match std::env::var(CHILD_ENV) {
        Ok(previous) if !previous.is_empty() => std::env::set_var("LD_PRELOAD", previous),
        _ => std::env::remove_var("LD_PRELOAD"),
    }
}

fn symbol(name: &str) -> Option<*mut c_void> {
    let name = CString::new(name).ok()?;
    // SAFETY: dlsym with RTLD_DEFAULT and a valid C string has no other requirement.
    let ptr = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    (!ptr.is_null()).then_some(ptr)
}

/// Turns `window` into an overlay layer surface, if possible. Must run before
/// the window is first shown. Returns whether layer-shell is active.
pub fn setup(window: &gtk::ApplicationWindow) -> bool {
    if std::env::var_os(CHILD_ENV).is_none() {
        return false;
    }
    type IsSupported = unsafe extern "C" fn() -> c_int;
    type WindowFn = unsafe extern "C" fn(*mut gtk::ffi::GtkWindow);
    type WindowInt = unsafe extern "C" fn(*mut gtk::ffi::GtkWindow, c_int);
    type WindowIntInt = unsafe extern "C" fn(*mut gtk::ffi::GtkWindow, c_int, c_int);
    type WindowStr = unsafe extern "C" fn(*mut gtk::ffi::GtkWindow, *const std::ffi::c_char);

    let (Some(is_supported), Some(init), Some(set_layer), Some(set_keyboard), Some(set_anchor), Some(set_namespace)) = (
        symbol("gtk_layer_is_supported"),
        symbol("gtk_layer_init_for_window"),
        symbol("gtk_layer_set_layer"),
        symbol("gtk_layer_set_keyboard_mode"),
        symbol("gtk_layer_set_anchor"),
        symbol("gtk_layer_set_namespace"),
    ) else {
        return false;
    };
    // SAFETY: the symbols come from gtk4-layer-shell, whose C signatures match
    // the types above; the window pointer is valid for the duration of the calls.
    unsafe {
        let is_supported: IsSupported = std::mem::transmute(is_supported);
        if is_supported() == 0 {
            return false;
        }
        let ptr: *mut gtk::ffi::GtkWindow = window.upcast_ref::<gtk::Window>().to_glib_none().0;
        std::mem::transmute::<*mut c_void, WindowFn>(init)(ptr);
        std::mem::transmute::<*mut c_void, WindowInt>(set_layer)(ptr, LAYER_OVERLAY);
        std::mem::transmute::<*mut c_void, WindowInt>(set_keyboard)(ptr, KEYBOARD_EXCLUSIVE);
        std::mem::transmute::<*mut c_void, WindowIntInt>(set_anchor)(ptr, EDGE_TOP, 1);
        let namespace = CString::new("backslash").expect("no NUL");
        std::mem::transmute::<*mut c_void, WindowStr>(set_namespace)(ptr, namespace.as_ptr());
    }
    true
}

/// Distance from the top of the screen, set each time the window opens.
pub fn set_top_margin(window: &gtk::ApplicationWindow, margin: i32) {
    type SetMargin = unsafe extern "C" fn(*mut gtk::ffi::GtkWindow, c_int, c_int);
    let Some(f) = symbol("gtk_layer_set_margin") else { return };
    // SAFETY: see `setup`.
    unsafe {
        let ptr: *mut gtk::ffi::GtkWindow = window.upcast_ref::<gtk::Window>().to_glib_none().0;
        std::mem::transmute::<*mut c_void, SetMargin>(f)(ptr, EDGE_TOP, margin);
    }
}
