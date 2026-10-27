# Roadmap

What exists today is a solid base: the launcher window, the extension engine,
the CLI and two bundled extensions. Next steps, roughly in order.

## Desktop integration
- [x] `gtk4-layer-shell` overlay when the library is installed (loaded at runtime).
      Needs testing on real Sway/Hyprland/KDE sessions.
- [x] Global shortcut through the XDG `GlobalShortcuts` portal, with
      `backslash` as the universal fallback.
- [x] Launch at login toggle from the launcher.
- [x] Packages: `.deb`, `.rpm` and tarball built on tags, AUR `PKGBUILD`, Deno downloaded on demand.
- [ ] Submit to the AUR, Nix flake, apt/dnf repositories so updates come with the system.

## Built-in features
- [x] Clipboard history with pins (images and paste-into-app still to do).
- [x] File search with recents and previews.
- [x] Window switcher for Hyprland, Sway, niri, X11. KDE and GNOME to do.
- [x] Unit and currency conversions.
- [x] Quicklinks and snippets in `config.toml` (an editor in the launcher to do).
- [x] Light, dark and system themes with a preview panel.
- [x] Aliases and per-item global shortcuts, `backslash open <item>`.

## Extensions
- [x] Settings view: enable/disable extensions, edit preferences, secrets
      in the system keyring (libsecret), quicklinks, snippets, hidden apps.
- [x] Ask for missing required preferences with a form on first run.
- [x] Permission prompt on first run and when permissions grow.
- [x] Registry: a git index of extensions pinned by commit hash and checksum,
      `backslash install/update/uninstall/search/publish`, store view.
- [x] Update badge on Browse Extensions (registry checked every 6 hours), Update All.
- [ ] Signed registry commits, so a compromised mirror cannot serve entries.
- [x] Grid layout for lists (emoji, images).
- [ ] List item detail pane, menu-bar/tray items.
- [ ] Background commands on an interval.
- [ ] OAuth helper (PKCE, loopback redirect).
- [x] Hot reload in `backslash dev` (watch files, re-run the open command).
- [ ] Sandbox exec-runtime extensions with bubblewrap.
- [ ] Python SDK.

## Quality
- [x] Welcome checklist on first launch, with the real state of each step on this desktop.
- [ ] Settings window for theme and window size; more themes.
- [ ] Localization (French first).
- [ ] UI tests driving the window under a virtual display in CI.
