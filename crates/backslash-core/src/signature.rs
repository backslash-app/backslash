//! SSH signatures on git commits (`git commit -S` with `gpg.format = ssh`),
//! checked without `git`, `ssh-keygen` or any configuration: the registry's
//! trusted keys ship with Backslash.
//!
//! Only Ed25519 keys are accepted, the default of `ssh-keygen` and the only
//! kind the registry's signing key needs to be.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha512};

const KEY_TYPE: &str = "ssh-ed25519";
const MAGIC: &[u8] = b"SSHSIG";
/// The namespace git signs commits and tags in.
const GIT_NAMESPACE: &str = "git";
const BEGIN: &str = "-----BEGIN SSH SIGNATURE-----";
const END: &str = "-----END SSH SIGNATURE-----";

/// A public key as written in `id_ed25519.pub`: `ssh-ed25519 AAAA… comment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    key: [u8; 32],
    pub comment: String,
}

impl PublicKey {
    pub fn parse(line: &str) -> Result<Self> {
        let mut parts = line.split_whitespace();
        let (Some(kind), Some(blob)) = (parts.next(), parts.next()) else {
            bail!("`{line}` is not an SSH public key");
        };
        if kind != KEY_TYPE {
            bail!("only ssh-ed25519 keys are supported, not {kind}");
        }
        let blob = base64::engine::general_purpose::STANDARD.decode(blob).context("invalid key")?;
        let key = Self::from_blob(&blob)?;
        Ok(Self { comment: parts.collect::<Vec<_>>().join(" "), ..key })
    }

    fn from_blob(blob: &[u8]) -> Result<Self> {
        let mut r = Reader(blob);
        if r.string()? != KEY_TYPE.as_bytes() {
            bail!("only ssh-ed25519 keys are supported");
        }
        let key: [u8; 32] = r.string()?.try_into().map_err(|_| anyhow!("invalid ed25519 key"))?;
        r.end()?;
        Ok(Self { key, comment: String::new() })
    }
}

/// Reads SSH wire format: big-endian lengths followed by bytes.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.0.len() < n {
            bail!("truncated signature");
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn string(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn end(&self) -> Result<()> {
        if !self.0.is_empty() {
            bail!("unexpected data after the signature");
        }
        Ok(())
    }
}

fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

/// What the signer actually signed (PROTOCOL.sshsig): the namespace and a
/// hash of the message.
fn signed_data(namespace: &str, hash_algorithm: &str, digest: &[u8]) -> Vec<u8> {
    let mut data = MAGIC.to_vec();
    put_string(&mut data, namespace.as_bytes());
    put_string(&mut data, b"");
    put_string(&mut data, hash_algorithm.as_bytes());
    put_string(&mut data, digest);
    data
}

fn digest(hash_algorithm: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    match hash_algorithm {
        b"sha512" => Ok(Sha512::digest(message).to_vec()),
        b"sha256" => Ok(sha2::Sha256::digest(message).to_vec()),
        other => bail!("unsupported hash {}", String::from_utf8_lossy(other)),
    }
}

