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
- [x] Clipboard history with pins, pasted straight into the app you were in (images still to do).
- [x] File search with recents and previews.
- [x] Window switcher for Hyprland, Sway, niri, KDE Plasma, GNOME and X11.
- [x] Unit and currency conversions.
- [x] Quicklinks and snippets, edited in the settings window.
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
- [x] Signed registry commits, so a compromised mirror cannot serve entries
      (needs the registry key: see CONTRIBUTING).
- [x] Grid layout for lists (emoji, images).
- [x] Rich preview panel for list items: stats, tags, people, links, progress.
- [x] List filters, footer status, command subtitles, action sections and submenus.
- [x] Backslash's own icon set, drawn in the theme colors.
- [x] Menu-bar/tray items.
- [x] Background commands on an interval.
- [x] OAuth helper (PKCE, loopback redirect, refresh, tokens in the keyring).
- [x] Hot reload in `backslash dev` (watch files, re-run the open command).
- [x] Sandbox exec-runtime extensions with bubblewrap.
- [x] Script commands: any executable with a few comment lines, in any language.
- [x] Python SDK.

## Quality
- [x] Welcome checklist on first launch, with the real state of each step on this desktop.
- [x] Settings window: shortcut, theme, accent color, density, extensions, clipboard, quicklinks, snippets.
- [x] More themes and window sizes.
- [ ] Localization (French first).
- [x] UI tests driving the window under a virtual display in CI.
