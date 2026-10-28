//! `BACKSLASH_UI_TRACE=<file>`: writes what the window shows to `<file>` as
//! JSON whenever it changes, so UI tests can drive the real window with
//! keystrokes (`tests/ui.rs`) and check the outcome without reading pixels.

use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{json, Value};

use super::Launcher;

impl Launcher {
    pub(super) fn start_trace(self: &Rc<Self>) {
        let Some(path) = std::env::var_os("BACKSLASH_UI_TRACE") else { return };
        let weak = Rc::downgrade(self);
        let mut last = Value::Null;
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(l) = weak.upgrade() else { return glib::ControlFlow::Break };
            let state = l.trace_state();
            if state != last {
                let tmp = std::path::PathBuf::from(&path).with_extension("tmp");
                if std::fs::write(&tmp, state.to_string()).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                    eprintln!("backslash: cannot write the UI trace");
                }
                last = state;
            }
            glib::ControlFlow::Continue
        });
    }

    fn trace_state(&self) -> Value {
        let s = self.state.borrow();
        let scroll =
            |adj: gtk::Adjustment| json!({ "value": adj.value(), "page": adj.page_size(), "upper": adj.upper() });
        let panel_open = self.action_panel.is_visible();
        json!({
            "visible": self.window.is_visible(),
            "size": [self.panel.width(), self.panel.height()],
            "depth": s.frames.len(),
            "page": self.pages.visible_child_name().map(|n| n.to_string()),
            "context": self.context.text().to_string(),
            "query": self.entry.text().to_string(),
            "rows": s.rows.iter().map(|r| r.title.clone()).collect::<Vec<_>>(),
            "selected": self.list.selected_row().map(|r| r.index()),
            "list_scroll": scroll(self.scroller.vadjustment()),
            "detail": self.detail.text().to_string(),
            "toast": if self.toast.is_visible() { self.toast.text().to_string() } else { String::new() },
            "theme": format!("{:?}", s.config.theme),
            "actions": if panel_open {
                json!({
                    "items": self.popover_actions.borrow().iter().map(|a| a.title.clone()).collect::<Vec<_>>(),
                    "selected": self.popover_list.selected_row().map(|r| r.index()),
                    "scroll": scroll(self.popover_scroll.vadjustment()),
                })
            } else {
                Value::Null
            },
        })
    }
}
