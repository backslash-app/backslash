//! Runs one extension command in its own process and talks JSON-RPC 2.0 with it,
//! one JSON message per line on stdin/stdout. Stderr is the extension's log.
//!
//! A crash, an infinite loop or a slow network call in an extension can never
//! freeze the launcher: the worst case is a killed child process.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

use crate::manifest::{CommandDef, Runtime};
use crate::paths;
use crate::registry::Extension;
use crate::store::{self, ExtensionStorage};

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RpcError {}

impl RpcError {
    pub const CANCELLED: i64 = -32800;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const EXITED: i64 = -32099;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

/// Messages initiated by the extension.
#[derive(Debug, Clone)]
pub enum Incoming {
    /// Needs [`Session::respond`]: clipboard, open, toast, close…
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Fire and forget, e.g. `view/update`.
    Notification {
        method: String,
        params: Value,
    },
    /// A line written to stderr (including `console.log` in Deno extensions).
    Log(String),
    Exited(Option<i32>),
}

type Reply = Result<Value, RpcError>;

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    pending: Mutex<HashMap<u64, async_channel::Sender<Reply>>>,
}

impl Shared {
    fn send(&self, message: &Value) -> Result<()> {
        let mut guard = self.stdin.lock().unwrap();
        let stdin = guard.as_mut().ok_or_else(|| anyhow!("extension is not running"))?;
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        stdin.write_all(&line)?;
        stdin.flush()?;
        Ok(())
    }

    fn fail_all(&self, error: RpcError) {
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.try_send(Err(error.clone()));
        }
    }
}

pub struct Session {
    pub extension_id: String,
    child: Mutex<Child>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    pub incoming: async_channel::Receiver<Incoming>,
}

/// A response that has not arrived yet.
pub struct Pending {
    pub id: u64,
    rx: async_channel::Receiver<Reply>,
}

impl Pending {
    pub async fn get(self) -> Reply {
        self.rx.recv().await.unwrap_or_else(|_| Err(RpcError::new(RpcError::EXITED, "extension exited")))
    }

    /// Non-blocking poll; `None` while the reply has not arrived.
    pub fn try_get(&self) -> Option<Reply> {
        match self.rx.try_recv() {
            Ok(reply) => Some(reply),
            Err(async_channel::TryRecvError::Closed) => Some(Err(RpcError::new(RpcError::EXITED, "extension exited"))),
            Err(async_channel::TryRecvError::Empty) => None,
        }
    }

