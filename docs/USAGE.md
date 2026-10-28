# Using Backslash

- [First run](#first-run)
- [Updates and the tray icon](#updates-and-the-tray-icon)
- [Open it with a shortcut](#open-it-with-a-shortcut)
- [Using it](#using-it)
- [Script commands](#script-commands)
- [Make it yours](#make-it-yours)
- [Write an extension](#write-an-extension)
- [Build from source](#build-from-source)
- [Why no Flatpak?](#why-no-flatpak)

## First run

The first time it opens, Backslash shows a short **welcome** checklist: the
shortcut (with the steps for your desktop when it can't register one itself),
theme, launch at login, clipboard history, the overlay on Wayland, and the
extension runtime. Open it again any time with **Welcome to Backslash**.

Extensions written in TypeScript run in [Deno](https://deno.com). If it isn't
installed, Backslash offers to download it the first time you need it (or run
`backslash install-runtime`); it comes from Deno's official npm package and is
checked before use. `backslash doctor` checks that everything is in place.

## Updates and the tray icon

Backslash checks GitHub for a new version when it starts, then every six
hours. When one is out, **Backslash 0.5.0 is available** shows at the bottom
left of the window and in the tray menu. Click it, or run **Update Backslash**:
you see what's new, and **Update and Restart** does the rest. The download is
checked against the release's `SHA256SUMS`, then installed the way Backslash
was installed: with apt or dnf/zypper for the `.deb` and `.rpm` (your system
asks for your password), or unpacked in place for the tarball. Installed from
the AUR or from source, you're told how to update instead.
**Check for Updates** looks right away.

While it runs, Backslash shows an icon in the system tray, with a menu to open
it, open the settings, update and quit. KDE Plasma, Xfce, Cinnamon, Budgie and
bars like waybar show it as is; on GNOME it needs the
[AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/)
(already there on Ubuntu). Both can be turned off in **Settings → General**,
or with `tray = false` and `check_updates = false` in `config.toml`.

## Open it with a shortcut

Backslash asks your desktop for <kbd>Super</kbd>+<kbd>Space</kbd> through the
GlobalShortcuts portal (KDE Plasma, GNOME 48+, Hyprland…): confirm it once when
asked. Change it with `hotkey = "Ctrl+Space"` in `config.toml`.

Without the portal, bind a key to the command `backslash` (running it again
toggles the window):

| Desktop | Where |
|---|---|
| GNOME | Settings → Keyboard → Custom Shortcuts → command `backslash` |
| KDE Plasma | System Settings → Shortcuts → Add New → Command `backslash` |
| Hyprland | `bind = SUPER, SPACE, exec, backslash` |
| Sway / i3 | `bindsym $mod+space exec backslash` |

On Sway, Hyprland, KDE and other wlroots-style compositors, install
`gtk4-layer-shell` and Backslash opens as a true overlay, with no window rules
needed. Without it, make the window float:
`windowrulev2 = float, class:^(app\.backslash\.Backslash)$` (Hyprland) or
`for_window [app_id="app.backslash.Backslash"] floating enable` (Sway).

Run the **Enable Launch at Login** command so Backslash is always ready.

## Using it

| Key | Action |
|---|---|
| type | search apps and commands; math and conversions are computed as you type |
| <kbd>Tab</kbd> | filter: All, Apps, Commands (and an extension's own filters) |
| <kbd>↑</kbd> <kbd>↓</kbd> / <kbd>Ctrl</kbd>+<kbd>N</kbd> <kbd>P</kbd> | move |
| <kbd>Enter</kbd> / <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | primary / secondary action |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | all actions; <kbd>→</kbd> opens a submenu, <kbd>←</kbd> goes back |
| <kbd>Esc</kbd> / <kbd>Backspace</kbd> on empty search | back, then close |

Results learn from you: what you pick for a given query comes first next time.

Any app or command can get an **alias** and its own **global shortcut**:
<kbd>Ctrl</kbd>+<kbd>K</kbd> on it, then **Set Alias…** (type `ff`, Firefox comes
first) or **Set Hotkey…** (e.g. <kbd>Super</kbd>+<kbd>V</kbd> for the clipboard
history). Without the shortcuts portal, bind `backslash open builtin:clipboard`
(or `app:…`, `cmd:extension/command`) to a key in your desktop settings.

Built in:

- **Calculator and conversions**: `12*(3+4)`, `5 km in miles`, `72 f to c`,
  `2 GiB en MB`, `20 € en $` (European Central Bank rates, cached).
- **Clipboard History**: searchable, with pins; Enter pastes into the app you
  were in (xdotool on X11, the desktop's permission once on GNOME and KDE,
  `wtype` on Sway/Hyprland/niri). Recorded in the background on
  wlroots compositors, KDE and Hyprland (install `wl-clipboard`) and on X11; on
  GNOME, each time Backslash opens. Password managers' copies are skipped.
- **Search Files**: in your home folder (`fd` if installed), recent files first,
  with a preview.
- **Switch Windows**: Hyprland, Sway, niri, KDE Plasma (through a KWin
  script, nothing to install), GNOME (through a tiny Shell extension the
  launcher installs for you, or "Window Calls") and X11.
- **Settings**: a window for the shortcut, theme, accent color, density,
  extensions, clipboard history, quicklinks and snippets.

## Script commands

Any script can be a command, in any language, without the SDK. Put it in
`~/.config/backslash/scripts/` (or run **Create Script Command**), make it
executable, and describe it in its first comment lines:

```sh
#!/bin/bash
# title: Restart Wi-Fi
# description: Turns Wi-Fi off and on again
# icon: 📶
# mode: silent
nmcli radio wifi off && nmcli radio wifi on
```

| Key | Meaning |
|---|---|
| `title` | Required: what you search for |
| `mode` | `silent` (runs in the background, tells you only if it fails), `output` (shows what it prints), `list` (one item per printed line, to paste or copy), `paste` (pastes what it prints) |
| `argument` | Asked before running, passed as `$1`, `$2`…: `# argument: name Placeholder`, or `name?` when optional; repeatable |
| `description`, `icon`, `keywords` | Shown and searched |
| `confirm` | A question asked before running |

`//` and `--` comments work too (Node, Lua…). Script commands get aliases and
hotkeys like everything else. They run as you, unsandboxed: they are your own
files.

## Make it yours

Everything is in the **Settings** window (<kbd>Ctrl</kbd>+<kbd>,</kbd>, or the
Settings command): shortcut, launch at login, theme, accent color and density,
clipboard history, extensions (enable, aliases and hotkeys of each command,
permissions, preferences), quicklinks, snippets and hidden apps. It all lives in plain files, so you can
also edit them by hand or keep them in your dotfiles.

`~/.config/backslash/config.toml`:

```toml
theme = "system"          # "light", "dark" or "system"
accent = "indigo"         # "blue", "green", "orange", "pink" or "graphite"
density = "comfortable"   # or "compact"
hotkey = "Super+Space"
tray = true               # the icon in the system tray
check_updates = true      # look for new versions of Backslash
hidden_apps = ["org.gnome.Tour.desktop"]

[[quicklinks]]            # type "gh tokio" to search GitHub
name = "Search GitHub"
url = "https://github.com/search?q={query}"
keyword = "gh"

[[snippets]]              # {date}, {time} and {clipboard} are expanded
name = "Email signature"
text = "Best,\nSteven"
keyword = "sig"

[extensions.npm]
enabled = true

[aliases]                 # type "ff" to get Firefox first
"app:org.mozilla.firefox.desktop" = "ff"

[hotkeys]                 # global shortcuts that open an item
"builtin:clipboard" = "Super+V"
"cmd:github/pull-requests" = "Super+G"
```

- `~/.config/backslash/style.css`: override any style or color, e.g.
  `@define-color bs_accent #c2410c;` (see [`style.css`](../crates/backslash/src/style.css)
  and the palettes in [`theme.rs`](../crates/backslash/src/theme.rs)).
- `~/.local/share/backslash/extensions/`: drop extension folders here.
- `backslash doctor` shows what works on your desktop.

## Write an extension

`backslash.toml`:

```toml
[extension]
id = "npm"
title = "npm"

[permissions]
net = ["registry.npmjs.org"]

[[commands]]
name = "search"
title = "Search npm Packages"
```

`commands/search.ts`:

```ts
import { copy, Icons, list, meta, open } from "@backslash/api";

export default list({
  placeholder: "Search npm packages…",
  async search(query, { signal }) {
    const res = await fetch(`https://registry.npmjs.org/-/v1/search?text=${query}`, { signal });
    const { objects } = await res.json();
    return objects.map(({ package: pkg, downloads }) => ({
      title: pkg.name,
      subtitle: pkg.description,
      icon: { source: Icons.package, tint: "red" },
      tags: [`v${pkg.version}`],
      detail: {
        stats: [{ label: "Weekly downloads", value: downloads.weekly.toLocaleString() }],
        metadata: [meta.text("License", pkg.license), meta.person("Publisher", pkg.publisher.username)],
      },
      actions: [open(pkg.links.npm), copy(`npm install ${pkg.name}`, "Copy Install Command")],
    }));
  },
});
```

That's it. `backslash new <name>` scaffolds one, `backslash dev` loads it with
logs in your terminal and reloads it when you save. `backslash publish` prepares
its registry entry. Read the [extension guide](EXTENSIONS.md).

## Build from source

```sh
# Debian/Ubuntu: sudo apt install libgtk-4-dev    Fedora: sudo dnf install gtk4-devel    Arch: sudo pacman -S gtk4
git clone https://github.com/backslash-app/backslash && cd backslash
make && sudo make install          # or: make install PREFIX=~/.local
```

`make dist` builds the release files in `dist/` (it needs
[nfpm](https://nfpm.goreleaser.com) for the `.deb` and `.rpm`). Releases are
made by CI, see [Releasing](../CONTRIBUTING.md#releasing).

## Why no Flatpak?

A launcher has to start other apps, read the list of
installed ones, watch the clipboard and run the tools extensions ask for.
Inside the Flatpak sandbox all of this goes through `flatpak-spawn --host`,
which gives the whole system away anyway while making everything slower and
harder to debug. Extensions get their own, finer sandbox from Deno instead.
