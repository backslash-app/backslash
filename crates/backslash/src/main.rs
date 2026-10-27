//! `backslash`: the launcher window, plus a CLI to run any command from a terminal.

mod apps;
mod autostart;
mod cli;
mod clipboard;
mod currency;
mod hotkey;
mod icons;
mod launcher;
mod layer;
mod markdown;
mod scaffold;
mod theme;

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
}

fn main() {
    if let Err(err) = run(Args::parse()) {
        eprintln!("backslash: {err:#}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    if !matches!(args.command, None | Some(Cmd::Toggle) | Some(Cmd::Dev { .. })) {
        // Behave like other Unix tools in pipelines (`backslash run … | head`):
        // die quietly on a closed pipe instead of panicking.
        // SAFETY: called before any thread is spawned; SIG_DFL is always valid.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    match args.command {
        None | Some(Cmd::Toggle) => launcher::run(launcher::Options { hidden: args.hidden, dev_dir: None }),
        Some(Cmd::Dev { dir }) => {
            let dir = dir.unwrap_or_else(|| PathBuf::from(".")).canonicalize()?;
            launcher::run(launcher::Options { hidden: false, dev_dir: Some(dir) })
        }
        Some(Cmd::Run { id, query, args, json, yes }) => {
            cli::run(&id, query.as_deref().unwrap_or(""), &args, json, yes)
        }
        Some(Cmd::List) => cli::list(),
        Some(Cmd::New { name, dir }) => scaffold::create(&name, dir.as_deref()),
        Some(Cmd::Doctor) => cli::doctor(),
    }
}
