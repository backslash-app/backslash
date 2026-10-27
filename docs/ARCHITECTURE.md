# Architecture

```
┌──────────────────────────── backslash (one binary) ─────────────────────────────┐
│  GTK4 window            CLI (`run`, `list`, `new`, `dev`, `doctor`)              │
│      │                       │                                                   │
│      └──────── backslash-core (no GUI, unit tested) ──────────────────────────┐  │
│               search (fuzzy + frecency) · calculator · manifests · config      │  │
│               registry (discovery) · host (process + JSON-RPC) · storage       │  │
└───────────────────────────────────────┬───────────────────────────────────────┘
                                        │ JSON-RPC, one line per message, stdin/stdout
              ┌─────────────────────────┼─────────────────────────┐
              ▼                         ▼                         ▼
   deno run --allow-net=…      deno run --allow-run=…        python3 main.py
   sdk/runtime.ts npm/search   sdk/runtime.ts system/…       (exec runtime)
```

## Principles

- **Extensions describe, the launcher draws.** Views are data (`List`, `Detail`,
  `Form`). Every extension looks native and consistent, keyboard handling is
  uniform, and the design can evolve without breaking anyone.
- **One process per running command.** A crash, a hang or a leak in an
  extension never reaches the launcher; leaving the command kills the process.
- **Capabilities are declared and enforced.** Manifest permissions become Deno
  flags (`--allow-net=api.github.com`), so the runtime enforces them, not
  goodwill. Storage, clipboard writes, toasts and opening URLs go through the
  launcher and need no permission.
- **Manifests are read without running code.** Searching commands is instant
  whatever the number of installed extensions.
- **The SDK ships with the launcher.** `@backslash/api` is mapped to the SDK of
  the installed version: no version skew, no `npm install`.
- **Plain files.** Config, preferences, usage history and storage are TOML/JSON
  under XDG directories, friendly to dotfiles.
- **Linux-native.** Applications come from GIO (all XDG rules, Flatpak and Snap
  exports, D-Bus activation), the color scheme from the settings portal,
  icons from the user's icon theme.

## Crates and folders

| Path | Role |
|---|---|
| `crates/backslash-core` | Protocol types, manifest parsing, extension host, ranking, calculator, config. No GTK. |
| `crates/backslash` | GTK4 window (`launcher.rs`), CLI (`cli.rs`), app discovery (`apps.rs`), theme (`style.css`). |
| `sdk/` | TypeScript API (`mod.ts`) and protocol runtime (`runtime.ts`), run by Deno. |
| `extensions/` | Bundled extensions, also the reference examples. |

## Why these choices

**GTK4 (gtk-rs) rather than a web view.** A launcher must open instantly and
type without lag, on every compositor. GTK4 is GPU-rendered, has first-class
Wayland and X11 support, input methods, accessibility and fractional scaling,
and is the toolkit `gtk4-layer-shell` targets. Tauri would run on
WebKitGTK, which on Linux is slower to start, heavier, and has known
rendering issues with some GPU drivers under Wayland. Since extensions never
touch the UI, this choice can be revisited without breaking a single extension.

**Deno rather than Node.** TypeScript without a build step, npm and JSR
imports without `node_modules`, fast startup, and a real permission model we
map the manifest onto.

**Plain views rather than a React renderer.** React in extensions means a
reconciler, hooks rules, and state living in the extension. Most launcher
commands are "load or search, then act", which values express directly; an
action returns the next view. State that must survive goes in `storage`.

## Lifecycle of a keystroke

1. Root view: GIO apps + manifest commands are ranked in-process
   (`search::Ranker`, nucleo fuzzy matching + usage boost). Calculator
   results are pinned on top.
2. Inside a host-filtered list: same ranker over the items the extension sent.
3. Inside an extension-searched list: debounced 120 ms, the previous request
   gets `$/cancel` (its `AbortSignal` fires), the newest answer wins.
