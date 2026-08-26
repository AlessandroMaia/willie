//! `distro …`: build and manage the Willie root filesystem image.
//!
//! The base is the official Debian *slim* root filesystem published for
//! container images, pinned by commit and sha256 in `distro/base.lock`.
//! Downloads use the Windows built-in `curl.exe` (it honours the system
//! proxy and certificate store); hashing is done in-process.
//!
//! The publisher stores the image as an OCI layout (`index.json` plus
//! friendly-named blob files) rather than a flat tarball, so pinning
//! walks index -> manifest -> layer, verifying a sha256 at every hop.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const BASE_REPO: &str = "debuerreotype/docker-debian-artifacts";
pub const BASE_BRANCH: &str = "dist-amd64";
pub const BASE_DIR: &str = "trixie/slim/oci";
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

fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("curl.exe")
        .args(["-fsSL", url])
        .output()
        .map_err(|e| format!("cannot run curl.exe: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "download of {url} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn fetch_json(url: &str) -> Result<Value, String> {
    let bytes = fetch_bytes(url)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("cannot parse JSON from {url}: {e}"))
}

/// Fail closed at every hop: a downloaded blob must hash to exactly the
/// digest its parent (index or manifest) declared for it.
fn verify_digest(
    what: &str,
    expected: &str,
    actual: &str,
) -> Result<(), String> {
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "sha256 mismatch for {what}: expected {expected} got {actual}"
        ))
    }
}

fn strip_sha256_prefix(digest: &str) -> Result<String, String> {
    digest
        .strip_prefix("sha256:")
        .map(str::to_owned)
        .ok_or_else(|| {
            format!("digest `{digest}` missing the `sha256:` prefix")
        })
}

/// The image index lists one manifest per platform; pick the linux/amd64
/// one, or the only entry if the index does not disambiguate by platform.
pub fn select_manifest_digest(index: &Value) -> Result<String, String> {
    let manifests = index["manifests"]
        .as_array()
        .ok_or_else(|| "index missing `manifests` array".to_owned())?;
    let chosen = if let [only] = manifests.as_slice() {
        only
    } else {
        let matches: Vec<&Value> = manifests
            .iter()
            .filter(|m| {
                m["platform"]["os"] == "linux"
                    && m["platform"]["architecture"] == "amd64"
            })
            .collect();
        match matches.as_slice() {
            [one] => *one,
            [] => return Err("no linux/amd64 manifest in index".to_owned()),
            many => {
                return Err(format!(
                    "{} linux/amd64 manifests in index, expected 1",
                    many.len()
                ));
            }
        }
    };
    let digest = chosen["digest"]
        .as_str()
        .ok_or_else(|| "manifest entry missing `digest`".to_owned())?;
    strip_sha256_prefix(digest)
}

