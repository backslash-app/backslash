//! The Deno runtime that runs TypeScript extensions.
//!
//! Most distributions don't package Deno, so Backslash can install a pinned
//! version for the user in `~/.local/share/backslash/runtime/deno`. It comes
//! from Deno's official npm packages, checked against the registry's SHA-512
//! integrity before use. Otherwise the `deno` on `$PATH` is used.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use sha2::{Digest, Sha512};

use crate::paths;

/// The Deno version installed by Backslash (the one its tests run with).
pub const DENO_VERSION: &str = "2.9.6";
const NPM_REGISTRY: &str = "https://registry.npmjs.org";

pub fn managed_path() -> PathBuf {
    paths::data_dir().join("runtime/deno")
}

/// The Deno binary to use: `$BACKSLASH_DENO`, then the copy installed by
/// Backslash, then the one on `$PATH`.
pub fn binary() -> PathBuf {
    if let Some(p) = std::env::var_os("BACKSLASH_DENO") {
        return p.into();
    }
    let managed = managed_path();
    if managed.is_file() {
        return managed;
    }
    PathBuf::from("deno")
}

/// `deno --version`'s first line, or `None` when Deno can't run.
pub fn version() -> Option<String> {
    let out = Command::new(binary()).arg("--version").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("deno").to_string())
}

/// Whether a Deno binary is there, without running it.
pub fn is_installed() -> bool {
    std::env::var_os("BACKSLASH_DENO").is_some()
        || managed_path().is_file()
        || crate::system::which(&["deno"]).is_some()
}

fn npm_package() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("@deno/linux-x64-glibc"),
        "aarch64" => Ok("@deno/linux-arm64-glibc"),
        arch => bail!("no Deno build for {arch}; install Deno from https://deno.com"),
    }
}

/// Checks `data` against an npm `sha512-<base64>` integrity string.
pub fn verify_integrity(data: &[u8], integrity: &str) -> Result<()> {
    let expected = integrity.strip_prefix("sha512-").ok_or_else(|| anyhow!("unsupported integrity `{integrity}`"))?;
    let actual = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(data));
    if actual != expected {
        bail!("integrity check failed: the download is not the published Deno");
    }
    Ok(())
}

/// Extracts `package/deno` from an npm tarball.
fn extract_binary(tgz: &[u8]) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new("package/deno") {
            let mut bin = Vec::new();
            entry.read_to_end(&mut bin)?;
            return Ok(bin);
        }
    }
    bail!("no deno binary in the package")
}

fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut response = agent.get(url).call().with_context(|| format!("cannot download {url}"))?;
    let mut body = Vec::new();
    response.body_mut().with_config().limit(300 * 1024 * 1024).reader().read_to_end(&mut body)?;
    Ok(body)
}

/// Downloads, verifies and installs Deno; returns its path.
pub fn install() -> Result<PathBuf> {
    let package = npm_package()?;
    let agent = crate::http::agent(std::time::Duration::from_secs(600));
    let meta: serde_json::Value =
        serde_json::from_slice(&get(&agent, &format!("{NPM_REGISTRY}/{package}/{DENO_VERSION}"))?)?;
    let tarball = meta["dist"]["tarball"].as_str().ok_or_else(|| anyhow!("no tarball in the package metadata"))?;
    let integrity = meta["dist"]["integrity"].as_str().ok_or_else(|| anyhow!("no integrity in the metadata"))?;
    if !tarball.starts_with(NPM_REGISTRY) {
        bail!("unexpected download location {tarball}");
    }
    let tgz = get(&agent, tarball)?;
    verify_integrity(&tgz, integrity)?;
    let binary = extract_binary(&tgz)?;

    let target = managed_path();
    std::fs::create_dir_all(target.parent().expect("runtime dir"))?;
    let tmp = target.with_extension("download");
    std::fs::write(&tmp, binary)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&tmp, &target)?;
    let out = Command::new(&target).arg("--version").output().context("the downloaded Deno does not run")?;
    if !out.status.success() {
        bail!("the downloaded Deno does not run on this system");
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity() {
        let data = b"deno";
        let good = format!("sha512-{}", base64::engine::general_purpose::STANDARD.encode(Sha512::digest(data)));
        assert!(verify_integrity(data, &good).is_ok());
        assert!(verify_integrity(b"evil", &good).is_err());
        assert!(verify_integrity(data, "sha1-abc").is_err());
    }

    #[test]
    fn finds_the_binary_in_a_package() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, "package/deno", &b"ELF!"[..]).unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        assert_eq!(extract_binary(&gz.finish().unwrap()).unwrap(), b"ELF!");
    }
}