/// Checks an armored SSH signature of `message` in `namespace`; returns the
/// trusted key that made it.
pub fn verify<'k>(armored: &str, namespace: &str, message: &[u8], trusted: &'k [PublicKey]) -> Result<&'k PublicKey> {
    let body: String = armored
        .trim()
        .strip_prefix(BEGIN)
        .and_then(|s| s.strip_suffix(END))
        .ok_or_else(|| anyhow!("not an SSH signature"))?
        .split_whitespace()
        .collect();
    let blob = base64::engine::general_purpose::STANDARD.decode(body).context("invalid signature")?;
    let mut r = Reader(&blob);
    if r.take(MAGIC.len())? != MAGIC || r.u32()? != 1 {
        bail!("not an SSH signature");
    }
    let signer = PublicKey::from_blob(r.string()?)?;
    if r.string()? != namespace.as_bytes() {
        bail!("signature made for another purpose");
    }
    let _reserved = r.string()?;
    let hash_algorithm = r.string()?;
    let mut sig = Reader(r.string()?);
    r.end()?;
    if sig.string()? != KEY_TYPE.as_bytes() {
        bail!("only ssh-ed25519 signatures are supported");
    }
    let sig = Signature::from_slice(sig.string()?).map_err(|_| anyhow!("invalid signature"))?;

    let key = trusted.iter().find(|k| k.key == signer.key).ok_or_else(|| anyhow!("signed by an unknown key"))?;
    let verifying = VerifyingKey::from_bytes(&key.key).map_err(|_| anyhow!("invalid trusted key"))?;
    let data = signed_data(namespace, &String::from_utf8_lossy(hash_algorithm), &digest(hash_algorithm, message)?);
    verifying.verify_strict(&data, &sig).map_err(|_| anyhow!("the signature does not match"))?;
    Ok(key)
}

/// A commit whose signature checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCommit {
    /// Committer time, in seconds since the epoch.
    pub time: i64,
    /// Comment of the key that signed it.
    pub signer: String,
}

/// Checks a raw commit object (`git cat-file commit <id>`) against trusted
/// keys. Git signs the object without its `gpgsig` header.
pub fn verify_commit(raw: &str, trusted: &[PublicKey]) -> Result<VerifiedCommit> {
    let (headers, message) = raw.split_once("\n\n").unwrap_or((raw, ""));
    let mut payload = String::with_capacity(raw.len());
    let mut signature: Option<String> = None;
    let mut in_signature = false;
    let mut time = None;
    for line in headers.lines() {
        if let Some(rest) = line.strip_prefix("gpgsig ") {
            if signature.is_some() {
                bail!("the commit has two signatures");
            }
            signature = Some(rest.to_string());
            in_signature = true;
            continue;
        }
        if in_signature {
            if let Some(rest) = line.strip_prefix(' ') {
                let sig = signature.as_mut().expect("in a signature");
                sig.push('\n');
                sig.push_str(rest);
                continue;
            }
            in_signature = false;
        }
        if let Some(committer) = line.strip_prefix("committer ") {
            // "Name <email> 1700000000 +0000"
            time = committer.rsplit(' ').nth(1).and_then(|t| t.parse().ok());
        }
        payload.push_str(line);
        payload.push('\n');
    }
    payload.push('\n');
    payload.push_str(message);
    let signature = signature.ok_or_else(|| anyhow!("the commit is not signed"))?;
    let key = verify(&signature, GIT_NAMESPACE, payload.as_bytes(), trusted)?;
    Ok(VerifiedCommit {
        time: time.ok_or_else(|| anyhow!("the commit has no committer"))?,
        signer: key.comment.clone(),
    })
}

/// Signs `raw_unsigned` (a commit object without `gpgsig`) the way git does,
/// so tests need neither `ssh-keygen` nor a configured git.
#[cfg(test)]
pub(crate) fn sign_commit(raw_unsigned: &str, key: &ed25519_dalek::SigningKey) -> String {
    use ed25519_dalek::Signer;
    let public = key.verifying_key().to_bytes();
    let mut public_blob = Vec::new();
    put_string(&mut public_blob, KEY_TYPE.as_bytes());
    put_string(&mut public_blob, &public);
    let data = signed_data(GIT_NAMESPACE, "sha512", &Sha512::digest(raw_unsigned.as_bytes()));
    let mut sig_blob = Vec::new();
    put_string(&mut sig_blob, KEY_TYPE.as_bytes());
    put_string(&mut sig_blob, &key.sign(&data).to_bytes());
    let mut blob = MAGIC.to_vec();
    blob.extend_from_slice(&1u32.to_be_bytes());
    put_string(&mut blob, &public_blob);
    put_string(&mut blob, GIT_NAMESPACE.as_bytes());
    put_string(&mut blob, b"");
    put_string(&mut blob, b"sha512");
    put_string(&mut blob, &sig_blob);
    let b64 = base64::engine::general_purpose::STANDARD.encode(blob);
    let armored = format!("{BEGIN}\n{b64}\n{END}");
    let (headers, message) = raw_unsigned.split_once("\n\n").expect("commit message");
    format!("{headers}\ngpgsig {}\n\n{message}", armored.replace('\n', "\n "))
}

