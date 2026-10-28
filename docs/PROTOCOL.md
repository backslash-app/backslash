# Extension protocol

The launcher starts one process per running command and talks
[JSON-RPC 2.0](https://www.jsonrpc.org/specification) with it: **one JSON
object per line** on stdin (launcher → extension) and stdout (extension →
launcher). Stderr is the extension's log.

The TypeScript SDK (`sdk/runtime.ts`) implements this for you. This document
is for other languages and for contributors. The Rust types live in
`crates/backslash-core/src/protocol.rs`.

## Lifecycle

1. The launcher spawns the process (Deno: `deno run [permissions] sdk/runtime.ts <entry>`;
   exec runtime: `<command...> <entry>`), working directory = extension folder.
2. It sends `start`; the reply is an **effect** (usually `push` with the first view).
3. While the view is open it sends `search` and `action` requests.
4. When the user leaves the command (Escape to the root, or the window hides),
   stdin is closed and the process is killed.

## Launcher → extension

| Method | Params | Result |
|---|---|---|
| `start` | `{ command, arguments, preferences, query }` | Effect |
| `search` | `{ view, query }` | View (a list) |
| `filter` | `{ view, value }` | View (the list for that filter option) |
| `submenu` | `{ action }` | Action[] (the content of a `lazy` submenu) |
| `action` | `{ action, values? }` | Effect |
| `$/cancel` (notification) | `{ id }` | — abort request `id` |

`values` is set when submitting a form: the action id is then `"<form view id>:submit"`.

A cancelled request should answer with error code `-32800`.

## Extension → launcher

| Method | Params | Result |
|---|---|---|
| `view/update` (notification) | `{ view }` | — replace the view with the same `id` |
| `toast` | `{ title, message?, style: "info" \| "success" \| "failure" }` | `null` |
| `clipboard/write` | `{ text }` | `null` |
| `clipboard/paste` | `{ text }` | `null`; the launcher hides and pastes into the focused app (or copies) |
| `clipboard/read` | `{}` | string (needs `clipboard-read`) |
| `open` | `{ target }` | `null` |
| `close` | `{}` | `null` |
| `storage/get` | `{ key }` | value or `null` |
| `storage/set` | `{ key, value }` (`null` deletes) | `null` |
| `storage/keys` | `{}` | string[] |
| `oauth/token` | `{ provider, interactive }` | access token; `null` when not signed in and `interactive` is `false` |
| `oauth/signOut` | `{ provider }` | `null` |
| `command/metadata` | `{ subtitle }` | `null`; the subtitle shown under the command in the root search (`null` clears it) |

`provider` is an object with `id`, `authorizeUrl`, `tokenUrl`, `clientId`
and optionally `title`, `clientSecret`, `scopes`, `scopeSeparator`, `params`,
`redirectPort`, `redirectPath` and `clientAuth` (`"basic"`), see the SDK's
`OAuthProvider`. The token URL's host must be in the extension's `net`
permission.

## Views

Every view carries an `id` chosen by the extension, sent back in `search`
and used to target `view/update`.

```jsonc
{
  "type": "list",
  "id": "v1",
  "title": null,
  "placeholder": "Search…",
  "filtering": "host",        // "host": the launcher filters `items`; "extension": `search` requests
  "loading": false,
  "layout": "list",           // or "grid": tiles with a big icon and the title below
  "columns": null,            // tiles per line in a grid (default 6)
  "filter": { "options": [{ "value": "mine", "title": "Mine" }], "value": "mine" },  // chips under the search field
  "status": "3 in progress",  // footer line
  "items": [{
    "id": "react",
    "title": "react",
    "subtitle": "UI library",
    "icon": "bs-package",       // a Backslash glyph (bs-*), theme icon name, absolute path, URL or emoji
    "iconTint": "blue",         // colors a glyph
    "section": null,
    "keywords": [],
    "accessories": [
      { "text": "75%", "tag": null, "icon": null, "color": "red", "tooltip": "Coverage" },
      { "avatar": "Ada Lovelace" }   // initials, or a picture URL from the `net` hosts
    ],
    "detail": {                 // the preview panel
      "image": null,
      "stats": [{ "label": "Downloads", "value": "25M", "color": null }],
      "markdown": "A JavaScript library…",
      "metadata": [
        { "label": "Version", "value": "19.0.0", "icon": "bs-tag", "color": null },
        { "label": "Keywords", "tags": [{ "text": "ui", "color": "blue" }] },
        { "separator": true },
        { "label": "Author", "value": "Meta", "avatar": "Meta" },
        { "label": "Homepage", "value": "react.dev", "link": "https://react.dev" },
        { "label": "Coverage", "value": "75%", "progress": 0.75 }
      ]
    },
    "actions": [
      { "id": "a1", "title": "Open", "kind": "open", "target": "https://…" },
      { "id": "a2", "title": "Copy", "kind": "copy", "text": "…", "section": "Copy" },   // or "paste"
      { "id": "a3", "title": "Details", "kind": "callback", "shortcut": "<Ctrl>d", "destructive": false, "confirm": null },
      { "id": "a4", "title": "Set Status", "kind": "submenu", "actions": [ /* … */ ], "lazy": false }
    ]
  }],
  "empty": { "title": "Type to search", "description": null, "icon": null },
  "actions": []
}
```

```jsonc
{ "type": "detail", "id": "v2", "title": "react", "markdown": "# react",
  "stats": [{ "label": "Stars", "value": "230k" }], "status": "Updated 2 h ago",
  "metadata": [{ "label": "Version", "value": "19" }], "actions": [], "loading": false }
```

```jsonc
{ "type": "form", "id": "v3", "title": "New issue", "submitTitle": "Create",
  "fields": [
    { "id": "title", "title": "Title", "kind": "text", "required": true, "placeholder": null, "value": null },
    { "id": "lang", "title": "Language", "kind": "dropdown", "options": [{ "value": "ts", "title": "TypeScript" }] }
  ] }
```

Field kinds: `text`, `password`, `textArea`, `checkbox`, `dropdown`.

`open` and `copy` actions run inside the launcher without waking the
extension; `callback` actions are sent back as `action` requests. A
`submenu` opens a second level in the action panel; when `lazy` is true its
actions are asked for with `submenu` the moment it opens. Actions sharing a
`section` are grouped under a header.

Colors (`iconTint`, `color`) are `red`, `orange`, `yellow`, `green`, `teal`,
`blue`, `purple`, `pink`, `gray`, or a `#rrggbb` value.

## Effects

```jsonc
{ "type": "none" }
{ "type": "push", "view": { ... } }
{ "type": "replace", "view": { ... } }
{ "type": "pop" }
{ "type": "close", "message": "Copied" }   // message optional
```

## Minimal extension in Python

```python
import json, sys

def send(msg):
    print(json.dumps({"jsonrpc": "2.0", **msg}), flush=True)

for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("method") == "start":
        items = [{"id": str(i), "title": f"Item {i}", "actions": [
            {"id": "copy", "title": "Copy", "kind": "copy", "text": f"Item {i}"}]} for i in range(3)]
        send({"id": msg["id"], "result": {"type": "push", "view": {"type": "list", "id": "main", "items": items}}})
```
