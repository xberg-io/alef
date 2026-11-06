//! Input fingerprint of an executable component (`producer_language = "executable"`).
//!
//! Unlike a Rust producer there is no cargo workspace to resolve: the fingerprint covers the
//! declared input files, the build command and the toolchain the consumer says built it. Each input
//! is one `<len>:<key>=<len>:<value>\n` record fed to the digest, in this order:
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
pub const EXECUTABLE_FINGERPRINT_VERSION: u32 = 2;

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
        digest.update(format!("{}:{key}={}:{value}\n", key.len(), value.len()).as_bytes());
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
/// A pattern that names an existing path is taken literally, before it is tried as a glob. A
/// directory matches every file below it. `.git` directories and anything under an `exclude`
/// directory (alef's own build output) are skipped, however the file was matched.
///
/// Symbolic links are never followed: a link is hashed as its target string, so a dangling or
/// cyclic link is an ordinary input, and retargeting it changes the hash. The one exception is a
/// literal pattern that is itself a link to a directory (a symlinked `producer_dir`): its direct
/// contents are read, while links below it stay unfollowed.
///
/// Labels are the matched paths relative to `config_dir` (the directory of `alef.toml`, as a path
/// relative to the working directory) with `/` separators, so the same tree hashes identically
/// whichever directory alef runs from.
///
/// # Errors
///
/// Fails on an invalid pattern, an unreadable file, or when nothing matches: a fingerprint over no
/// files would reuse a stale build for any change.
pub fn executable_inputs(patterns: &[String], config_dir: &Path, exclude: &[PathBuf]) -> Result<Vec<(String, String)>> {
    let scope = Scope::new(config_dir, exclude);
    let mut files = BTreeMap::new();
    for pattern in patterns {
        let mut matched = BTreeMap::new();
        let direct = PathBuf::from(pattern);
        if direct.symlink_metadata().is_ok() {
            collect(&direct, &scope, true, &mut matched)?;
        } else {
            for entry in glob::glob(pattern).with_context(|| format!("invalid inputs pattern `{pattern}`"))? {
                let path = entry.with_context(|| format!("cannot read a match of inputs pattern `{pattern}`"))?;
                collect(&path, &scope, false, &mut matched)?;
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

/// Where labels are rooted and what is never an input.
struct Scope {
    config_prefix: String,
    exclude: Vec<PathBuf>,
}

impl Scope {
    fn new(config_dir: &Path, exclude: &[PathBuf]) -> Self {
        let prefix = slashed(config_dir);
        Self {
            config_prefix: if prefix.is_empty() || prefix == "." {
                String::new()
            } else {
                format!("{}/", prefix.trim_end_matches('/'))
            },
            exclude: exclude.iter().map(|path| absolute_clean(path)).collect(),
        }
    }

    /// Label of `path`: relative to the config directory when it lies below it.
    fn label(&self, path: &Path) -> String {
        let text = slashed(path);
        let text = text.strip_prefix("./").unwrap_or(&text);
        text.strip_prefix(&self.config_prefix).unwrap_or(text).to_string()
    }

    fn skips(&self, path: &Path) -> bool {
        let label = self.label(path);
        label.split('/').any(|segment| segment == ".git")
            || (!self.exclude.is_empty() && {
                let absolute = absolute_clean(path);
                self.exclude.iter().any(|excluded| absolute.starts_with(excluded))
            })
    }
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn absolute_clean(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    crate::core::config::component::lexically_clean(&absolute)
}

fn collect(path: &Path, scope: &Scope, follow_root: bool, files: &mut BTreeMap<String, String>) -> Result<()> {
    if scope.skips(path) {
        return Ok(());
    }
    let metadata = path
        .symlink_metadata()
        .with_context(|| format!("failed to stat {}", path.display()))?;
    let is_dir = metadata.is_dir() || (follow_root && metadata.is_symlink() && path.is_dir());
    if is_dir {
        let mut children = std::fs::read_dir(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("failed to read {}", path.display()))?;
        children.sort();
        for child in children {
            collect(&child, scope, false, files)?;
        }
    } else if metadata.is_symlink() {
        let target = std::fs::read_link(path).with_context(|| format!("failed to read link {}", path.display()))?;
        let record = format!("symlink:{}", slashed(&target));
        files.insert(scope.label(path), sha256_bytes(record.as_bytes()));
    } else if metadata.is_file() {
        files.insert(scope.label(path), sha256_file(path)?);
    }
    Ok(())
}

/// Locate the POSIX shell every executable component command runs through.
///
/// # Errors
///
/// Fails with an actionable message when no `sh` is on `path` (the value of `PATH`).
pub fn ensure_shell(path: Option<&std::ffi::OsStr>) -> Result<()> {
    let found = path.is_some_and(|path| {
        std::env::split_paths(path).any(|dir| ["sh", "sh.exe"].iter().any(|name| dir.join(name).is_file()))
    });
    ensure!(
        found,
        "executable components run their commands with `sh -c`, but no `sh` was found on PATH; install a POSIX \
         shell (on Windows, Git for Windows or MSYS2) and put it on PATH"
    );
    Ok(())
}

/// Trimmed standard output of `command`, run with `sh -c` in `dir`.
///
/// # Errors
///
/// Fails when the command cannot start, exits unsuccessfully or prints nothing: an unidentifiable
/// toolchain must not silently hash as an empty one.
pub fn toolchain_identity(command: &str, dir: &Path) -> Result<String> {
    ensure_shell(std::env::var_os("PATH").as_deref())?;
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