    /// Blocking variant for the CLI.
    pub fn wait(self, timeout: Duration) -> Reply {
        let deadline = Instant::now() + timeout;
        loop {
            match self.rx.try_recv() {
                Ok(reply) => return reply,
                Err(async_channel::TryRecvError::Closed) => {
                    return Err(RpcError::new(RpcError::EXITED, "extension exited"))
                }
                Err(async_channel::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err(RpcError::new(RpcError::CANCELLED, "extension timed out"))
                }
                Err(async_channel::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

/// How to start the extension process.
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Deno binary: `$BACKSLASH_DENO`, then the copy managed by Backslash, then `$PATH`.
pub fn deno_binary() -> PathBuf {
    crate::runtime::binary()
}

/// Python for the Python runtime: `$BACKSLASH_PYTHON`, else `python3`.
pub fn python_binary() -> PathBuf {
    std::env::var_os("BACKSLASH_PYTHON").map(PathBuf::from).unwrap_or_else(|| "python3".into())
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(",")
}

/// Builds the command line. For Deno, the manifest's permissions become Deno
/// flags, so the sandbox is enforced by the runtime, not by good manners.
pub fn launch_for(ext: &Extension, command: &CommandDef, sdk: &Path) -> Result<Launch> {
    let dir = ext.dir.canonicalize().with_context(|| format!("{} is missing", ext.dir.display()))?;
    let entry = dir.join(command.entry_path(&ext.manifest.runtime));
    if !entry.is_file() {
        anyhow::bail!("{}: entry {} not found", ext.manifest.extension.id, entry.display());
    }
    match &ext.manifest.runtime {
        Runtime::Exec { command: argv } => {
            let (program, rest) = argv.split_first().context("runtime.command is empty")?;
            let program = if program.contains('/') { dir.join(program) } else { PathBuf::from(program) };
            let mut args = rest.to_vec();
            args.push(entry.to_string_lossy().into_owned());
            crate::sandbox::wrap(Launch { program, args, cwd: dir }, &ext.manifest.permissions, &[])
        }
        Runtime::Python => {
            // -I: no user site-packages or PYTHON* variables; the runtime puts
            // the SDK and the extension's folder on the module path itself.
            let sdk = sdk.join("python");
            let args = vec![
                "-I".into(),
                "-u".into(),
                sdk.join("runtime.py").to_string_lossy().into_owned(),
                entry.to_string_lossy().into_owned(),
            ];
            let launch = Launch { program: python_binary(), args, cwd: dir };
            crate::sandbox::wrap(launch, &ext.manifest.permissions, &[sdk])
        }
        Runtime::Deno => {
            let perms = &ext.manifest.permissions;
            let import_map = write_import_map(&ext.manifest.extension.id, &dir, sdk)?;
            let mut read = vec![dir.clone(), sdk.to_path_buf()];
            read.extend(perms.read.iter().map(|p| paths::expand(p)));
            let mut args = vec![
                "run".to_string(),
                "--quiet".into(),
                "--no-prompt".into(),
                "--no-config".into(),
                "--no-lock".into(),
                format!("--import-map={}", import_map.display()),
                format!("--allow-read={}", join_paths(&read)),
            ];
            if perms.net.iter().any(|h| h == "*") {
                args.push("--allow-net".into());
            } else if !perms.net.is_empty() {
                args.push(format!("--allow-net={}", perms.net.join(",")));
            }
            if !perms.run.is_empty() {
                args.push(format!("--allow-run={}", perms.run.join(",")));
            }
            if !perms.write.is_empty() {
                let write: Vec<PathBuf> = perms.write.iter().map(|p| paths::expand(p)).collect();
                args.push(format!("--allow-write={}", join_paths(&write)));
            }
            if !perms.env.is_empty() {
                args.push(format!("--allow-env={}", perms.env.join(",")));
            }
            args.push(sdk.join("runtime.ts").to_string_lossy().into_owned());
            args.push(entry.to_string_lossy().into_owned());
            Ok(Launch { program: deno_binary(), args, cwd: dir })
        }
    }
}

/// Merges the extension's own `deno.json` imports with `@backslash/api`
/// pointing at the SDK bundled with this launcher.
fn write_import_map(id: &str, dir: &Path, sdk: &Path) -> Result<PathBuf> {
    let mut imports = Map::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("deno.json")) {
        if let Ok(Value::Object(mut cfg)) = serde_json::from_str::<Value>(&text) {
            if let Some(Value::Object(own)) = cfg.remove("imports") {
                imports.extend(own);
            }
        }
    }
    let sdk_url = format!("file://{}/", sdk.display());
    imports.insert("@backslash/api".into(), format!("{sdk_url}mod.ts").into());
    imports.insert("@backslash/api/".into(), sdk_url.into());
    let path = paths::state_dir().join("import-maps").join(format!("{id}.json"));
    store::write_json(&path, &json!({ "imports": imports }))?;
    Ok(path)
}

impl Session {
    pub fn spawn(extension_id: &str, launch: &Launch, storage_root: &Path) -> Result<Self> {
        let mut child = Command::new(&launch.program)
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .env("NO_COLOR", "1")
            .env("DENO_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot start {}", launch.program.display()))?;

        let shared = Arc::new(Shared { stdin: Mutex::new(child.stdin.take()), pending: Mutex::new(HashMap::new()) });
        let (tx, incoming) = async_channel::unbounded();

        let stdout = child.stdout.take().expect("piped stdout");
        let reader_shared = shared.clone();
        let reader_tx = tx.clone();
        let storage = ExtensionStorage::new(storage_root, extension_id);
        std::thread::Builder::new().name(format!("ext-{extension_id}-out")).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(msg) => dispatch(msg, &reader_shared, &reader_tx, &storage),
                    Err(_) => {
                        let _ = reader_tx.try_send(Incoming::Log(format!("[stdout] {line}")));
                    }
                }
            }
            reader_shared.fail_all(RpcError::new(RpcError::EXITED, "extension exited"));
        })?;

