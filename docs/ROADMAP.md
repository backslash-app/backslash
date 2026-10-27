# Roadmap

What exists today is a solid base: the launcher window, the extension engine,
the CLI and two bundled extensions. Next steps, roughly in order.

## Desktop integration
- [ ] `gtk4-layer-shell` on wlroots compositors (Sway, Hyprland, river) and KDE,
      as an optional build feature: true overlay, no window rules needed.
- [ ] Global shortcut through the XDG `GlobalShortcuts` portal (KDE, GNOME 48+,
      Hyprland), keeping `backslash toggle` as the universal fallback.
- [ ] Autostart toggle from the launcher (writes the XDG autostart entry).
- [ ] Packages: Flatpak, AUR, `.deb`/`.rpm` via CI, Nix flake.

## Built-in features
- [ ] Clipboard history (with pinning and an ignore list for password managers).
- [ ] File search (`plocate`/`fd`, with recents from `recently-used.xbel`).
- [ ] Window switcher (Hyprland, Sway, KDE via D-Bus, GNOME via extension).
- [ ] Unit and currency conversions in the calculator.
- [ ] Quicklinks and snippets defined in `config.toml`.
- [ ] Aliases and per-command hotkeys.

## Extensions
- [ ] Settings view: enable/disable extensions, edit preferences, secrets
      stored with libsecret instead of `secrets.toml`.
- [ ] Ask for missing required preferences with a form on first run.
- [ ] Permission prompt when an extension is installed or its permissions grow.
- [ ] Registry: a git index of extensions pinned by commit hash,
      `backslash install <id>`, update checks, store view in the launcher.
- [ ] `Grid` view (emoji, images), list item detail pane, menu-bar/tray items.
- [ ] Background commands on an interval.
- [ ] OAuth helper (PKCE, loopback redirect).
- [ ] Hot reload in `backslash dev` (watch files, re-run the open command).
- [ ] Sandbox exec-runtime extensions with bubblewrap.
- [ ] Python SDK.

## Quality
- [ ] Settings window for theme and window size; more themes.
- [ ] Localization (French first).
- [ ] UI tests driving the window under a virtual display in CI.
