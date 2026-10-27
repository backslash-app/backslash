<p align="center"><img src="data/icons/backslash.png" width="96" alt=""/></p>

<h1 align="center">Backslash</h1>

<p align="center"><b>A fast, keyboard-first launcher for Linux, with extensions you can write in five minutes.</b></p>

<p align="center">
  <a href="docs/EXTENSIONS.md">Write an extension</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/ROADMAP.md">Roadmap</a> ·
  <a href="https://discord.gg/sTzwBzDkK9">Discord</a>
</p>

![Backslash](docs/screenshot.png)

> [!IMPORTANT]
> Backslash 0.3 is a ground-up rewrite (Rust + GTK4, Deno extensions). It is
> young: expect rough edges, and see the [roadmap](docs/ROADMAP.md).

## Why Backslash

- **Native and instant.** One small Rust binary, GTK4, GPU-rendered. It stays
  in the background and opens with no delay, on Wayland and X11.
- **Made for Linux.** Applications come straight from your system (Flatpak and
  Snap included), icons from your icon theme, light/dark from your desktop.
- **Extensions that cannot hurt you.** Each extension declares what it needs
  (`net = ["api.github.com"]`, `run = ["git"]`) and the runtime enforces it.
  Each command runs in its own process: a buggy extension can't freeze or
  crash the launcher.
- **Extensions that are easy to write.** TypeScript, no build step, no
  `node_modules`. Describe a list, the launcher does the rest: fuzzy search,
  keyboard navigation, action panel, cancellation of slow requests.
- **Scriptable.** Every command also runs from a terminal:
  `backslash run npm/search react --json | jq`.
- **Yours.** Open source (MIT), no account, no telemetry, no paywall. Config
  and theme are plain files you can keep in your dotfiles.

## A complete extension

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
import { copy, list, open } from "@backslash/api";

export default list({
  placeholder: "Search npm packages…",
  async search(query, { signal }) {
    const res = await fetch(`https://registry.npmjs.org/-/v1/search?text=${query}`, { signal });
    const { objects } = await res.json();
    return objects.map(({ package: pkg }) => ({
      title: pkg.name,
      subtitle: pkg.description,
      tags: [`v${pkg.version}`],
      actions: [open(pkg.links.npm), copy(`npm install ${pkg.name}`, "Copy Install Command")],
    }));
  },
});
```

That's it. `backslash new <name>` scaffolds one, `backslash dev` loads it with
logs in your terminal. Read the [extension guide](docs/EXTENSIONS.md).

## Install

Requirements: GTK 4.14+, [Deno](https://deno.com) 2 (runs extensions), Rust to build.

```sh
# Debian/Ubuntu: sudo apt install libgtk-4-dev    Fedora: sudo dnf install gtk4-devel
# Arch: sudo pacman -S gtk4                        Deno: see https://docs.deno.com/runtime/getting_started/installation/
git clone https://github.com/backslash-app/backslash && cd backslash
make && sudo make install          # or: make install PREFIX=~/.local
backslash doctor                   # checks everything is in place
```

### Open it with a shortcut

Bind a key (e.g. <kbd>Super</kbd>+<kbd>Space</kbd>) to the command `backslash`.
Running it again while the window is open hides it.

| Desktop | Where |
|---|---|
| GNOME | Settings → Keyboard → Custom Shortcuts → command `backslash` |
| KDE Plasma | System Settings → Shortcuts → Add New → Command `backslash` |
| Hyprland | `bind = SUPER, SPACE, exec, backslash` and `windowrulev2 = float, class:^(app\.backslash\.Backslash)$` |
| Sway / i3 | `bindsym $mod+space exec backslash` and `for_window [app_id="app.backslash.Backslash"] floating enable` |

For instant opening, start it with your session: copy
`data/backslash-autostart.desktop` to `~/.config/autostart/`.

## Using it

| Key | Action |
|---|---|
| type | search apps, commands; math is computed as you type |
| <kbd>↑</kbd> <kbd>↓</kbd> / <kbd>Ctrl</kbd>+<kbd>N</kbd> <kbd>P</kbd> | move |
| <kbd>Enter</kbd> / <kbd>Ctrl</kbd>+<kbd>Enter</kbd> | primary / secondary action |
| <kbd>Ctrl</kbd>+<kbd>K</kbd> | all actions |
| <kbd>Esc</kbd> / <kbd>Backspace</kbd> on empty search | back, then close |

Results learn from you: what you pick for a given query comes first next time.

## Make it yours

- `~/.config/backslash/style.css`: override the theme (see
  [`style.css`](crates/backslash/src/style.css) for the classes and colors).
- `~/.config/backslash/config.toml`: hidden apps, disabled extensions, preferences.
- `~/.local/share/backslash/extensions/`: drop extension folders here.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bundled extensions in
[`extensions/`](extensions/) are good examples to start from.

## License

[MIT](LICENSE)
