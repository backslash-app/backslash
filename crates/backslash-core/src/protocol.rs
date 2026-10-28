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
    /// What a `menu-bar` command shows in the system tray.
    Menu(MenuView),
}

/// An icon in the system tray, with a menu.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MenuView {
    pub id: Option<String>,
    /// Text next to the icon, where the tray shows one (GNOME, Ubuntu).
    pub title: Option<String>,
    /// Defaults to the command's icon.
    pub icon: Option<String>,
    pub icon_tint: Option<String>,
    pub tooltip: Option<String>,
    pub items: Vec<MenuItem>,
}

/// A line of a tray menu: runs its action when clicked, or opens `items`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MenuItem {
    /// Stable across runs (defaults to the title): a click runs the command
    /// again and finds the item by its id.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<String>,
    pub icon_tint: Option<String>,
    /// Items with the same section are grouped under a header.
    pub section: Option<String>,
    pub enabled: bool,
    pub action: Option<Action>,
    /// A submenu.
    pub items: Vec<MenuItem>,
}

impl Default for MenuItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            subtitle: None,
            icon: None,
            icon_tint: None,
            section: None,
            enabled: true,
            action: None,
            items: Vec::new(),
        }
    }
}

impl MenuView {
    /// The item at `path` (ids from the top level down).
    pub fn find(&self, path: &[String]) -> Option<&MenuItem> {
        let (first, rest) = path.split_first()?;
        let mut item = self.items.iter().find(|i| &i.id == first)?;
        for id in rest {
            item = item.items.iter().find(|i| &i.id == id)?;
        }
        Some(item)
    }
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
    /// Choices shown as chips under the search bar (an assignee, a team…).
    pub filter: Option<ListFilter>,
    /// A short line in the footer: "12 issues · updated just now".
    pub status: Option<String>,
}

/// Mutually exclusive choices that change what a list shows. Picking one
/// sends `filter` and the extension answers with the new list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListFilter {
    pub options: Vec<DropdownOption>,
    pub value: String,
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
    /// Colors a symbolic icon (one of the [`Accessory::color`] names).
    pub icon_tint: Option<String>,
    /// What the side panel shows for this item; without it, the launcher
    /// derives a summary from the title, subtitle and accessories.
    pub detail: Option<ItemDetail>,
}

/// The side panel of a list item.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemDetail {
    /// A picture shown big at the top (album cover, screenshot): a URL from
    /// the extension's `net` hosts, or an absolute path.
    pub image: Option<String>,
    /// Big numbers in a row of small cards: "3 · In progress".
    pub stats: Vec<Stat>,
    pub markdown: Option<String>,
    pub metadata: Vec<MetadataEntry>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stat {
    pub label: String,
    pub value: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Accessory {
    pub text: Option<String>,
    /// Rendered as a pill.
    pub tag: Option<String>,
    pub icon: Option<String>,
    /// One of the theme accents: `red`, `orange`, `yellow`, `green`, `blue`,
    /// `purple`, `teal`, `pink`, `gray`.
    pub color: Option<String>,
    /// A person: initials in a colored circle, or a picture (URL or path).
    pub avatar: Option<String>,
    /// Shown on hover.
    pub tooltip: Option<String>,
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
    /// Groups actions under a header in the action panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(flatten)]
    pub kind: ActionKind,
}

/// Built-in actions run inside the launcher without waking the extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActionKind {
    /// Open a URL, file or directory with the user's default handler.
    Open { target: String },
    /// Copy text to the clipboard and close the window. `concealed` marks a
    /// secret: clipboard managers (ours included) don't record it.
    Copy {
        text: String,
        #[serde(default)]
        concealed: bool,
    },
    /// Paste text into the app that had the focus (copies it where that is not possible).
    Paste {
        text: String,
        #[serde(default)]
        concealed: bool,
    },
    /// Handled by the extension (`action` request), which answers with an [`Effect`].
    Callback,
    /// Opens more actions in the action panel. With `lazy`, they are asked
    /// for (`submenu` request) when the user opens it.
    Submenu {
        #[serde(default)]
        actions: Vec<Action>,
        #[serde(default)]
        lazy: bool,
    },
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
    pub stats: Vec<Stat>,
    /// A short line in the footer.
    pub status: Option<String>,
}

