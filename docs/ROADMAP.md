# Roadmap

What exists today is a solid base: the launcher window, the extension engine,
the CLI and two bundled extensions. Next steps, roughly in order.

## Desktop integration
- [x] `gtk4-layer-shell` overlay when the library is installed (loaded at runtime).
      Needs testing on real Sway/Hyprland/KDE sessions.
- [x] Global shortcut through the XDG `GlobalShortcuts` portal, with
      `backslash` as the universal fallback.
- [x] Launch at login toggle from the launcher.
- [ ] Packages: Flatpak, AUR, `.deb`/`.rpm` via CI, Nix flake.

## Built-in features
- [x] Clipboard history with pins (images and paste-into-app still to do).
- [x] File search with recents and previews.
- [x] Window switcher for Hyprland, Sway, niri, X11. KDE and GNOME to do.
- [x] Unit and currency conversions.
- [x] Quicklinks and snippets in `config.toml` (an editor in the launcher to do).
- [x] Light, dark and system themes with a preview panel.
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
