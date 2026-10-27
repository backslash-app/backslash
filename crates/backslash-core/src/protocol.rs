//! Wire types shared by the launcher and extensions.
//!
//! Extensions never draw anything: they describe *views* as plain data and the
//! launcher renders them. Every type here is serialized as camelCase JSON and is
//! documented in `docs/PROTOCOL.md`; keep both in sync.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A screen produced by an extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum View {
    List(ListView),
    Detail(DetailView),
    Form(FormView),
}

/// Who filters list items when the user types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Filtering {
    /// The launcher fuzzy-filters `items` locally. No round trip per keystroke.
    #[default]
    Host,
    /// Every keystroke is sent to the extension (`search` request).
    Extension,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListView {
    /// Assigned by the extension runtime; sent back with `search` and `action`.
    pub id: Option<String>,
    pub title: Option<String>,
    pub placeholder: Option<String>,
    pub filtering: Filtering,
    pub loading: bool,
    pub items: Vec<Item>,
    pub empty: Option<EmptyState>,
    /// Actions available even when no item is selected.
    pub actions: Vec<Action>,
    pub layout: Layout,
    /// Tiles per line in a grid.
    pub columns: Option<u32>,
}

/// How list items are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Rows with a preview panel.
    #[default]
    List,
    /// Tiles with a big icon and the title below: emoji, images, colors.
    Grid,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EmptyState {
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Icon name from the icon theme, an absolute path, or a single emoji.
    pub icon: Option<String>,
    /// Items with the same section are grouped under a header.
    pub section: Option<String>,
    pub keywords: Vec<String>,
    pub accessories: Vec<Accessory>,
    /// The first action is the primary one (Enter).
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Accessory {
    pub text: Option<String>,
    /// Rendered as a pill.
    pub tag: Option<String>,
    pub icon: Option<String>,
    /// One of the theme accents: `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `gray`.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// GTK accelerator syntax, e.g. `<Ctrl>c` or `<Ctrl><Shift>Return`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub destructive: bool,
    /// Ask the user before running the action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(flatten)]
    pub kind: ActionKind,
}

/// Built-in actions run inside the launcher without waking the extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActionKind {
    /// Open a URL, file or directory with the user's default handler.
    Open { target: String },
    /// Copy text to the clipboard and close the window.
    Copy { text: String },
    /// Paste text into the app that had the focus (copies it where that is not possible).
    Paste { text: String },
    /// Handled by the extension (`action` request), which answers with an [`Effect`].
    Callback,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DetailView {
    /// Assigned by the extension runtime; sent back with `search` and `action`.
    pub id: Option<String>,
    pub title: Option<String>,
    /// CommonMark subset: headings, emphasis, code, links and lists.
    pub markdown: String,
    pub metadata: Vec<MetadataEntry>,
    pub actions: Vec<Action>,
    /// More text is coming (a streamed answer).
    pub loading: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetadataEntry {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FormView {
    /// Assigned by the extension runtime; sent back with `search` and `action`.
    pub id: Option<String>,
    pub title: Option<String>,
    pub fields: Vec<Field>,
    pub submit_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(flatten)]
    pub kind: FieldKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Password,
    TextArea,
    Checkbox,
    Dropdown { options: Vec<DropdownOption> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DropdownOption {
    pub value: String,
    pub title: String,
}

/// What the launcher should do after an extension handled an action.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Effect {
    /// Stay on the current view.
    #[default]
    None,
    /// Navigate to a new view; Escape comes back.
    Push { view: View },
    /// Replace the current view.
    Replace { view: View },
    /// Go back to the previous view.
    Pop,
    /// Hide the launcher, optionally flashing a short message.
    Close {
        #[serde(default)]
        message: Option<String>,
    },
}

/// Request sent by the launcher to start a command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartParams {
    pub command: String,
    pub arguments: Map<String, Value>,
    pub preferences: Map<String, Value>,
    pub query: String,
}

/// Toast severity used by `toast` requests from extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToastStyle {
    #[default]
    Info,
    Success,
    Failure,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_view_round_trips_with_defaults() {
        let view: View = serde_json::from_value(json!({
            "type": "list",
            "items": [{
                "id": "react",
                "title": "react",
                "accessories": [{ "tag": "v19.0.0" }],
                "actions": [
                    { "id": "a0", "title": "Open", "kind": "open", "target": "https://npmjs.com/package/react" },
                    { "id": "a1", "title": "Kill", "kind": "callback", "destructive": true }
                ]
            }]
        }))
        .unwrap();

        let View::List(list) = &view else { panic!("expected a list") };
        assert_eq!(list.filtering, Filtering::Host);
        assert_eq!(list.layout, Layout::List);
        assert_eq!(list.items[0].accessories[0].tag.as_deref(), Some("v19.0.0"));
        assert_eq!(
            list.items[0].actions[0].kind,
            ActionKind::Open { target: "https://npmjs.com/package/react".into() }
        );
        assert!(list.items[0].actions[1].destructive);

        let back: View = serde_json::from_value(serde_json::to_value(&view).unwrap()).unwrap();
        assert_eq!(back, view);
    }

    #[test]
    fn grid_layout() {
        let view: View =
            serde_json::from_value(json!({ "type": "list", "layout": "grid", "columns": 8, "items": [] })).unwrap();
        let View::List(list) = view else { panic!("expected a list") };
        assert_eq!((list.layout, list.columns), (Layout::Grid, Some(8)));
    }

    #[test]
    fn effects_parse() {
        let effect: Effect = serde_json::from_value(json!({ "type": "close", "message": "Copied" })).unwrap();
        assert_eq!(effect, Effect::Close { message: Some("Copied".into()) });
        let effect: Effect = serde_json::from_value(json!({ "type": "none" })).unwrap();
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn form_fields_parse() {
        let view: View = serde_json::from_value(json!({
            "type": "form",
            "fields": [
                { "id": "name", "title": "Name", "kind": "text", "required": true },
                { "id": "lang", "title": "Lang", "kind": "dropdown",
                  "options": [{ "value": "ts", "title": "TypeScript" }] }
            ]
        }))
        .unwrap();
        let View::Form(form) = view else { panic!("expected a form") };
        assert!(form.fields[0].required);
        assert!(matches!(form.fields[1].kind, FieldKind::Dropdown { ref options } if options.len() == 1));
    }
}