/// The `ssh-ed25519 …` line of a test key.
#[cfg(test)]
pub(crate) fn public_line(key: &ed25519_dalek::SigningKey, comment: &str) -> String {
    let mut blob = Vec::new();
    put_string(&mut blob, KEY_TYPE.as_bytes());
    put_string(&mut blob, &key.verifying_key().to_bytes());
    format!("{KEY_TYPE} {} {comment}", base64::engine::general_purpose::STANDARD.encode(blob))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed by `ssh-keygen` through `git commit-tree -S` (gpg.format = ssh).
    const COMMIT: &str = "tree 5b510de32b0406d827336f81d0a989d4c9416bfd
author CI <ci@x> 1700000000 +0000
committer CI <ci@x> 1700000000 +0000
gpgsig -----BEGIN SSH SIGNATURE-----
 U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgn5yh+5bg0NPuZMhIodCanu0eRm
 ByjYCvvfz+wfumnwgAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5
 AAAAQIYlPiEOjYwRoxlwDoop0hlDLGU+w75GbPsPl368OM/Br9UgyefvWZgPG0BXMREG/C
 VkHXLJUyo3rMteVrmeRwY=
 -----END SSH SIGNATURE-----

Registry at abc
";
    const SIGNER: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ+cofuW4NDT7mTISKHQmp7tHkZgco2Ar738/sH7pp8I registry-test";
    const OTHER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK7ANh3XEWt43BIooeYAVE6+cNidyLnOXRheqXWYOwal other";

    #[test]
    fn verifies_a_commit_signed_by_git() {
        let trusted = [PublicKey::parse(OTHER).unwrap(), PublicKey::parse(SIGNER).unwrap()];
        let verified = verify_commit(COMMIT, &trusted).unwrap();
        assert_eq!(verified, VerifiedCommit { time: 1_700_000_000, signer: "registry-test".into() });
    }

    #[test]
    fn refuses_unknown_keys_changes_and_unsigned_commits() {
        let trusted = [PublicKey::parse(SIGNER).unwrap()];
        let other = [PublicKey::parse(OTHER).unwrap()];
        assert!(verify_commit(COMMIT, &other).unwrap_err().to_string().contains("unknown key"));
        let tampered = COMMIT.replace("Registry at abc", "Registry at abd");
        assert!(verify_commit(&tampered, &trusted).unwrap_err().to_string().contains("does not match"));
        let moved = COMMIT.replace("5b510de3", "00000000");
        assert!(verify_commit(&moved, &trusted).is_err());
        let unsigned = "tree 5b51\ncommitter CI <ci@x> 1 +0000\n\nhi\n";
        assert!(verify_commit(unsigned, &trusted).unwrap_err().to_string().contains("not signed"));
    }

    #[test]
    fn signs_like_git() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let trusted = [PublicKey::parse(&public_line(&key, "test")).unwrap()];
        let signed = sign_commit("tree 1\ncommitter A <a@a> 42 +0000\n\nmsg\n", &key);
        assert_eq!(verify_commit(&signed, &trusted).unwrap().time, 42);
    }

    #[test]
    fn parses_keys() {
        assert_eq!(PublicKey::parse(SIGNER).unwrap().comment, "registry-test");
        assert!(PublicKey::parse("ssh-rsa AAAAB3NzaC1yc2E= me").is_err());
        assert!(PublicKey::parse("nonsense").is_err());
    }
}
