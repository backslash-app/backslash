//! Updating Backslash itself from its GitHub releases.
//!
//! The release workflow publishes a `.deb`, an `.rpm` and a tarball per
//! architecture, plus `SHA256SUMS`. An update downloads the file matching how
//! this copy was installed, checks it against the checksums, then hands it to
//! the package manager (through `pkexec`) or unpacks it over the tarball
//! install. Copies managed elsewhere (the AUR, a source build) only get told
//! that a new version exists.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// `BACKSLASH_RELEASES_URL` replaces it, for forks and mirrors.
const LATEST_URL: &str = "https://api.github.com/repos/backslash-app/backslash/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/backslash-app/backslash/releases/latest";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Without the leading `v`.
    pub version: String,
    /// The release notes, in markdown.
    pub notes: String,
    pub url: String,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// How this copy of Backslash was installed, which decides how it updates.
#[derive(Clone, Debug, PartialEq)]
pub enum Install {
    Deb,
    Rpm,
    /// Unpacked from the release tarball into a prefix the user can write.
    Tarball {
        prefix: PathBuf,
    },
    /// Updated by something else: `how` says what to run.
    Managed {
        how: String,
    },
    /// A build from a source checkout.
    Source,
}

impl Install {
    /// Whether Backslash can install the update itself.
    pub fn can_update(&self) -> bool {
        matches!(self, Self::Deb | Self::Rpm | Self::Tarball { .. })
    }

    /// One sentence on what installing does, for the update screen.
    pub fn describe(&self) -> String {
        match self {
            Self::Deb => "Backslash downloads the `.deb` from GitHub, checks it against the release checksums, \
                          and installs it with apt. Your system asks for your password."
                .into(),
            Self::Rpm => "Backslash downloads the `.rpm` from GitHub, checks it against the release checksums, \
                          and installs it with your package manager. Your system asks for your password."
                .into(),
            Self::Tarball { prefix } => format!(
                "Backslash downloads the release from GitHub, checks it against the release checksums, \
                 and unpacks it into `{}`.",
                prefix.display()
            ),
            Self::Managed { how } => how.clone(),
            Self::Source => "This is a build from source: pull and run `make install` again.".into(),
        }
    }
}

/// True when `latest` is a newer version than `current` (`1.2.3`, with an
/// optional `-pre` suffix that sorts before the plain version).
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<(Vec<u64>, Option<&str>)> {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let nums = core.split('.').map(|n| n.parse().ok()).collect::<Option<Vec<u64>>>()?;
        Some((nums, pre))
    }
    let (Some((a, a_pre)), Some((b, b_pre))) = (parse(latest), parse(current)) else { return false };
    let len = a.len().max(b.len());
    let pad = |v: &[u64]| (0..len).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
    match pad(&a).cmp(&pad(&b)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

/// Reads GitHub's `releases/latest` answer.
pub fn parse_release(json: &serde_json::Value) -> Result<Release> {
    let tag = json["tag_name"].as_str().ok_or_else(|| anyhow!("the release has no tag"))?;
    let assets = json["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(Asset {
                        name: x["name"].as_str()?.to_string(),
                        url: x["browser_download_url"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        notes: json["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
        url: json["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        assets,
    })
}

/// The latest published release.
pub fn latest() -> Result<Release> {
    let agent = crate::http::agent(Duration::from_secs(20));
    let url = std::env::var("BACKSLASH_RELEASES_URL").unwrap_or_else(|_| LATEST_URL.into());
    let mut response = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", &format!("backslash/{CURRENT}"))
        .call()
        .context("cannot reach GitHub")?;
    let json: serde_json::Value = serde_json::from_reader(response.body_mut().as_reader())?;
    parse_release(&json)
}

/// The latest release when it is newer than this copy.
pub fn check() -> Result<Option<Release>> {
    let release = latest()?;
    Ok(is_newer(&release.version, CURRENT).then_some(release))
}

/// Works out how the running binary was installed.
pub fn detect() -> Install {
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return Install::Managed { how: "Download the new version from the release page.".into() };
    };
    detect_for(&exe, &|program, args| {
        Command::new(program).args(args).output().map(|o| o.status.success()).unwrap_or(false)
    })
}

/// [`detect`] with the package manager queries injected, for tests.
pub fn detect_for(exe: &Path, owns: &dyn Fn(&str, &[&str]) -> bool) -> Install {
    let path = exe.to_string_lossy();
    if path.contains("/target/release/") || path.contains("/target/debug/") {
        return Install::Source;
    }
    if owns("dpkg", &["-S", &path]) {
        return Install::Deb;
    }
    if owns("pacman", &["-Qo", &path]) {
        return Install::Managed {
            how: "Backslash comes from the AUR here: update it with your AUR helper, e.g. `yay -Syu backslash`.".into(),
        };
    }
    if owns("rpm", &["-qf", &path]) {
        return Install::Rpm;
    }
    if let Some(prefix) = exe.parent().filter(|b| b.ends_with("bin")).and_then(Path::parent) {
        if writable(&prefix.join("bin")) {
            return Install::Tarball { prefix: prefix.to_path_buf() };
        }
    }
    Install::Managed { how: "Download the new version from the release page and install it the same way.".into() }
}

fn writable(dir: &Path) -> bool {
    tempfile::NamedTempFile::new_in(dir).is_ok()
}

/// The release file for this kind of install and this machine.
pub fn asset_for<'a>(release: &'a Release, install: &Install, arch: &str) -> Option<&'a Asset> {
    let deb_arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    let suffix = match install {
        Install::Deb => format!("_{deb_arch}.deb"),
        Install::Rpm => format!(".{arch}.rpm"),
        Install::Tarball { .. } => format!("-{arch}.tar.gz"),
        Install::Managed { .. } | Install::Source => return None,
    };
    release.assets.iter().find(|a| a.name.ends_with(&suffix))
}

