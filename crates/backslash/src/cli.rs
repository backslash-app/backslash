//! Terminal side: every command is scriptable (`backslash run npm/search react --json | jq`).

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use backslash_core::config::Config;
use backslash_core::host::{deno_binary, Incoming, Pending, RpcError, Session};
use backslash_core::protocol::{Effect, Filtering, Item, View};
use backslash_core::registry::Registry;
use backslash_core::search::{Ranker, Searchable};
use backslash_core::{paths, runner};
use serde_json::{json, Map, Value};

const TIMEOUT: Duration = Duration::from_secs(30);

pub fn registry() -> Registry {
    Registry::scan(&paths::extension_dirs())
}

/// Waits for a reply while serving what the extension asks for in the meantime.
fn wait(session: &Session, pending: Pending) -> Result<Value> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(reply) = pending.try_get() {
            return reply.map_err(|e| anyhow!("{}: {e}", session.extension_id));
        }
        while let Ok(msg) = session.incoming.try_recv() {
            serve(session, msg);
        }
        if Instant::now() > deadline {
            bail!("{} did not answer within {}s", session.extension_id, TIMEOUT.as_secs());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn serve(session: &Session, msg: Incoming) {
    match msg {
        Incoming::Log(line) => eprintln!("[{}] {line}", session.extension_id),
        Incoming::Request { id, method, params } => {
            let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let result = match method.as_str() {
                "toast" => {
                    eprintln!("{}", [text("title"), text("message")].join(" ").trim());
                    Ok(Value::Null)
                }
                "open" => std::process::Command::new("xdg-open")
                    .arg(text("target"))
                    .spawn()
                    .map(|_| Value::Null)
                    .map_err(|e| RpcError::new(-32000, e.to_string())),
                "clipboard/write" => {
                    println!("{}", text("text"));
                    Ok(Value::Null)
                }
                _ => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, format!("{method} is not available in the CLI"))),
            };
            session.respond(id, result);
        }
        Incoming::Notification { .. } | Incoming::Exited(_) => {}
    }
}

struct Row<'a>(&'a Item);
impl Searchable for Row<'_> {
    fn title(&self) -> &str {
        &self.0.title
    }
    fn keywords(&self) -> &[String] {
        &self.0.keywords
    }
    fn subtitle(&self) -> Option<&str> {
        self.0.subtitle.as_deref()
    }
}

pub fn run(id: &str, query: &str, raw_args: &[String], json_out: bool) -> Result<()> {
    let registry = registry();
    let config = Config::load();
    let (ext, command) =
        registry.resolve(id).with_context(|| format!("unknown command `{id}` (see `backslash list`)"))?;

    let mut raw = Map::new();
    for pair in raw_args {
        let (k, v) = pair.split_once('=').with_context(|| format!("expected NAME=VALUE, got `{pair}`"))?;
        raw.insert(k.to_string(), v.into());
    }
    let arguments = runner::parse_arguments(command, &raw).map_err(|e| anyhow!(e))?;
    let (session, pending) = runner::start(ext, command, &config, arguments, query)?;
    let effect: Effect = serde_json::from_value(wait(&session, pending)?)?;

    let view = match effect {
        Effect::Push { view } | Effect::Replace { view } => view,
        Effect::Close { message } => {
            if let Some(message) = message {
                println!("{message}");
            }
            return Ok(());
        }
        Effect::None | Effect::Pop => return Ok(()),
    };

    let view = match view {
        View::List(list) if list.filtering == Filtering::Extension && !query.is_empty() => {
            let pending = session.request("search", json!({ "view": list.id, "query": query }))?;
            serde_json::from_value(wait(&session, pending)?)?
        }
        // Progressive lists: wait for the final update.
        View::List(list) if list.loading => loop {
            match session.incoming.recv_blocking() {
                Ok(Incoming::Notification { method, params }) if method == "view/update" => {
                    let view: View = serde_json::from_value(params["view"].clone())?;
                    if matches!(&view, View::List(l) if !l.loading) {
                        break view;
                    }
                }
                Ok(msg) => serve(&session, msg),
                Err(_) => bail!("{id} exited"),
            }
        },
        other => other,
    };

    if json_out {
        println!("{}", serde_json::to_string_pretty(&view)?);
        return Ok(());
    }
    match view {
        View::List(list) => {
            let rows: Vec<Row> = list.items.iter().map(Row).collect();
            let order: Vec<usize> = if list.filtering == Filtering::Host && !query.is_empty() {
                Ranker::default().rank(query, &rows, None).into_iter().map(|(i, _)| i).collect()
            } else {
                (0..rows.len()).collect()
            };
            for i in order {
                let item = &list.items[i];
                let tags: Vec<String> =
                    item.accessories.iter().filter_map(|a| a.tag.clone().or_else(|| a.text.clone())).collect();
                println!(
                    "{}{}{}",
                    item.title,
                    item.subtitle.as_deref().map(|s| format!("  — {s}")).unwrap_or_default(),
                    if tags.is_empty() { String::new() } else { format!("  [{}]", tags.join("] [")) }
                );
            }
        }
        View::Detail(detail) => {
            println!("{}", detail.markdown);
            for m in detail.metadata {
                println!("{}: {}", m.label, m.value);
            }
        }
        View::Form(form) => {
            bail!("{id} shows a form ({} fields); open it in the launcher", form.fields.len())
        }
    }
    Ok(())
}

pub fn list() -> Result<()> {
    let registry = registry();
    let config = Config::load();
    for ext in &registry.extensions {
        let m = &ext.manifest;
        let state = if config.is_enabled(&m.extension.id) { "" } else { " (disabled)" };
        println!("{} {} — {}{state}", m.extension.id, m.extension.version, m.extension.title);
        println!("  {}", ext.dir.display());
        if matches!(m.runtime, backslash_core::manifest::Runtime::Exec { .. }) {
            println!("  ! Not sandboxed: runs as a regular program");
        }
        for p in m.permissions.describe() {
            println!("  ! {p}");
        }
        for c in &m.commands {
            let args: String = c.arguments.iter().map(|a| format!(" <{}>", a.name)).collect();
            println!("    {}/{}{args}  {}", m.extension.id, c.name, c.title);
        }
    }
    for (dir, err) in &registry.errors {
        eprintln!("error: {}: {err}", dir.display());
    }
    Ok(())
}

pub fn doctor() -> Result<()> {
    let ok = |b: bool| if b { "ok  " } else { "FAIL" };
    let deno = std::process::Command::new(deno_binary()).arg("--version").output();
    let deno_version =
        deno.as_ref().ok().and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_owned));
    println!(
        "[{}] Deno runtime: {}",
        ok(deno_version.is_some()),
        deno_version.as_deref().unwrap_or("not found, install it from https://deno.com or your package manager")
    );
    let sdk = paths::sdk_dir();
    println!("[{}] SDK: {}", ok(sdk.join("mod.ts").exists()), sdk.display());

    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());
    println!("[info] Session: {session}, desktop: {desktop}");
    println!("[info] Bind a key to `backslash toggle` in your desktop or compositor settings (see README).");

    let registry = registry();
    println!(
        "[{}] Extensions: {} loaded, {} broken",
        ok(registry.errors.is_empty()),
        registry.extensions.len(),
        registry.errors.len()
    );
    for dir in paths::extension_dirs() {
        println!("       searched {}", dir.display());
    }
    for (dir, err) in &registry.errors {
        println!("       {}: {err}", dir.display());
    }
    Ok(())
}
