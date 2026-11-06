//! Input fingerprint of an executable component (`producer_language = "executable"`).
//!
//! Unlike a Rust producer there is no cargo workspace to resolve: the fingerprint covers the
//! declared input files, the build command and the toolchain the consumer says built it. Each input
//! is one `key=<len>:<value>\n` record fed to the digest, in this order:
//!
//! 1. `executable-fingerprint-version`, `component` (`<crate>/<component>`), `target`, `profile`
//!    and `file` (the payload file name for the target). The alef version is deliberately absent:
//!    alef does not shape the bytes of a command it only runs, so upgrading alef alone reuses.
//! 2. `build`, the command template exactly as configured (before substitution).
//! 3. `contract:<name>` for every provided contract, sorted by name.
//! 4. `distribution.*`, as for native components: a reused artifact keeps the URL and signing key of
//!    the release that built it.
//! 5. `toolchain`, the SHA-256 of the trimmed output of `toolchain_version_cmd`, or `none`.
//! 6. `env:<NAME>` for every `--env` name that is set, sorted, and `extra:<key>` for every
//!    `--extra key=value`, sorted.
//! 7. `input:<label>` for every file the `inputs` globs match (the producer directory by default),
//!    sorted by label, valued with the SHA-256 of its bytes.
//!
//! Only the files and the build command are portable to the host that assembles the lock, so
//! [`portable_executable_hash`] covers just those.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use super::hex;
use crate::component::artifact::{sha256_bytes, sha256_file};
use crate::core::config::ComponentDistributionConfig;

/// Bump when the set or encoding of hashed inputs changes.
pub const EXECUTABLE_FINGERPRINT_VERSION: u32 = 1;

/// Everything about one executable build except its input files.
#[derive(Debug, Clone)]
pub struct ExecutableFingerprintInput<'a> {
    pub crate_name: &'a str,
    pub component: &'a str,
    pub target: &'a str,
    /// `true` for a release build (the default), `false` for `--debug`.
    pub release: bool,
    /// Payload file name for `target`.
    pub file: &'a str,
    /// The `build` command template as configured.
    pub build: &'a str,
    /// `(contract name, contract hash)` for every provided contract.
    pub contracts: &'a [(String, String)],
    pub distribution: Option<&'a ComponentDistributionConfig>,
    /// Trimmed output of `toolchain_version_cmd`; empty when none is configured.
    pub toolchain: &'a str,
    /// `--env` variables that are set, by name.
    pub env: &'a [(String, String)],
    /// User `--extra key=value` pairs.
    pub extras: &'a [(String, String)],
}

/// Compute the fingerprint from already gathered inputs. Pure and deterministic.
#[must_use]
pub fn compute_executable(input: &ExecutableFingerprintInput<'_>, files: &[(String, String)]) -> String {
    let mut digest = Sha256::new();
    let mut put = |key: &str, value: &str| {
        digest.update(format!("{key}={}:{value}\n", value.len()).as_bytes());
    };
    put(
        "executable-fingerprint-version",
        &EXECUTABLE_FINGERPRINT_VERSION.to_string(),
    );
    put("component", &format!("{}/{}", input.crate_name, input.component));
    put("target", input.target);
    put("profile", if input.release { "release" } else { "debug" });
    put("file", input.file);
    put("build", input.build);
    let contracts = input
        .contracts
        .iter()
        .map(|(name, hash)| (name, hash))
        .collect::<BTreeMap<_, _>>();
    for (name, hash) in contracts {
        put(&format!("contract:{name}"), hash);
    }
    if let Some(distribution) = input.distribution {
        put("distribution.url_template", &distribution.url_template);
        put(
            "distribution.artifact_name_template",
            distribution.artifact_name_template.as_deref().unwrap_or(""),
        );
        for (id, key) in &distribution.public_keys {
            put(&format!("distribution.public_key:{id}"), key);
        }
    }
    put(
        "toolchain",
        &if input.toolchain.is_empty() {
            "none".to_string()
        } else {
            sha256_bytes(input.toolchain.as_bytes())
        },
    );
    let env = input
        .env
        .iter()
        .map(|(key, value)| (key, value))
        .collect::<BTreeMap<_, _>>();
    for (name, value) in env {
        put(&format!("env:{name}"), value);
    }
    let extras = input
        .extras
        .iter()
        .map(|(key, value)| (key, value))
        .collect::<BTreeMap<_, _>>();
    for (key, value) in extras {
        put(&format!("extra:{key}"), value);
    }
    let files = files
        .iter()
        .map(|(key, value)| (key, value))
        .collect::<BTreeMap<_, _>>();
    for (label, hash) in files {
        put(&format!("input:{label}"), hash);
    }
    hex(&digest.finalize())
}

/// The part of an executable's identity the lock assembly host can recompute: the input files and
/// the build command.
#[must_use]
pub fn portable_executable_hash(files: &[(String, String)], build: &str) -> String {
    let files = files
        .iter()
        .map(|(key, value)| (key, value))
        .collect::<BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&(files, build)).expect("executable inputs are serializable");
    sha256_bytes(&bytes)
}

/// `(label, sha256)` of every file `patterns` match, sorted by label.
///
/// A pattern naming a directory matches every file below it (`.git` directories are skipped).
/// Labels are the matched paths with `/` separators, so a checkout at another location hashes the
/// same as long as `patterns` are relative.
///
/// # Errors
///
/// Fails on an invalid pattern, an unreadable file, or when nothing matches: a fingerprint over no
/// files would reuse a stale build for any change.
pub fn executable_inputs(patterns: &[String]) -> Result<Vec<(String, String)>> {
    let mut files = BTreeMap::new();
    for pattern in patterns {
        let mut matched = BTreeMap::new();
        let direct = PathBuf::from(pattern);
        if direct.exists() {
            collect(&direct, &mut matched)?;
        } else {
            for entry in glob::glob(pattern).with_context(|| format!("invalid inputs pattern `{pattern}`"))? {
                let path = entry.with_context(|| format!("cannot read a match of inputs pattern `{pattern}`"))?;
                collect(&path, &mut matched)?;
            }
        }
        ensure!(
            !matched.is_empty(),
            "executable inputs pattern `{pattern}` matched no files"
        );
        files.extend(matched);
    }
    Ok(files.into_iter().collect())
}

fn collect(path: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
    if path.is_dir() {
        let mut children = std::fs::read_dir(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("failed to read {}", path.display()))?;
        children.sort();
        for child in children {
            if child.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            collect(&child, files)?;
        }
        return Ok(());
    }
    let label = path.to_string_lossy().replace('\\', "/");
    let label = label.strip_prefix("./").unwrap_or(&label).to_string();
    files.insert(label, sha256_file(path)?);
    Ok(())
}

/// Trimmed standard output of `command`, run with `sh -c` in `dir`.
///
/// # Errors
///
/// Fails when the command cannot start, exits unsuccessfully or prints nothing: an unidentifiable
/// toolchain must not silently hash as an empty one.
pub fn toolchain_identity(command: &str, dir: &Path) -> Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(dir)
        .output()
        .with_context(|| format!("failed to run toolchain_version_cmd `{command}`"))?;
    if !output.status.success() {
        bail!(
            "toolchain_version_cmd `{command}` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ensure!(
        !identity.is_empty(),
        "toolchain_version_cmd `{command}` printed nothing"
    );
    Ok(identity)
}

#[cfg(test)]
#[path = "executable_tests.rs"]
mod tests;