/// Finds the checksum of `name` in a `sha256sum` listing.
pub fn checksum_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

fn download(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut response = agent
        .get(url)
        .header("User-Agent", &format!("backslash/{CURRENT}"))
        .call()
        .with_context(|| format!("cannot download {url}"))?;
    let mut body = Vec::new();
    response.body_mut().with_config().limit(200 * 1024 * 1024).reader().read_to_end(&mut body)?;
    Ok(body)
}

/// Downloads the release file for `install` and checks it against
/// `SHA256SUMS`. Returns where it was saved.
pub fn download_verified(release: &Release, install: &Install) -> Result<PathBuf> {
    let asset = asset_for(release, install, std::env::consts::ARCH)
        .ok_or_else(|| anyhow!("this release has no file for {} on {}", kind(install), std::env::consts::ARCH))?;
    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .ok_or_else(|| anyhow!("this release has no SHA256SUMS"))?;
    let agent = crate::http::agent(Duration::from_secs(600));
    let sums = String::from_utf8(download(&agent, &sums_asset.url)?)?;
    let expected = checksum_of(&sums, &asset.name).ok_or_else(|| anyhow!("{} is not in SHA256SUMS", asset.name))?;
    let data = download(&agent, &asset.url)?;
    let actual: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        bail!("the download of {} does not match the release checksum", asset.name);
    }
    let dir = crate::paths::cache_dir().join("updates");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&asset.name);
    std::fs::write(&path, data)?;
    Ok(path)
}

fn kind(install: &Install) -> &'static str {
    match install {
        Install::Deb => "Debian packages",
        Install::Rpm => "RPM packages",
        Install::Tarball { .. } => "tarballs",
        Install::Managed { .. } | Install::Source => "this install",
    }
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file())).unwrap_or(false)
}

/// The commands that install a downloaded package, first available wins.
pub fn install_command(install: &Install, file: &Path, has: &dyn Fn(&str) -> bool) -> Option<Vec<String>> {
    let file = file.display().to_string();
    let cmd = |parts: &[&str]| Some(parts.iter().map(|s| s.to_string()).collect());
    match install {
        Install::Deb if has("apt-get") => cmd(&["pkexec", "apt-get", "install", "-y", &file]),
        Install::Deb => cmd(&["pkexec", "dpkg", "-i", &file]),
        Install::Rpm if has("dnf") => cmd(&["pkexec", "dnf", "install", "-y", &file]),
        Install::Rpm if has("zypper") => {
            cmd(&["pkexec", "zypper", "--non-interactive", "install", "--allow-unsigned-rpm", &file])
        }
        Install::Rpm => cmd(&["pkexec", "rpm", "-U", &file]),
        _ => None,
    }
}

