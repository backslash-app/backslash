<p align="center"><img src="docs/media/logo.svg" width="112" alt="Backslash logo"/></p>

<h1 align="center">Backslash</h1>

<p align="center"><b>The launcher your Linux desktop deserves.</b><br/>
Apps, files, clipboard, math, and your whole dev stack, one keystroke away.</p>

<p align="center">
  <a href="https://github.com/backslash-app/backslash/releases/latest"><img src="https://img.shields.io/github/v/release/backslash-app/backslash?style=flat-square&color=e81cff&label=release" alt="Latest release"/></a>
  <a href="https://github.com/backslash-app/plugins"><img src="https://img.shields.io/badge/extensions-growing_every_week-7a22d6?style=flat-square" alt="Extensions"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3b1d8f?style=flat-square" alt="MIT"/></a>
  <a href="https://discord.gg/sTzwBzDkK9"><img src="https://img.shields.io/badge/chat-Discord-5865f2?style=flat-square" alt="Discord"/></a>
</p>

<p align="center">
  <a href="docs/media/demo.mp4"><img src="docs/media/demo.gif" alt="Backslash in action: Linear, GitHub, Kubernetes, Git and Dev Utils from the keyboard" width="100%"/></a>
  <br/><sub><a href="docs/media/demo.mp4">▶ Watch it in HD</a></sub>
</p>

<p align="center">
  <a href="#install"><b>Install</b></a> ·
  <a href="https://github.com/backslash-app/plugins"><b>Extensions</b></a> ·
  <a href="docs/USAGE.md">Guide</a> ·
  <a href="docs/EXTENSIONS.md">Write an extension</a>
</p>

---

## What is it?

Press <kbd>Super</kbd>+<kbd>Space</kbd>, type a few letters, hit <kbd>Enter</kbd>.
Backslash is a native, keyboard-first launcher for Linux (in the spirit of
Raycast and Alfred) that opens instantly and goes much further than starting
apps: review pull requests, triage issues, find the crashing pod or decode a
JWT without leaving the keyboard.

<table>
  <tr>
    <td width="50%"><img src="docs/media/github.png" alt="GitHub pull requests"/><p align="center"><b>GitHub</b>: pull requests with checks and reviews</p></td>
    <td width="50%"><img src="docs/media/kubernetes.png" alt="Kubernetes pods"/><p align="center"><b>Kubernetes</b>: problems first, logs one key away</p></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/media/linear.png" alt="Linear issues"/><p align="center"><b>Linear</b>: triage, assign, change status</p></td>
    <td width="50%"><img src="docs/media/git.png" alt="Git repositories"/><p align="center"><b>Git Repos</b>: every repo, branch and pending change</p></td>
  </tr>
</table>

## Why you'll like it

|  |  |
|---|---|
| ⚡ **Instant** | One small Rust + GTK4 binary that stays in the background. Wayland and X11. |
| 🐧 **Made for Linux** | Your apps (Flatpak and Snap too), your icon theme, your light or dark mode. GNOME, KDE, Hyprland, Sway, niri… |
| 🧩 **Extensions for everything** | A large and growing library, installed from the built-in store. Each one says what it may access, you approve it once, and the sandbox enforces it. |
| 🛠️ **Built for developers** | Extensions in TypeScript, no build step. Scripts in any language become commands. Everything runs from the terminal too: `backslash run github/pull-requests --json`. |
| 📋 **Useful from day one** | Calculator and conversions as you type, clipboard history, file search, window switcher, snippets and quicklinks. |
| 🔓 **Yours** | Open source (MIT). No account, no telemetry, no paywall. Plain config files for your dotfiles. |

## Extensions for developers

