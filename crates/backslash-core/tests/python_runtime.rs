//! End-to-end: the Rust host driving the Python runtime inside bubblewrap.
//! Skipped when Python or a working bubblewrap is missing.

use std::path::Path;
use std::time::{Duration, Instant};

use backslash_core::host::{launch_for, Incoming, RpcError, Session};
use backslash_core::protocol::{ActionKind, Effect, View};
use backslash_core::registry::Registry;
use serde_json::json;

const T: Duration = Duration::from_secs(30);

fn available() -> bool {
    let python = std::process::Command::new("python3").arg("--version").output().is_ok_and(|o| o.status.success());
    let bwrap = backslash_core::sandbox::bwrap().is_some_and(|b| {
        std::process::Command::new(b)
            .args(["--ro-bind", "/", "/", "--unshare-all", "true"])
            .status()
            .is_ok_and(|s| s.success())
    });
    if !(python && bwrap) {
        eprintln!("python3 or bubblewrap not usable here: skipped");
    }
    python && bwrap
}

fn start(command: &str, args: serde_json::Value) -> (Session, Result<Effect, RpcError>, tempfile::TempDir) {
    start_as(command, args, "userInitiated")
}

fn start_as(
    command: &str,
    args: serde_json::Value,
    launch_type: &str,
) -> (Session, Result<Effect, RpcError>, tempfile::TempDir) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let registry = Registry::scan(&[fixtures]);
    let (ext, cmd) = registry.resolve(&format!("pydemo/{command}")).expect("fixture command");
    let state = tempfile::tempdir().unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk").canonicalize().unwrap();
    let launch = launch_for(ext, cmd, &sdk).unwrap();
    let session = Session::spawn("pydemo", &launch, state.path()).unwrap();
    let reply = session
        .request(
            "start",
            json!({ "command": command, "arguments": args, "preferences": {}, "query": "", "launchType": launch_type }),
        )
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
    if !available() {
        return;
    }
    let (session, reply, _state) = start("fruits", json!({}));
    let list = pushed_list(reply.unwrap());
    assert_eq!(list.items.len(), 2);
    assert_eq!(list.items[0].accessories[0].tag.as_deref(), Some("clicked 0"));

    let count = &list.items[0].actions[0];
    assert_eq!(count.kind, ActionKind::Callback);
    let effect = session.request("action", json!({ "action": count.id })).unwrap().wait(T).unwrap();
    let list = pushed_list(serde_json::from_value(effect).unwrap());
    assert_eq!(list.items[0].accessories[0].tag.as_deref(), Some("clicked 1"));

    let details = &list.items[1].actions[0];
    let effect: Effect =
        serde_json::from_value(session.request("action", json!({ "action": details.id })).unwrap().wait(T).unwrap())
            .unwrap();
    assert!(matches!(effect, Effect::Push { view: View::Detail(ref d) } if d.markdown == "# Banana"));
}

#[test]
fn search_is_cancellable_and_prints_go_to_the_log() {
    if !available() {
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

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut logged = false;
    while !logged && Instant::now() < deadline {
        match session.incoming.try_recv() {
            Ok(Incoming::Log(line)) => logged = line.contains("searching hey"),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    assert!(logged, "print() must reach the host log, not corrupt the protocol");
}

#[test]
fn no_view_commands_close_with_their_message() {
    if !available() {
        return;
    }
    let (_session, reply, _state) = start("hello", json!({ "name": "Ada" }));
    assert!(matches!(reply.unwrap(), Effect::Close { message: Some(m) } if m == "Hello Ada"));
}

#[test]
fn the_sandbox_holds() {
    if !available() {
        return;
    }
    let (_session, reply, _state) = start("sneaky", json!({}));
    match reply.unwrap() {
        Effect::Close { message: Some(m) } => {
            assert_eq!(m, "0 files in home, no network, own folder read-only")
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn detail_streams_as_it_grows() {
    if !available() {
        return;
    }
    let (session, reply, _state) = start("story", json!({}));
    assert!(matches!(reply.unwrap(), Effect::Push { view: View::Detail(ref d) } if d.loading));
    let deadline = Instant::now() + T;
    loop {
        assert!(Instant::now() < deadline, "the story never finished");
        match session.incoming.try_recv() {
            Ok(Incoming::Notification { method, params }) if method == "view/update" => {
                if params["view"]["loading"] == false {
                    assert_eq!(params["view"]["markdown"], "Once upon a time");
                    break;
                }
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[test]
fn tray_menus() {
    if !available() {
        return;
    }
    let (session, reply, _state) = start_as("tray", json!({}), "background");
    let Effect::Push { view: View::Menu(menu) } = reply.unwrap() else { panic!("expected a menu") };
    assert_eq!((menu.title.as_deref(), menu.icon.as_deref()), (Some("2"), Some("bs-bell")));
    assert_eq!(menu.items[0].title, "background");
    assert!(!menu.items[0].enabled);
    assert_eq!(menu.items[1].section.as_deref(), Some("Mail"));
    let archive = menu.find(&["Inbox".into(), "Archive".into()]).unwrap();
    let action = archive.action.as_ref().unwrap();
    let effect = session.request("action", json!({ "action": action.id })).unwrap().wait(T).unwrap();
    assert_eq!(effect, json!({ "type": "close", "message": "Archived" }));
}
