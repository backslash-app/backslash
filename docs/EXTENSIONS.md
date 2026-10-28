# Writing extensions

An extension is a folder with a `backslash.toml` manifest and one TypeScript
file per command. No build step, no `node_modules`, no bundler: save the file
and run the command again.

```sh
backslash new github        # scaffold ./github
cd github
backslash dev               # opens the launcher with your extension loaded: logs in the terminal,
                            # and the open command reloads when you save a file
backslash run github/hello  # or run a command straight from the terminal
```

## Anatomy

```
github/
├── backslash.toml        what the extension is and what it may do
├── commands/
│   └── hello.ts          one file per command (default export)
└── deno.json             editor support only (types, npm/jsr imports)
```

### `backslash.toml`

```toml
[extension]
id = "github"                       # [a-z0-9-]+, unique
title = "GitHub"
description = "Search repositories and pull requests"
version = "0.1.0"
author = "you"
icon = "folder-remote"              # icon theme name, absolute path or emoji
license = "MIT"

# Everything not listed is denied (enforced by Deno). `backslash list` shows it.
[permissions]
net = ["api.github.com"]            # hosts, or ["*"]
run = ["git"]                       # programs you may spawn
read = ["~/Projects"]               # beyond your own folder
write = []
env = ["GITHUB_TOKEN"]
clipboard-read = false              # writing the clipboard is always allowed

[[commands]]
name = "repos"                      # → commands/repos.ts
title = "Search Repositories"
description = "Your repositories, filtered as you type"
icon = "folder-remote"
keywords = ["git", "repo"]
mode = "view"                       # "view" (default) or "no-view"

[[commands]]
name = "clone"
title = "Clone Repository"
mode = "no-view"
[[commands.arguments]]              # asked before running, validated by the launcher
name = "url"
placeholder = "https://github.com/…"
required = true
# type = "text" (default) | "number" (min/max) | "choice" (options = [...])

[[preferences]]                     # edited in Settings → Extensions → Configure
name = "username"
title = "GitHub username"
required = true
# type = "text" (default) | "secret" (secrets.toml) | "boolean" | "number"
```

## Commands

A command module default-exports what to show. There are four shapes.

### A list filtered by the launcher

Load the items once; the launcher fuzzy-filters them as the user types,
instantly, with no round trip to your code.

```ts
import { action, copy, exec, list, open, refresh } from "@backslash/api";

export default list({
  placeholder: "Filter branches…",
  items: async () => {
    const out = await exec("git", ["branch", "--format=%(refname:short)"]);
    return out.split("\n").map((branch) => ({
      title: branch,
      icon: "🌿",
      actions: [
        action("Checkout", async () => {
          await exec("git", ["checkout", branch]);
          return refresh();
        }),
        copy(branch, "Copy Branch Name"),
      ],
    }));
  },
});
```

`items` can also be a plain array, or an **async generator** that yields
several times to show results progressively (the spinner stays on until it
returns).

### A list searched by your code

When results depend on the query (web APIs), use `search`. The launcher
debounces keystrokes and **aborts the previous call**: pass `signal` to
`fetch` and slow requests are cancelled for free.

```ts
export default list({
  placeholder: "Search packages…",
  async search(query, { signal }) {
    const res = await fetch(`https://registry.npmjs.org/-/v1/search?text=${query}`, { signal });
    const { objects } = await res.json();
    return objects.map((o) => ({ title: o.package.name, subtitle: o.package.description }));
  },
});
```

### A grid

For things you recognize by sight (emoji, images, colors, icons), show tiles
instead of rows. Everything else works the same: filtering, sections, actions.

```ts
export default list({
  layout: "grid",
  columns: 8,
  items: [{ title: "Party Popper", icon: "🎉", actions: [copy("🎉")] }],
});
```

Arrow keys move in both directions; the title shows under each tile.

### A detail or a form

```ts
import { close, detail, field, form, toast } from "@backslash/api";

export default detail({
  title: "About",
  markdown: "# Hello\n\nMarkdown with **bold**, `code` and [links](https://deno.com).",
  metadata: { Version: "1.0", License: "MIT" },
});

// or
export default form({
  title: "New Issue",
  fields: [
    field.text("title", "Title", { required: true }),
    field.textArea("body", "Description"),
    field.dropdown("label", "Label", ["bug", "feature"]),
    field.checkbox("draft", "Draft"),
  ],
  async submit(values) {
    // values = { title: "…", body: "…", label: "bug", draft: false }
    await toast("Issue created", { style: "success" });
    return close();
  },
});
```

To stream a long answer (an AI reply, a log), make `markdown` an async
generator yielding the whole text so far; the launcher shows it as it grows,
with a spinner until the generator ends:

```ts
export default detail({
  async *markdown({ signal }) {
    let text = "";
    for await (const chunk of answer(signal)) yield (text += chunk);
  },
});
```

A command can also export a function returning any view, to decide at runtime:
`export default async ({ preferences }) => list({...})`.

### A command without a view

```ts
import { exec, run } from "@backslash/api";