/// The base image is a single-layer rootfs; reject any other shape.
pub fn select_layer(manifest: &Value) -> Result<(String, String), String> {
    let layers = manifest["layers"]
        .as_array()
        .ok_or_else(|| "manifest missing `layers` array".to_owned())?;
    let [layer] = layers.as_slice() else {
        return Err(format!(
            "manifest has {} layers, expected exactly 1",
            layers.len()
        ));
    };
    let media_type = layer["mediaType"]
        .as_str()
        .ok_or_else(|| "layer missing `mediaType`".to_owned())?
        .to_owned();
    if !media_type.ends_with("tar+gzip") {
        return Err(format!("layer mediaType `{media_type}` is not tar+gzip"));
    }
    let digest = layer["digest"]
        .as_str()
        .ok_or_else(|| "layer missing `digest`".to_owned())?;
    Ok((strip_sha256_prefix(digest)?, media_type))
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
    let base = format!(
        "https://raw.githubusercontent.com/{BASE_REPO}/{commit}/{BASE_DIR}"
    );

    let index = fetch_json(&format!("{base}/index.json"))?;
    let manifest_digest = select_manifest_digest(&index)?;

    let manifest_url = format!("{base}/blobs/image-manifest.json");
    let manifest_bytes = fetch_bytes(&manifest_url)?;
    verify_digest(
        "the base image manifest",
        &manifest_digest,
        &hex_digest(&manifest_bytes),
    )?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("cannot parse JSON from {manifest_url}: {e}"))?;
    let (layer_digest, _media_type) = select_layer(&manifest)?;

    let url = format!("{base}/blobs/rootfs.tar.gz");
    let target = root.join("target/distro/base/rootfs.tar.gz");
    curl(&url, &target)?;
    let sha256 = sha256_file(&target)?;
    if let Err(e) =
        verify_digest("the base rootfs layer", &layer_digest, &sha256)
    {
        let _ = fs::remove_file(&target);
        return Err(e);
    }

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
    let target = root.join("target/distro/base/rootfs.tar.gz");
    if target.is_file() && sha256_file(&target)? == lock.sha256 {
        println!("base rootfs present and verified: {}", target.display());
        return Ok(target);
    }
    curl(&lock.url, &target)?;
    let actual = sha256_file(&target)?;
    if let Err(e) = verify_digest("the base rootfs", &lock.sha256, &actual) {
        let _ = fs::remove_file(&target);
        return Err(e);
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

    #[test]
    fn verify_digest_accepts_a_match() {
        assert!(verify_digest("thing", "aaaa", "aaaa").is_ok());
    }

    #[test]
    fn verify_digest_rejects_a_mismatch() {
        assert!(verify_digest("thing", "aaaa", "bbbb").is_err());
    }

    fn index_with(manifests: &str) -> Value {
        serde_json::from_str(&format!(
            r#"{{"schemaVersion":2,"manifests":[{manifests}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn select_manifest_digest_picks_the_linux_amd64_entry() {
        let index = index_with(
            r#"
            {"digest":"sha256:aaaa","platform":{"os":"linux","architecture":"arm64"}},
            {"digest":"sha256:bbbb","platform":{"os":"linux","architecture":"amd64"}}
            "#,
        );
        assert_eq!(select_manifest_digest(&index).unwrap(), "bbbb");
    }

    #[test]
    fn select_manifest_digest_accepts_the_only_entry_regardless_of_platform() {
        let index = index_with(
            r#"{"digest":"sha256:cccc","platform":{"os":"windows","architecture":"arm"}}"#,
        );
        assert_eq!(select_manifest_digest(&index).unwrap(), "cccc");
    }

    #[test]
    fn select_manifest_digest_rejects_no_linux_amd64_match() {
        let index = index_with(
            r#"
            {"digest":"sha256:aaaa","platform":{"os":"linux","architecture":"arm64"}},
            {"digest":"sha256:bbbb","platform":{"os":"windows","architecture":"amd64"}}
            "#,
        );
        assert!(select_manifest_digest(&index).is_err());
    }

    fn manifest_with(layers: &str) -> Value {
        serde_json::from_str(&format!(
            r#"{{"schemaVersion":2,"layers":[{layers}]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn select_layer_returns_the_digest_hex_without_the_sha256_prefix() {
        let manifest = manifest_with(
            r#"{"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":"sha256:dddd"}"#,
        );
        let (digest, media_type) = select_layer(&manifest).unwrap();
        assert_eq!(digest, "dddd");
        assert_eq!(media_type, "application/vnd.oci.image.layer.v1.tar+gzip");
    }

    #[test]
    fn select_layer_rejects_more_than_one_layer() {
        let manifest = manifest_with(
            r#"
            {"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":"sha256:dddd"},
            {"mediaType":"application/vnd.oci.image.layer.v1.tar+gzip","digest":"sha256:eeee"}
            "#,
        );
        assert!(select_layer(&manifest).is_err());
    }

    #[test]
    fn select_layer_rejects_a_non_gzip_layer() {
        let manifest = manifest_with(
            r#"{"mediaType":"application/vnd.oci.image.layer.v1.tar","digest":"sha256:dddd"}"#,
        );
        assert!(select_layer(&manifest).is_err());
    }
}