/// Installs a file from [`download_verified`].
pub fn apply(install: &Install, file: &Path) -> Result<()> {
    if let Install::Tarball { prefix } = install {
        return unpack(file, prefix);
    }
    let command = install_command(install, file, &has).ok_or_else(|| anyhow!("Backslash can't update this install"))?;
    let out =
        Command::new(&command[0]).args(&command[1..]).output().with_context(|| format!("cannot run {}", command[0]))?;
    if !out.status.success() {
        // pkexec exits with 126 when the user dismisses the password prompt.
        if out.status.code() == Some(126) {
            bail!("the update was cancelled");
        }
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{} failed: {}", command[1], err.lines().last().unwrap_or("unknown error"));
    }
    Ok(())
}

/// Unpacks a release tarball over `prefix`. Files are removed before being
/// written again, because the running binary can't be overwritten in place.
pub fn unpack(tarball: &Path, prefix: &Path) -> Result<()> {
    let data = std::fs::read(tarball)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(&data[..]));
    // Bundled extensions and the SDK are replaced as a whole, so ones the new
    // version dropped don't linger.
    let shared = prefix.join("share/backslash");
    if shared.is_dir() {
        std::fs::remove_dir_all(&shared)?;
    }
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if entry.header().entry_type().is_file() {
            let dest = prefix.join(&path);
            match std::fs::remove_file(&dest) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("cannot replace {}", dest.display())),
            }
        }
        // `unpack_in` refuses paths that leave the prefix.
        entry.unpack_in(prefix).with_context(|| format!("cannot unpack {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("0.5.0", "0.4.0"));
        assert!(is_newer("v0.4.1", "0.4.0"));
        assert!(is_newer("1.0.0", "0.99.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
        assert!(is_newer("0.5.0", "0.5.0-beta.1"));
        assert!(!is_newer("0.5.0-beta.1", "0.5.0"));
        assert!(is_newer("0.5.0-beta.2", "0.5.0-beta.1"));
        assert!(is_newer("0.5", "0.4.9"));
        assert!(!is_newer("garbage", "0.4.0"));
    }

    fn release() -> Release {
        parse_release(&serde_json::json!({
            "tag_name": "v0.5.0",
            "html_url": "https://github.com/backslash-app/backslash/releases/tag/v0.5.0",
            "body": "## What's Changed\r\n* Things",
            "assets": [
                { "name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS" },
                { "name": "backslash-0.5.0-1.aarch64.rpm", "browser_download_url": "https://x/a.rpm" },
                { "name": "backslash-0.5.0-1.x86_64.rpm", "browser_download_url": "https://x/x.rpm" },
                { "name": "backslash-0.5.0-aarch64.tar.gz", "browser_download_url": "https://x/a.tgz" },
                { "name": "backslash-0.5.0-x86_64.tar.gz", "browser_download_url": "https://x/x.tgz" },
                { "name": "backslash_0.5.0_amd64.deb", "browser_download_url": "https://x/x.deb" },
                { "name": "backslash_0.5.0_arm64.deb", "browser_download_url": "https://x/a.deb" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn reads_a_release() {
        let r = release();
        assert_eq!(r.version, "0.5.0");
        assert_eq!(r.notes, "## What's Changed\n* Things");
        assert_eq!(r.assets.len(), 7);
    }

    #[test]
    fn picks_the_right_file() {
        let r = release();
        let name = |i: &Install, arch| asset_for(&r, i, arch).map(|a| a.name.as_str());
        assert_eq!(name(&Install::Deb, "x86_64"), Some("backslash_0.5.0_amd64.deb"));
        assert_eq!(name(&Install::Deb, "aarch64"), Some("backslash_0.5.0_arm64.deb"));
        assert_eq!(name(&Install::Rpm, "x86_64"), Some("backslash-0.5.0-1.x86_64.rpm"));
        let tarball = Install::Tarball { prefix: "/home/me/.local".into() };
        assert_eq!(name(&tarball, "aarch64"), Some("backslash-0.5.0-aarch64.tar.gz"));
        assert_eq!(name(&Install::Source, "x86_64"), None);
        assert_eq!(name(&Install::Deb, "riscv64"), None);
    }

    #[test]
    fn checksums() {
        let sums = "abc123  backslash_0.5.0_amd64.deb\nDEF456 *backslash-0.5.0-x86_64.tar.gz\n";
        assert_eq!(checksum_of(sums, "backslash_0.5.0_amd64.deb").as_deref(), Some("abc123"));
        assert_eq!(checksum_of(sums, "backslash-0.5.0-x86_64.tar.gz").as_deref(), Some("def456"));
        assert_eq!(checksum_of(sums, "backslash_0.5.0_arm64.deb"), None);
    }

    #[test]
    fn detects_the_install() {
        let none = |_: &str, _: &[&str]| false;
        assert_eq!(detect_for(Path::new("/src/backslash/target/release/backslash"), &none), Install::Source);
        let dpkg = |p: &str, _: &[&str]| p == "dpkg";
        assert_eq!(detect_for(Path::new("/usr/bin/backslash"), &dpkg), Install::Deb);
        let rpm = |p: &str, _: &[&str]| p == "rpm";
        assert_eq!(detect_for(Path::new("/usr/bin/backslash"), &rpm), Install::Rpm);
        let pacman = |p: &str, _: &[&str]| p == "pacman";
        assert!(matches!(detect_for(Path::new("/usr/bin/backslash"), &pacman), Install::Managed { .. }));

        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("bin")).unwrap();
        let exe = home.path().join("bin/backslash");
        assert_eq!(detect_for(&exe, &none), Install::Tarball { prefix: home.path().to_path_buf() });
    }

    #[test]
    fn install_commands() {
        let file = Path::new("/tmp/b.deb");
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(install_command(&Install::Deb, file, &all).unwrap()[..3], ["pkexec", "apt-get", "install"]);
        assert_eq!(install_command(&Install::Deb, file, &none).unwrap()[1], "dpkg");
        assert_eq!(install_command(&Install::Rpm, file, &all).unwrap()[1], "dnf");
        let zypper = |p: &str| p == "zypper";
        assert_eq!(install_command(&Install::Rpm, file, &zypper).unwrap()[1], "zypper");
        assert_eq!(install_command(&Install::Rpm, file, &none).unwrap()[1..3], ["rpm", "-U"]);
        assert!(install_command(&Install::Source, file, &all).is_none());
    }

    #[test]
    fn unpacks_over_a_prefix() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("bin")).unwrap();
        std::fs::create_dir_all(src.path().join("share/backslash/extensions/npm")).unwrap();
        std::fs::write(src.path().join("bin/backslash"), "new").unwrap();
        std::fs::write(src.path().join("share/backslash/extensions/npm/backslash.toml"), "new").unwrap();
        let tgz = src.path().join("release.tar.gz");
        let file = std::fs::File::create(&tgz).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(file, flate2::Compression::fast()));
        builder.append_dir_all("./bin", src.path().join("bin")).unwrap();
        builder.append_dir_all("./share", src.path().join("share")).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let prefix = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(prefix.path().join("bin")).unwrap();
        std::fs::create_dir_all(prefix.path().join("share/backslash/extensions/dropped")).unwrap();
        std::fs::write(prefix.path().join("bin/backslash"), "old").unwrap();
        std::fs::write(prefix.path().join("bin/other-tool"), "keep").unwrap();
        unpack(&tgz, prefix.path()).unwrap();

        let read = |p: &str| std::fs::read_to_string(prefix.path().join(p)).unwrap();
        assert_eq!(read("bin/backslash"), "new");
        assert_eq!(read("bin/other-tool"), "keep");
        assert_eq!(read("share/backslash/extensions/npm/backslash.toml"), "new");
        assert!(!prefix.path().join("share/backslash/extensions/dropped").exists());
    }
}