/// One line of a side panel: a label and a value, which can be text with
/// an icon, colored tags, a link, a person or a progress bar. A `separator`
/// draws a line instead.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetadataEntry {
    pub label: String,
    pub value: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub tags: Vec<Tag>,
    /// Opened when the value is clicked.
    pub link: Option<String>,
    pub avatar: Option<String>,
    /// From 0 to 1.
    pub progress: Option<f64>,
    pub separator: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tag {
    pub text: String,
    pub color: Option<String>,
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
    #[serde(default)]
    pub launch_type: LaunchType,
}

/// Why a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchType {
    /// The user opened it, or clicked its tray menu.
    #[default]
    UserInitiated,
    /// On its `interval`, or a tray menu refreshing on its own.
    Background,
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
    fn rich_items_filters_and_submenus() {
        let view: View = serde_json::from_value(json!({
            "type": "list",
            "filter": { "options": [{ "value": "me", "title": "Mine" }], "value": "me" },
            "items": [{
                "id": "ENG-1", "title": "Crash", "icon": "bs-circle-half", "iconTint": "yellow",
                "accessories": [{ "avatar": "Ada Lovelace" }],
                "detail": {
                    "stats": [{ "label": "Points", "value": "3" }],
                    "metadata": [
                        { "label": "Labels", "tags": [{ "text": "Bug", "color": "red" }] },
                        { "separator": true },
                        { "label": "Progress", "progress": 0.5 }
                    ]
                },
                "actions": [{ "id": "s", "title": "Status", "kind": "submenu", "section": "Issue", "actions": [
                    { "id": "s1", "title": "Done", "kind": "callback" }
                ] }]
            }]
        }))
        .unwrap();
        let View::List(list) = view else { panic!("expected a list") };
        assert_eq!(list.filter.unwrap().value, "me");
        let item = &list.items[0];
        assert_eq!(item.icon_tint.as_deref(), Some("yellow"));
        let detail = item.detail.as_ref().unwrap();
        assert_eq!(detail.metadata[0].tags[0].color.as_deref(), Some("red"));
        assert!(detail.metadata[1].separator);
        assert_eq!(item.actions[0].section.as_deref(), Some("Issue"));
        let ActionKind::Submenu { actions, lazy } = &item.actions[0].kind else { panic!("expected a submenu") };
        assert_eq!((actions.len(), *lazy), (1, false));

        // Old extensions don't send `concealed`.
        let copy: Action =
            serde_json::from_value(json!({ "id": "c", "title": "Copy", "kind": "copy", "text": "x" })).unwrap();
        assert_eq!(copy.kind, ActionKind::Copy { text: "x".into(), concealed: false });
        let secret: Action = serde_json::from_value(
            json!({ "id": "p", "title": "Copy Password", "kind": "copy", "text": "x", "concealed": true }),
        )
        .unwrap();
        assert_eq!(secret.kind, ActionKind::Copy { text: "x".into(), concealed: true });
    }

    #[test]
    fn menu_views() {
        let view: View = serde_json::from_value(serde_json::json!({
            "type": "menu", "id": "v1", "title": "3", "icon": "bs-bell",
            "items": [
                { "id": "inbox", "title": "Inbox", "items": [
                    { "id": "pr", "title": "PR #12", "action": { "id": "v1:a1", "title": "Open", "kind": "callback" } }
                ] },
                { "id": "off", "title": "Disabled", "enabled": false }
            ]
        }))
        .unwrap();
        let View::Menu(menu) = view else { panic!("expected a menu") };
        assert!(menu.items[0].enabled, "enabled by default");
        assert!(!menu.items[1].enabled);
        let pr = menu.find(&["inbox".into(), "pr".into()]).unwrap();
        assert_eq!(pr.action.as_ref().unwrap().id, "v1:a1");
        assert!(menu.find(&["pr".into()]).is_none());
        let start: StartParams = serde_json::from_value(serde_json::json!({
            "command": "c", "arguments": {}, "preferences": {}, "query": "", "launchType": "background"
        }))
        .unwrap();
        assert_eq!(start.launch_type, LaunchType::Background);
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
