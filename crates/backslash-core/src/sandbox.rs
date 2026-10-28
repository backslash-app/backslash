//! Bubblewrap sandbox for extensions that are not run by Deno (`exec` and
//! `python` runtimes): they see the system read-only, their own folder
//! read-only, an empty home and `/tmp`, and only the files, environment
//! variables and network their manifest declares.
//!
//! Bubblewrap cannot filter by host or by program: an extension declaring
//! any `net` host gets the network, and it may start any installed program.
//! The permission prompt says so (see [`crate::approvals::requested`]).

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::host::Launch;
use crate::manifest::Permissions;
use crate::paths;

/// System folders shared read-only, when they exist. `/etc` holds the
/// dynamic linker cache, certificates, `resolv.conf` and alternatives.
const SYSTEM: &[&str] = &["/usr", "/etc", "/opt", "/nix/store", "/gnu/store", "/run/current-system"];
/// Folders that are often links into `/usr` (merged /usr).
const LINKED: &[&str] = &["/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32"];
/// Environment kept from the launcher, besides what the manifest lists.
const KEPT_ENV: &[&str] = &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "LANGUAGE", "TZ"];

/// The `bwrap` binary: `$BACKSLASH_BWRAP`, then `$PATH`.
pub fn bwrap() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("BACKSLASH_BWRAP") {
        return Some(p.into());
    }
    find_on_path("bwrap")
}

/// The full path of `program` on `$PATH`.
fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| p.is_file())
}

pub fn is_available() -> bool {
    bwrap().is_some()
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|s| s.to_string()));
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Runs `launch` inside bubblewrap with `permissions`. `shared` are extra
/// folders the runtime itself needs, read-only (the SDK).
pub fn wrap(launch: Launch, permissions: &Permissions, shared: &[PathBuf]) -> Result<Launch> {
    let Some(bwrap) = bwrap() else {
        bail!("this extension runs in a sandbox that needs bubblewrap: install the `bubblewrap` package");
    };
    Ok(Launch { program: bwrap, args: arguments(&launch, permissions, shared), cwd: launch.cwd })
}