A few of the [many extensions](https://github.com/backslash-app/plugins#first-party-extensions), with more added all the time:

| | |
|---|---|
| **Code and reviews** | [GitHub](https://github.com/backslash-app/plugins/tree/main/extensions/github) · [GitLab](https://github.com/backslash-app/plugins/tree/main/extensions/gitlab) · [Codeberg](https://github.com/backslash-app/plugins/tree/main/extensions/codeberg) · [Git Repos](https://github.com/backslash-app/plugins/tree/main/extensions/git-repos) · [Code Projects](https://github.com/backslash-app/plugins/tree/main/extensions/projects) |
| **Issues and errors** | [Linear](https://github.com/backslash-app/plugins/tree/main/extensions/linear) · [Jira](https://github.com/backslash-app/plugins/tree/main/extensions/jira) · [Sentry](https://github.com/backslash-app/plugins/tree/main/extensions/sentry) · [Trello](https://github.com/backslash-app/plugins/tree/main/extensions/trello) · [ClickUp](https://github.com/backslash-app/plugins/tree/main/extensions/clickup) |
| **Cloud and deploys** | [Kubernetes](https://github.com/backslash-app/plugins/tree/main/extensions/kubernetes) · [AWS](https://github.com/backslash-app/plugins/tree/main/extensions/aws) · [Google Cloud](https://github.com/backslash-app/plugins/tree/main/extensions/google-cloud) · [Cloudflare](https://github.com/backslash-app/plugins/tree/main/extensions/cloudflare) · [Vercel & Netlify](https://github.com/backslash-app/plugins/tree/main/extensions/deployments) |
| **Local stack** | [Containers](https://github.com/backslash-app/plugins/tree/main/extensions/docker) · [Ports](https://github.com/backslash-app/plugins/tree/main/extensions/ports) · [Databases](https://github.com/backslash-app/plugins/tree/main/extensions/databases) · [Services](https://github.com/backslash-app/plugins/tree/main/extensions/systemd) · [System Logs](https://github.com/backslash-app/plugins/tree/main/extensions/system-logs) · [SSH Hosts](https://github.com/backslash-app/plugins/tree/main/extensions/ssh) · [Terminal Sessions](https://github.com/backslash-app/plugins/tree/main/extensions/terminal-sessions) · [Distrobox & VMs](https://github.com/backslash-app/plugins/tree/main/extensions/distrobox) |
| **Everyday tools** | [Dev Utils](https://github.com/backslash-app/plugins/tree/main/extensions/dev-utils) · [HTTP Client](https://github.com/backslash-app/plugins/tree/main/extensions/http-client) · [Regex Tester](https://github.com/backslash-app/plugins/tree/main/extensions/regex) · [Cron Helper](https://github.com/backslash-app/plugins/tree/main/extensions/cron) · [Dev Reference](https://github.com/backslash-app/plugins/tree/main/extensions/dev-reference) · [Docs & tldr](https://github.com/backslash-app/plugins/tree/main/extensions/docs) · [Package Registries](https://github.com/backslash-app/plugins/tree/main/extensions/package-registries) · [Nerd Font Icons](https://github.com/backslash-app/plugins/tree/main/extensions/nerd-fonts) |
| **AI** | [AI](https://github.com/backslash-app/plugins/tree/main/extensions/ai) (Claude, OpenAI, Mistral, Ollama…) · [Ollama Models](https://github.com/backslash-app/plugins/tree/main/extensions/ollama) |

<p align="center"><img src="docs/media/dev-utils.png" width="80%" alt="Dev Utils decoding a JWT"/><br/><sub>Paste a JWT, a timestamp or some Base64: Dev Utils already knows what it is.</sub></p>

Missing one? An extension is a TOML file and a TypeScript file, and
`backslash new` writes them for you. See the [extension guide](docs/EXTENSIONS.md).

## Install

Backslash needs GTK 4.14 or newer (Ubuntu 24.04+, Debian 13+, Fedora 40+, Arch,
openSUSE Tumbleweed…). Get the package for your system from the
[latest release](https://github.com/backslash-app/backslash/releases/latest):

| Distribution | Command |
|---|---|
| Debian, Ubuntu | `sudo apt install ./backslash_<version>_amd64.deb` |
| Fedora, openSUSE | `sudo dnf install ./backslash-<version>-1.x86_64.rpm` |
| Arch | `makepkg -si` with [`packaging/aur/PKGBUILD`](packaging/aur/PKGBUILD) |
| Anything else | `tar -xzf backslash-<version>-x86_64.tar.gz -C ~/.local` |

Then open it with <kbd>Super</kbd>+<kbd>Space</kbd>: a short welcome checklist
sets up the shortcut, theme and launch at login. The [guide](docs/USAGE.md)
covers shortcuts on every desktop, keys, script commands, configuration and
building from source.

## Contributing

Ideas, bug reports and pull requests are welcome: see
[CONTRIBUTING.md](CONTRIBUTING.md), the [architecture](docs/ARCHITECTURE.md) and
the [roadmap](docs/ROADMAP.md). Extensions go to
[backslash-app/plugins](https://github.com/backslash-app/plugins).

## License

[MIT](LICENSE)
