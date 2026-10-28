//! `backslash`: the launcher window, plus a CLI to run any command from a terminal.

mod apps;
mod autostart;
mod catalog_cli;
mod cli;
mod clipboard;
mod currency;
mod glyphs;
mod hotkey;
mod icons;
mod images;
mod kwin;
mod launcher;
mod layer;
mod markdown;
mod paste;
mod portal;
mod scaffold;
mod theme;
mod tray;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "backslash", version, about = "A fast, extensible launcher for Linux")]
struct Args {
    /// Start in the background without showing the window (for autostart).
    #[arg(long)]
    hidden: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show or hide the launcher (bind this to a key in your desktop settings).
    Toggle,
    /// Open an app, command or other root search item in the launcher, e.g.
    /// `builtin:clipboard`, `cmd:github/pull-requests`, `app:firefox.desktop`.
    /// Bind it to a key where the desktop has no shortcut portal.
    Open { key: String },
    /// Run an extension command and print its results.
    Run {
        /// `extension/command`, e.g. `npm/search`.
        id: String,
        /// Text typed in the search bar.
        query: Option<String>,
        /// Command argument as `name=value` (repeatable).
        #[arg(short, long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Approve the extension's permissions without asking.
        #[arg(short, long)]
        yes: bool,
    },
    /// List installed extensions and their commands.
    List,
    /// Create a new extension.
    New {
        /// Extension id, e.g. `github`.
        name: String,
        /// Parent directory (default: current directory).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Open the launcher with an extension loaded from a directory and show its logs.
    Dev {
        /// Extension directory (default: current directory).
        dir: Option<PathBuf>,
    },
    /// Check the installation: runtime, extensions, desktop integration.
    Doctor,
    /// Install an extension from the registry (`github`) or a git repository (URL or path).
    Install {
        /// Registry id, or git URL / local repository path.
        target: String,
        /// Branch, tag or commit (git sources).
        #[arg(long)]
        rev: Option<String>,
        /// Folder of the extension inside the repository (git sources).
        #[arg(long)]
        path: Option<String>,
        /// Don't ask for confirmation.
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove an extension installed in your user folder.
    Uninstall { id: String },
    /// Update installed extensions (all, or one).
    Update {
        id: Option<String>,
        #[arg(short, long)]
        yes: bool,
        /// Wait for the latest registry instead of falling back to the cached copy.
        #[arg(long)]
        refresh: bool,
    },
    /// List extensions with updates available.
    Outdated {
        /// Wait for the latest registry instead of falling back to the cached copy.
        #[arg(long)]
        refresh: bool,
    },
    /// Search the extension registry.
    Search {
        query: Option<String>,
        /// Wait for the latest registry instead of falling back to the cached copy.
        #[arg(long)]
        refresh: bool,
    },
    /// Print the registry entry for the extension in a git repository.
    Publish { dir: Option<PathBuf> },
    /// Download Deno, the runtime of TypeScript extensions, into Backslash's data folder.
    InstallRuntime,
}

fn main() {
    if let Err(err) = run(Args::parse()) {
        eprintln!("backslash: {err:#}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    if !matches!(args.command, None | Some(Cmd::Toggle) | Some(Cmd::Dev { .. }) | Some(Cmd::Open { .. })) {
        // Behave like other Unix tools in pipelines (`backslash run … | head`):
        // die quietly on a closed pipe instead of panicking.
        // SAFETY: called before any thread is spawned; SIG_DFL is always valid.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    match args.command {
        None | Some(Cmd::Toggle) => launcher::run(launcher::Options { hidden: args.hidden, dev_dir: None, open: None }),
        Some(Cmd::Open { key }) => launcher::run(launcher::Options { hidden: false, dev_dir: None, open: Some(key) }),
        Some(Cmd::Dev { dir }) => {
            let dir = dir.unwrap_or_else(|| PathBuf::from(".")).canonicalize()?;
            launcher::run(launcher::Options { hidden: false, dev_dir: Some(dir), open: None })
        }
        Some(Cmd::Run { id, query, args, json, yes }) => {
            cli::run(&id, query.as_deref().unwrap_or(""), &args, json, yes)
        }
        Some(Cmd::List) => cli::list(),
        Some(Cmd::New { name, dir }) => scaffold::create(&name, dir.as_deref()),
        Some(Cmd::Doctor) => cli::doctor(),
        Some(Cmd::Install { target, rev, path, yes }) => catalog_cli::install(&target, rev, path, yes),
        Some(Cmd::Uninstall { id }) => catalog_cli::uninstall(&id),
        Some(Cmd::Update { id, yes, refresh }) => catalog_cli::update(id.as_deref(), yes, refresh),
        Some(Cmd::Outdated { refresh }) => catalog_cli::outdated(refresh),
        Some(Cmd::Search { query, refresh }) => catalog_cli::search(query.as_deref(), refresh),
        Some(Cmd::Publish { dir }) => catalog_cli::publish(dir.as_deref()),
        Some(Cmd::InstallRuntime) => cli::install_runtime(),
    }
}
