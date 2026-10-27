//! End-to-end: the Rust host driving the real TypeScript runtime.
//! Skipped when Deno is not installed.

use std::path::Path;
use std::time::Duration;

use backslash_core::host::{launch_for, Incoming, RpcError, Session};
use backslash_core::protocol::{ActionKind, Effect, View};
use backslash_core::registry::Registry;
use serde_json::json;

const T: Duration = Duration::from_secs(30);

fn deno_available() -> bool {
    std::process::Command::new(backslash_core::host::deno_binary()).arg("--version").output().is_ok()
}

fn start(command: &str, args: serde_json::Value) -> (Session, Result<Effect, RpcError>, tempfile::TempDir) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let registry = Registry::scan(&[fixtures]);
    let (ext, cmd) = registry.resolve(&format!("demo/{command}")).expect("fixture command");
    // Import maps go to $XDG_STATE_HOME. Tests run in parallel and the
    // environment is process-wide, so set it once for all of them.
    static STATE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let shared = STATE.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("XDG_STATE_HOME", shared.path());
    let state = tempfile::tempdir().unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk").canonicalize().unwrap();
    let launch = launch_for(ext, cmd, &sdk).unwrap();
    let session = Session::spawn("demo", &launch, state.path()).unwrap();
    let reply = session
        .request("start", json!({ "command": command, "arguments": args, "preferences": {}, "query": "" }))
        .unwrap()
        .wait(T)
        .map(|v| serde_json::from_value(v).unwrap());
    (session, reply, state)
}

fn pushed_list(effect: Effect) -> backslash_core::protocol::ListView {
    match effect {
        Effect::Push { view: View::List(list) } | Effect::Replace { view: View::List(list) } => list,
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn list_callbacks_storage_and_navigation() {
    if !deno_available() {
        eprintln!("deno not installed, skipping");
        return;
    }
    let (session, reply, _state) = start("fruits", json!({}));
    let list = pushed_list(reply.unwrap());
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.items[0].accessories[0].tag.as_deref(), Some("clicked 0"));

    // Callback → storage → refresh re-renders the same view.
    let count = &list.items[0].actions[0];
    assert_eq!(count.kind, ActionKind::Callback);
    let effect = session.request("action", json!({ "action": count.id })).unwrap().wait(T).unwrap();
    let list = pushed_list(serde_json::from_value(effect).unwrap());
    assert_eq!(list.items[0].accessories[0].tag.as_deref(), Some("clicked 1"));

    // Returning a view pushes it.
    let details = &list.items[1].actions[0];
    let effect: Effect =
        serde_json::from_value(session.request("action", json!({ "action": details.id })).unwrap().wait(T).unwrap())
            .unwrap();
    assert!(matches!(effect, Effect::Push { view: View::Detail(ref d) } if d.markdown == "# Banana"));
}

#[test]
fn search_is_cancellable_and_logs_go_to_stderr() {
    if !deno_available() {
        return;
    }
    let (session, reply, _state) = start("echo", json!({}));
    let list = pushed_list(reply.unwrap());
    let view = list.id.clone().unwrap();

    let slow = session.request("search", json!({ "view": view, "query": "slow" })).unwrap();
    session.cancel(slow.id);
    assert_eq!(slow.wait(T).unwrap_err().code, RpcError::CANCELLED);

    let fast = session.request("search", json!({ "view": view, "query": "hey" })).unwrap().wait(T).unwrap();
    assert_eq!(fast["items"][0]["title"], "You typed hey");

    let mut logged = false;
    for _ in 0..200 {
        match session.incoming.try_recv() {
            Ok(Incoming::Log(line)) if line.contains("searching hey") => {
                logged = true;
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    assert!(logged, "console.log must reach the host log, not corrupt stdout");
}

#[test]
fn no_view_command_and_sandbox() {
    if !deno_available() {
        return;
    }
    let (_session, reply, _state) = start("hello", json!({ "name": "Linux" }));
    assert_eq!(reply.unwrap(), Effect::Close { message: Some("Hello Linux".into()) });

    let (_session, reply, _state) = start("sneaky", json!({}));
    let err = reply.unwrap_err();
    assert!(err.message.contains("NotCapable") || err.message.contains("read access"), "{err}");
}

#[test]
fn detail_streams_its_markdown() {
    if !deno_available() {
        return;
    }
    let (session, reply, _state) = start("story", json!({}));
    let Effect::Push { view: View::Detail(first) } = reply.unwrap() else { panic!("expected a detail") };
    assert!(first.loading);
    let mut updates = Vec::new();
    loop {
        let Ok(Incoming::Notification { method, params }) = session.incoming.recv_blocking() else {
            panic!("session closed before the stream ended")
        };
        assert_eq!(method, "view/update");
        let View::Detail(d) = serde_json::from_value(params["view"].clone()).unwrap() else { panic!("not a detail") };
        assert_eq!(d.id, first.id, "updates target the same view");
        let done = !d.loading;
        updates.push(d.markdown);
        if done {
            break;
        }
    }
    assert!(updates.len() >= 2, "text arrives in several updates: {updates:?}");
    assert_eq!(updates.last().unwrap(), "Once upon a time");
}