        let stderr = child.stderr.take().expect("piped stderr");
        let log_tx = tx;
        std::thread::Builder::new().name(format!("ext-{extension_id}-err")).spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = log_tx.try_send(Incoming::Log(line));
            }
        })?;

        Ok(Self {
            extension_id: extension_id.to_string(),
            child: Mutex::new(child),
            shared,
            next_id: AtomicU64::new(1),
            incoming,
        })
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Pending> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = async_channel::bounded(1);
        self.shared.pending.lock().unwrap().insert(id, tx);
        let sent = self.shared.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        if let Err(err) = sent {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(err);
        }
        Ok(Pending { id, rx })
    }

    /// Asks the extension to abort a request (its `AbortSignal` fires). The
    /// pending response resolves with a cancellation error right away.
    pub fn cancel(&self, id: u64) {
        if let Some(tx) = self.shared.pending.lock().unwrap().remove(&id) {
            let _ = tx.try_send(Err(RpcError::new(RpcError::CANCELLED, "cancelled")));
        }
        let _ = self.notify("$/cancel", json!({ "id": id }));
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.shared.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) {
        let msg = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } }),
        };
        let _ = self.shared.send(&msg);
    }

    /// Checks whether the process ended, without blocking.
    pub fn exit_status(&self) -> Option<Option<i32>> {
        self.child.lock().unwrap().try_wait().ok().flatten().map(|s| s.code())
    }

    pub fn kill(&self) {
        self.shared.stdin.lock().unwrap().take();
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

fn dispatch(msg: Value, shared: &Shared, tx: &async_channel::Sender<Incoming>, storage: &ExtensionStorage) {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match (msg.get("id"), method) {
        // Response to one of our requests.
        (Some(id), None) => {
            let Some(id) = id.as_u64() else { return };
            let Some(reply_tx) = shared.pending.lock().unwrap().remove(&id) else { return };
            let reply = match msg.get("error") {
                Some(err) => Err(RpcError::new(
                    err.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                    err.get("message").and_then(Value::as_str).unwrap_or("extension error"),
                )),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = reply_tx.try_send(reply);
        }
        // Storage never needs the UI: answer on this thread.
        (Some(id), Some(method)) if method.starts_with("storage/") => {
            let key = params.get("key").and_then(Value::as_str).unwrap_or_default();
            let result = match method.as_str() {
                "storage/get" => Ok(storage.get(key)),
                "storage/set" => storage
                    .set(key, params.get("value").cloned().unwrap_or(Value::Null))
                    .map(|_| Value::Null)
                    .map_err(|e| RpcError::new(-32000, e.to_string())),
                "storage/keys" => Ok(json!(storage.keys())),
                _ => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, format!("unknown method {method}"))),
            };
            let msg = match result {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": e.code, "message": e.message } }),
            };
            let _ = shared.send(&msg);
        }
        (Some(id), Some(method)) => {
            let _ = tx.try_send(Incoming::Request { id: id.clone(), method, params });
        }
        (None, Some(method)) => {
            let _ = tx.try_send(Incoming::Notification { method, params });
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny protocol peer written in shell + jq-free sed, so the test needs no Deno.
    const ECHO_PEER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"start"'*)
      echo '{"jsonrpc":"2.0","id":7,"method":"storage/set","params":{"key":"k","value":42}}'
      echo "log from extension" >&2
      echo '{"jsonrpc":"2.0","method":"view/update","params":{"n":1}}'
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"type\":\"detail\",\"markdown\":\"hi\"}}" ;;
    *'"method":"crash"'*) exit 3 ;;
  esac
done
"#;

    fn peer() -> (tempfile::TempDir, Launch) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("peer.sh");
        std::fs::write(&script, ECHO_PEER).unwrap();
        let launch =
            Launch { program: "sh".into(), args: vec![script.to_string_lossy().into()], cwd: dir.path().into() };
        (dir, launch)
    }

    #[test]
    fn request_response_storage_and_notifications() {
        let (dir, launch) = peer();
        let session = Session::spawn("echo", &launch, dir.path()).unwrap();
        let reply = session.request("start", json!({})).unwrap().wait(Duration::from_secs(5)).unwrap();
        assert_eq!(reply["markdown"], "hi");

        let mut saw_update = false;
        let mut saw_log = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(saw_update && saw_log) && Instant::now() < deadline {
            match session.incoming.try_recv() {
                Ok(Incoming::Notification { method, .. }) => saw_update |= method == "view/update",
                Ok(Incoming::Log(line)) => saw_log |= line == "log from extension",
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(saw_update && saw_log);
        // The storage request was answered by the host thread itself.
        assert_eq!(ExtensionStorage::new(dir.path(), "echo").get("k"), json!(42));
    }

    #[test]
    fn crash_fails_pending_requests() {
        let (dir, launch) = peer();
        let session = Session::spawn("echo", &launch, dir.path()).unwrap();
        let err = session.request("crash", json!({})).unwrap().wait(Duration::from_secs(5)).unwrap_err();
        assert_eq!(err.code, RpcError::EXITED);
    }
}