export default run<{ level: number }>(async ({ args }) => {
  await exec("wpctl", ["set-volume", "@DEFAULT_AUDIO_SINK@", `${args.level}%`]);
  return `Volume ${args.level}%`; // flashed, then the launcher closes
});
```

Throwing shows the error in the launcher. That applies everywhere.

## Items

```ts
import { Icons, meta } from "@backslash/api";

{
  id: "unique-id",            // keeps the selection across refreshes (default: title)
  title: "react",
  subtitle: "A JavaScript library",
  icon: { source: Icons.package, tint: "blue" },  // or a theme icon name, /absolute/path.png, a URL, "🔥"
  section: "Recent",          // grouped under a header
  keywords: ["ui"],           // extra words to match
  tags: ["v19.0.0"],          // pills on the right
  accessories: [{ text: "75%", color: "red", tooltip: "Coverage" }, { avatar: "Ada Lovelace" }],
  detail: {                   // the preview panel on the right
    stats: [{ label: "Downloads", value: "25M" }],
    markdown: "A JavaScript library for building user interfaces.",
    metadata: [
      meta.text("Version", "19.0.0", { icon: Icons.tag }),
      meta.tags("Keywords", [{ text: "ui", color: "blue" }, "web"]),
      meta.separator(),
      meta.person("Author", "Meta"),  // initials, or pass a picture URL
      meta.link("Homepage", "react.dev", "https://react.dev"),
      meta.progress("Coverage", 0.75),
    ],
  },
  actions: [ /* first = Enter, second = Ctrl+Enter, all in Ctrl+K */ ],
}
```

**Icons.** `Icons` lists Backslash's own glyphs (`Icons.inProgress`,
`Icons.pullRequest`, `Icons.priorityHigh`…): crisp vector icons drawn in
the theme's colors, the same on every desktop. Prefer them to theme icon
names and emoji. `tint` takes `red`, `orange`, `yellow`, `green`, `teal`,
`blue`, `purple`, `pink`, `gray` or a `#rrggbb` color. Pictures from the web
(avatars, favicons, covers in `detail.image`) are shown only when their host
is in your `net` permission.

**Filters and status.** A list can offer a few views of its data as chips
under the search field, cycled with Tab. The picked value is `ctx.filter`
and is remembered:

```ts
export default list({
  filter: { options: [{ value: "assigned", title: "Assigned" }, { value: "created", title: "Created" }] },
  status: (items) => `${items.length} issues`,   // the footer line
  async items({ filter }) { /* … */ },
});
```

**Command subtitle.** `updateCommandMetadata({ subtitle: "3 in progress" })`
shows a line under your command in the root search until the next update;
`null` clears it.

A `detail()` view takes `stats`, `status` and `metadata` as an array of
`meta.*` lines too.

## Actions

