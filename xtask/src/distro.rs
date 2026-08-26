//! `distro …`: build and manage the Willie root filesystem image.
//!
//! The base is the official Debian *slim* root filesystem published for
//! container images, pinned by commit and sha256 in `distro/base.lock`.
//! Downloads use the Windows built-in `curl.exe` (it honours the system
//! proxy and certificate store); hashing is done in-process.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

pub const BASE_REPO: &str = "debuerreotype/docker-debian-artifacts";
pub const BASE_BRANCH: &str = "dist-amd64";
pub const BASE_FILE: &str = "trixie/slim/rootfs.tar.xz";
pub const LOCK_PATH: &str = "distro/base.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseLock {
    pub url: String,
    pub commit: String,
    pub sha256: String,
    pub pinned_at: String,
}

impl BaseLock {
    /// `key = value` lines, `#` comments; unknown keys ignored.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut url = None;
        let mut commit = None;
        let mut sha256 = None;
        let mut pinned_at = None;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("bad lock line `{line}`"))?;
            let value = value.trim().to_owned();
            match key.trim() {
                "url" => url = Some(value),
                "commit" => commit = Some(value),
                "sha256" => sha256 = Some(value.to_lowercase()),
                "pinned_at" => pinned_at = Some(value),
                _ => {}
            }
        }
        Ok(Self {
            url: url.ok_or("lock missing `url`")?,
            commit: commit.ok_or("lock missing `commit`")?,
            sha256: sha256.ok_or("lock missing `sha256`")?,
            pinned_at: pinned_at.unwrap_or_default(),
        })
    }

    pub fn render(&self) -> String {
        format!(
            "# Base root filesystem for the Willie distribution image.\n\
             # Written by `cargo xtask distro pin`; verified by `distro fetch`.\n\
             url = {}\ncommit = {}\nsha256 = {}\npinned_at = {}\n",
            self.url, self.commit, self.sha256, self.pinned_at
        )
    }
}

pub fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(hex_digest(&bytes))
}

fn curl(url: &str, out: &Path) -> Result<(), String> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let status = Command::new("curl.exe")
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(out)
        .arg(url)
        .status()
        .map_err(|e| format!("cannot run curl.exe: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("download of {url} failed with {status}"))
    }
}

fn current_commit() -> Result<String, String> {
    let api = format!(
        "https://api.github.com/repos/{BASE_REPO}/commits/{BASE_BRANCH}"
    );
    let output = Command::new("curl.exe")
        .args(["-fsSL", "-H", "Accept: application/vnd.github.sha", &api])
        .output()
        .map_err(|e| format!("cannot run curl.exe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot resolve branch {BASE_BRANCH}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.len() != 40 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("unexpected commit response `{sha}`"));
    }
    Ok(sha)
}

pub fn pin(root: &Path) -> Result<(), String> {
    let commit = current_commit()?;
    let url = format!(
        "https://raw.githubusercontent.com/{BASE_REPO}/{commit}/{BASE_FILE}"
    );
    let target = root.join("target/distro/base/rootfs.tar.xz");
    curl(&url, &target)?;
    let sha256 = sha256_file(&target)?;
    let pinned_at = Command::new("git")
        .args(["log", "-1", "--format=%cs"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    let lock = BaseLock {
        url,
        commit,
        sha256,
        pinned_at,
    };
    fs::write(root.join(LOCK_PATH), lock.render())
        .map_err(|e| format!("cannot write {LOCK_PATH}: {e}"))?;
    println!(
        "pinned {}\n  commit {}\n  sha256 {}",
        lock.url, lock.commit, lock.sha256
    );
    Ok(())
}

pub fn read_lock(root: &Path) -> Result<BaseLock, String> {
    let text = fs::read_to_string(root.join(LOCK_PATH)).map_err(|e| {
        format!(
            "cannot read {LOCK_PATH}: {e} — run `cargo xtask distro pin` first"
        )
    })?;
    BaseLock::parse(&text)
}

pub fn fetch(root: &Path) -> Result<PathBuf, String> {
    let lock = read_lock(root)?;
    let target = root.join("target/distro/base/rootfs.tar.xz");
    if target.is_file() && sha256_file(&target)? == lock.sha256 {
        println!("base rootfs present and verified: {}", target.display());
        return Ok(target);
    }
    curl(&lock.url, &target)?;
    let actual = sha256_file(&target)?;
    if actual != lock.sha256 {
        let _ = fs::remove_file(&target);
        return Err(format!(
            "sha256 mismatch for base rootfs: expected {} got {actual}",
            lock.sha256
        ));
    }
    println!("base rootfs downloaded and verified: {}", target.display());
    Ok(target)
}

pub fn run(root: &Path, args: &[String]) -> crate::TaskResult {
    match args.first().map(String::as_str) {
        Some("pin") => pin(root),
        Some("fetch") => fetch(root).map(drop),
        other => Err(format!(
            "unknown distro command {other:?}; expected pin | fetch"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_round_trips_through_render_and_parse() {
        let lock = BaseLock {
            url: "https://example.invalid/rootfs.tar.xz".into(),
            commit: "abc123".into(),
            sha256: "00ff".into(),
            pinned_at: "2026-08-25".into(),
        };
        assert_eq!(BaseLock::parse(&lock.render()).unwrap(), lock);
    }

    #[test]
    fn lock_without_sha256_is_rejected() {
        assert!(BaseLock::parse("url = x\ncommit = y\n").is_err());
    }

    #[test]
    fn sha256_of_empty_input_is_the_known_constant() {
        assert_eq!(
            hex_digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