fn arguments(launch: &Launch, permissions: &Permissions, shared: &[PathBuf]) -> Vec<String> {
    let mut args = Vec::new();
    push(&mut args, &["--die-with-parent", "--new-session", "--unshare-all"]);
    if !permissions.net.is_empty() {
        push(&mut args, &["--share-net"]);
        // resolv.conf often points there (systemd-resolved, NetworkManager).
        for dir in ["/run/systemd/resolve", "/run/NetworkManager"] {
            push(&mut args, &["--ro-bind-try", dir, dir]);
        }
    }
    for dir in SYSTEM {
        push(&mut args, &["--ro-bind-try", dir, dir]);
    }
    for dir in LINKED {
        let path = Path::new(dir);
        match std::fs::read_link(path) {
            Ok(target) => push(&mut args, &["--symlink", &s(&target), dir]),
            Err(_) => push(&mut args, &["--ro-bind-try", dir, dir]),
        }
    }
    push(&mut args, &["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    let home = paths::home();
    push(&mut args, &["--tmpfs", &s(&home)]);

    // A program found on $PATH outside the system folders (~/.local/bin).
    if !launch.program.as_os_str().is_empty() {
        let found = if launch.program.is_absolute() {
            Some(launch.program.clone())
        } else {
            find_on_path(&launch.program.to_string_lossy())
        };
        if let Some(found) = found.and_then(|p| p.canonicalize().ok()) {
            push(&mut args, &["--ro-bind", &s(&found), &s(&found)]);
        }
    }
    for dir in shared {
        push(&mut args, &["--ro-bind", &s(dir), &s(dir)]);
    }
    for p in &permissions.read {
        let p = s(&paths::expand(p));
        push(&mut args, &["--ro-bind-try", &p, &p]);
    }
    for p in &permissions.write {
        let p = s(&paths::expand(p));
        push(&mut args, &["--bind-try", &p, &p]);
    }
    // Its own folder last: never writable, even inside a writable path.
    push(&mut args, &["--ro-bind", &s(&launch.cwd), &s(&launch.cwd), "--chdir", &s(&launch.cwd)]);

    push(&mut args, &["--clearenv", "--setenv", "HOME", &s(&home), "--setenv", "TMPDIR", "/tmp"]);
    push(&mut args, &["--setenv", "NO_COLOR", "1", "--setenv", "PYTHONDONTWRITEBYTECODE", "1"]);
    let names = KEPT_ENV.iter().map(|n| n.to_string()).chain(permissions.env.iter().cloned());
    for name in names {
        if let Ok(value) = std::env::var(&name) {
            push(&mut args, &["--setenv", &name, &value]);
        }
    }
    args.push("--".into());
    args.push(s(&launch.program));
    args.extend(launch.args.iter().cloned());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(dir: &Path) -> Launch {
        Launch { program: "sh".into(), args: vec!["-c".into(), "true".into()], cwd: dir.to_path_buf() }
    }

    #[test]
    fn network_and_files_follow_the_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let none = arguments(&launch(dir.path()), &Permissions::default(), &[]);
        assert!(none.contains(&"--unshare-all".to_string()));
        assert!(!none.contains(&"--share-net".to_string()));
        assert!(!none.iter().any(|a| a.contains("/run/systemd")));
        assert!(none.ends_with(&["--".into(), "sh".into(), "-c".into(), "true".into()]));

        let perms = Permissions {
            net: vec!["api.example.com".into()],
            read: vec!["/srv/data".into()],
            write: vec!["/srv/out".into()],
            env: vec!["BACKSLASH_TEST_TOKEN".into()],
            ..Default::default()
        };
        std::env::set_var("BACKSLASH_TEST_TOKEN", "t0k");
        let args = arguments(&launch(dir.path()), &perms, &[PathBuf::from("/opt/sdk")]).join(" ");
        assert!(args.contains("--share-net"));
        assert!(args.contains("--ro-bind-try /srv/data /srv/data"));
        assert!(args.contains("--bind-try /srv/out /srv/out"));
        assert!(args.contains("--ro-bind /opt/sdk /opt/sdk"));
        assert!(args.contains("--setenv BACKSLASH_TEST_TOKEN t0k"));
        // The extension's folder is mounted after the writable paths: read-only.
        let own = format!("--ro-bind {0} {0}", dir.path().display());
        assert!(args.find(&own).unwrap() > args.find("--bind-try /srv/out").unwrap());
    }

    /// Runs for real where bubblewrap works (CI, desktops); skipped elsewhere.
    #[test]
    fn confines_a_program() {
        if !is_available()
            || !std::process::Command::new(bwrap().unwrap())
                .args(["--ro-bind", "/", "/", "--unshare-all", "true"])
                .status()
                .is_ok_and(|s| s.success())
        {
            eprintln!("bubblewrap is not usable here: skipped");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let script = "echo hi > own.txt 2>/dev/null && echo own-writable; \
                      echo hi > \"$OUT/out.txt\" && echo out-writable; \
                      ls \"$HOME\" | wc -l; echo \"${SECRET_THING:-no-secret}\"";
        let perms = Permissions { write: vec![out.path().to_string_lossy().into()], ..Default::default() };
        let launch = Launch {
            program: "sh".into(),
            args: vec!["-c".into(), script.replace("$OUT", &out.path().to_string_lossy())],
            cwd: dir.path().to_path_buf(),
        };
        std::env::set_var("SECRET_THING", "leaked");
        let wrapped = wrap(launch, &perms, &[]).unwrap();
        let output = std::process::Command::new(&wrapped.program).args(&wrapped.args).output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(text.lines().collect::<Vec<_>>(), ["out-writable", "0", "no-secret"], "{text}");
        assert!(out.path().join("out.txt").is_file());
    }
}