| Helper | Runs | Notes |
|---|---|---|
| `open(url, title?)` | in the launcher | URL, file or folder with the default app |
| `copy(text, title?)` | in the launcher | copies and closes |
| `paste(text, title?)` | in the launcher | pastes into the app the user was in (copies where the desktop doesn't allow it) |
| `action(title, fn)` | your code | return a view to navigate to it, or an effect |
| `submenu(title, actions)` | in the launcher | a second level in the action panel; pass a function to load it when it opens |

Options for all of them: `{ icon, shortcut: "<Ctrl><Shift>c", destructive: true, confirm: "Really?", section: "Copy" }`.
Actions with the same `section` are grouped under a header in Ctrl+K.
`confirm` asks in the footer before running, keyboard only.

**Effects** an action can return: `close(message?)`, `pop()`, `refresh()`
(reloads the current view), `replace(view)`. Returning a view pushes it;
Escape goes back.

## Services

```ts
import { clipboard, exec, openExternal, storage, toast } from "@backslash/api";

await storage.set("recent", ["react"]);       // private JSON key/value store, no permission needed
const recent = await storage.get<string[]>("recent");
await clipboard.write("text");
await clipboard.paste("text");                // into the app the user was in; closes the launcher
await toast("Saved", { style: "success" });   // "info" | "success" | "failure"
await openExternal("https://deno.com");
const out = await exec("git", ["status"]);    // program must be in permissions.run
```

`fetch`, `Deno.readTextFile` and the rest of the Deno and Web APIs work as
usual within the declared permissions. `console.log` goes to the log
(`backslash dev` prints it), never to the protocol.

## Signing in to a service (OAuth)

For services with OAuth 2.0, describe the provider and ask for a token; the
launcher does the rest (authorization code with PKCE, the browser, the
redirect, the refresh):

```ts
import { list, oauth } from "@backslash/api";

const linear = {
  id: "linear",
  title: "Linear",
  authorizeUrl: "https://linear.app/oauth/authorize",
  tokenUrl: "https://api.linear.app/oauth/token",
  clientId: "your-client-id",
  scopes: ["read"],
  scopeSeparator: ",",
};

export default list({
  items: async () => {
    const token = await oauth.token(linear); // signs in the first time
    const res = await fetch("https://api.linear.app/graphql", { headers: { Authorization: `Bearer ${token}` } /* … */ });
    // …
  },
});
```

- Register `http://127.0.0.1/callback` as the redirect URI with the service
  (RFC 8252: any port). If it insists on an exact match, set `redirectPort`
  and register `http://127.0.0.1:<port>/callback`.
- The token URL's host must be in your `[permissions] net`.
- Tokens are stored in the system keyring, refreshed when they expire, and
  never pass through your extension's storage. `oauth.savedToken(provider)`
  returns the token without ever opening the browser (or `undefined`), and
  `oauth.signOut(provider)` forgets it.
- `clientSecret` is there for services that want one even with PKCE; it
  ships with your extension, so treat it as public, as any desktop app does.
- Opening the browser usually hides the launcher. Once the user has signed
  in, the launcher comes back and runs your command again.
- From a terminal (`backslash run`), saved tokens work; signing in needs the
  launcher.

## Dependencies

Import from npm or JSR directly, or map names in `deno.json`:

```ts
import { format } from "npm:date-fns@4";
import { parse } from "jsr:@std/yaml@1";
```

The launcher merges the `imports` of your `deno.json` with `@backslash/api`,
which always points at the SDK shipped with the installed launcher, so an
extension can never run against a mismatched API.

## Other languages

The SDK is a convenience. Any program speaking the protocol over
stdin/stdout works, see [PROTOCOL.md](PROTOCOL.md):

```toml
[runtime]
kind = "exec"
command = ["python3"]   # the command entry path is appended
```

Extensions that are not run by Deno are sandboxed with
[bubblewrap](https://github.com/containers/bubblewrap) (the `bubblewrap`
package, required to run them). They see the system read-only, their own
folder read-only, an empty home and `/tmp`, the `read` and `write` paths and
`env` variables they declare, and no network unless `net` lists a host.
Bubblewrap cannot filter hosts or programs: declaring any host gives the whole
network, and any installed program can be started. The permission prompt says
so.

## Preferences and permissions

- Users set preferences in **Settings → Extensions → Configure**. If a
  `required` preference is missing, the form opens by itself before the command
  runs. Your command receives them in `ctx.preferences`.
- `type = "secret"` values are stored in the system keyring (libsecret: GNOME
  Keyring, KWallet, KeePassXC…), never in `config.toml`.
- The first time a command of your extension runs, the user sees what it asks
  for (the `[permissions]` lines) and approves it. Asking for more in an update
  asks again. Extensions declaring no permission run without a prompt.
  From a terminal, `backslash run` asks the same question (or `--yes`).

## Sharing and installing

Users install extensions from the **Browse Extensions** command, or:

```sh
backslash install github                          # from the registry
backslash install https://github.com/you/repo     # straight from git (not reviewed)
backslash install ./my-repo --path extensions/x   # a local repository
backslash update                                  # update everything installed
backslash search --refresh                        # wait for the latest registry
```

The list of extensions is a small git repository that Backslash keeps a copy
of. It is refreshed every hour, when **Browse Extensions** opens with a copy
older than that (the cached list shows first, then updates), and on demand with
**Refresh Extensions** (<kbd>Ctrl</kbd>+<kbd>R</kbd> in the store). `search`,
`update` and `outdated` refresh it first too, answering from the cached copy
when the registry doesn't respond within a few seconds; `--refresh` waits for
it and fails rather than use an old copy.

To list yours in the registry, push it to a public git repository and run
`backslash publish`: it prints the registry entry for your last commit (commit
hash, checksum of the files, permissions). Open a pull request adding it to
[backslash-app/plugins](https://github.com/backslash-app/plugins) as
`registry/<id>.toml`. Your code stays in your repository; Backslash installs
exactly the reviewed commit and refuses anything that differs.
The registry itself is signed: Backslash reads it only from commits signed
by the registry key, and never goes back to an older one than it has seen,
so a mirror can neither forge entries nor serve stale ones. A registry of
your own (`registry` in `config.toml`) needs its own keys,
`registry_signers = ["ssh-ed25519 AAAA…"]`, or `[]` to read it unchecked.

For local development, copy or symlink the folder into
`~/.local/share/backslash/extensions/`, or use `backslash dev`.
