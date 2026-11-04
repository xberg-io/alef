//! Input fingerprint of one component build.
//!
//! The fingerprint is the SHA-256 (lowercase hex) of everything that can change the bytes of the
//! built library, so two builds with equal fingerprints may share one artifact. It is recorded in
//! the artifact record (`input_hash`), copied into the lock entry, and compared by
//! `alef component plan`.
//!
//! # Hashed inputs, in order
//!
//! Each input is one `key=<len>:<value>\n` record fed to the digest (the length prefix keeps the
//! encoding unambiguous), in this order:
//!
//! 1. `fingerprint-version`, `alef` (the alef version), `component` (`<crate>/<component>`),
//!    `target`, `profile` (`release` or `debug`), `features` (sorted, comma-joined) and
//!    `default-features`.
//! 2. `contract:<name>` for every provided contract, sorted by name, valued with its hash.
//! 3. `distribution.url_template`, `distribution.artifact_name_template` and
//!    `distribution.public_key:<id>` (sorted by id): a reused artifact keeps the URL and signing
//!    key of the release that built it, so a change to either must rebuild.
//! 4. `rustc` (SHA-256 of the `release:`, `commit-hash:` and `LLVM version:` lines of `rustc -vV`,
//!    never `host:`, so the same compiler on two build machines agrees) and `env:<NAME>` for the
//!    build-affecting environment (see [`build_env`]), sorted by name; unset variables are absent.
//!    `toolchain:<key>` follows, only for `*-apple-*` targets (`apple-sdk:<sdk>:version` and
//!    `apple-sdk:<sdk>:build` from `xcrun --sdk <macosx|iphoneos|iphonesimulator>`, `apple-ld`, the
//!    first line of `ld -v`) and `*-windows-msvc` targets (`msvc:VCToolsVersion` and
//!    `msvc:WindowsSDKVersion` from the environment); a tool or variable that is missing hashes the
//!    marker `absent`. See [`native_toolchain`].
//! 5. `config:<scope>:<depth>:<file>`: SHA-256 of each `.cargo/config.toml`, `.cargo/config`,
//!    `rust-toolchain.toml` and `rust-toolchain` found walking up from the working directory
//!    (`cwd`) and from the producer directory (`producer`) to the filesystem root, plus
//!    `config:cargo-home:*` for `$CARGO_HOME/config.toml`. Labels never contain absolute paths.
//! 6. `manifest:<table>` for the producer workspace root manifest's `profile`, `patch`, `replace`,
//!    `workspace.package` (without `version`), `workspace.lints` and `workspace.dependencies`
//!    (without `version` on path dependencies), rendered with sorted keys.
//! 7. `extra:<key>` for every user `--extra key=value`, sorted by key.
//! 8. One `pkg` record per package in the producer's dependency closure, sorted by their rendered
//!    text. The closure comes from `cargo metadata --manifest-path <producer manifest> --locked`
//!    without `--filter-platform`, a sound superset that includes host-only build dependencies, so
//!    the producer's own workspace and `Cargo.lock` are used (a producer may live in another
//!    workspace than the core crate). Normal and build dependencies are followed; dev dependencies
//!    are not. Metadata unifies features across the whole workspace, so an unrelated member's
//!    feature change can cause a rebuild (over-sensitive, never stale). Each record carries the
//!    package's resolved feature set and either
//!    * registry or git packages: `name@version#source#checksum`, the checksum read from the
//!      producer workspace's `Cargo.lock` (empty for git sources, whose `source` pins the commit);
//!    * path packages: `path:name` and a tree digest, without the version, so a lock-step version
//!      bump that does not touch the package's files leaves the fingerprint alone. See
//!      [`tree_digest`].
//!
//! Workspace members outside the producer's closure are not hashed, so editing an unrelated member
//! does not change the fingerprint. Package versions are excluded on purpose so lock-step releases
//! still reuse; `env!("CARGO_PKG_VERSION")` in a reused artifact reports the version it was built at.
//! The C compiler version is not probed: pass it through `--extra`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

mod env;
mod toolchain;
mod tree;

pub use env::{build_env, cargo_home, implicit_config, process_vars};
pub use toolchain::{ABSENT, SystemProbe, ToolProbe, native_toolchain};
pub use tree::tree_digest;

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::artifact::sha256_bytes;
use crate::core::config::ComponentDistributionConfig;

/// Bump when the set or encoding of hashed inputs changes, so old and new fingerprints never collide.
pub const FINGERPRINT_VERSION: u32 = 1;

/// Everything about one component build that does not come from the producer's cargo workspace.
#[derive(Debug, Clone)]
pub struct FingerprintInput<'a> {
    pub crate_name: &'a str,
    pub component: &'a str,
    pub target: &'a str,
    /// `true` for a release build (the default), `false` for `--debug`.
    pub release: bool,
    pub features: &'a [String],
    pub default_features: bool,
    /// `(contract name, contract hash)` for every provided contract.
    pub contracts: &'a [(String, String)],
    pub distribution: Option<&'a ComponentDistributionConfig>,
    pub alef_version: &'a str,
    /// Output of `rustc -vV`; only its identity lines are hashed (see [`rustc_identity`]).
    pub rustc: &'a str,
    /// Build-affecting environment, as filtered by [`build_env`].
    pub env: &'a [(String, String)],
    /// Native toolchain identity from [`native_toolchain`]; empty for non-Apple, non-MSVC targets.
    pub toolchain: &'a [(String, String)],
    /// User `--extra key=value` pairs, as parsed by [`parse_extras`].
    pub extras: &'a [(String, String)],
}

/// The parts of the fingerprint derived from the producer's cargo workspace and the checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProducerInputs {
    /// Rendered `pkg` records of the dependency closure.
    pub packages: Vec<String>,
    /// Canonical rendering of the workspace root's `profile`, `patch`, `replace`, `workspace.package`,
    /// `workspace.lints` and `workspace.dependencies` tables.
    pub manifest_tables: Vec<(String, String)>,
    /// `(label, sha256)` of implicit build configuration files.
    pub config_files: Vec<(String, String)>,
}

/// Compute the fingerprint from already gathered inputs. Pure and deterministic.
#[must_use]
pub fn compute(input: &FingerprintInput<'_>, producer: &ProducerInputs) -> String {
    let mut digest = Sha256::new();
    let mut put = |key: &str, value: &str| {
        digest.update(format!("{key}={}:{value}\n", value.len()).as_bytes());
    };
    put("fingerprint-version", &FINGERPRINT_VERSION.to_string());
    put("alef", input.alef_version);
    put("component", &format!("{}/{}", input.crate_name, input.component));
    put("target", input.target);
    put("profile", if input.release { "release" } else { "debug" });
    let features = input.features.iter().collect::<BTreeSet<_>>();
    put(
        "features",
        &features.into_iter().map(String::as_str).collect::<Vec<_>>().join(","),
    );
    put("default-features", &input.default_features.to_string());

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
    put("rustc", &sha256_bytes(rustc_identity(input.rustc).as_bytes()));
    for (name, value) in sorted(input.env) {
        put(&format!("env:{name}"), value);
    }
    for (name, value) in sorted(input.toolchain) {
        put(&format!("toolchain:{name}"), value);
    }
    for (label, hash) in &producer.config_files {
        put(&format!("config:{label}"), hash);
    }
    for (table, rendered) in &producer.manifest_tables {
        put(&format!("manifest:{table}"), rendered);
    }
    for (key, value) in sorted(input.extras) {
        put(&format!("extra:{key}"), value);
    }
    let mut packages = producer.packages.clone();
    packages.sort();
    for package in &packages {
        put("pkg", package);
    }
    hex(&digest.finalize())
}

fn sorted(pairs: &[(String, String)]) -> Vec<(&String, &String)> {
    let mut sorted = pairs.iter().map(|(key, value)| (key, value)).collect::<Vec<_>>();
    sorted.sort();
    sorted
}

/// Gather the producer-side inputs: dependency closure, manifest tables and implicit config.
///
/// `cwd` is the directory alef runs cargo from (where `.cargo/config.toml` and
/// `rust-toolchain.toml` are discovered).
///
/// # Errors
///
/// Fails when `cargo metadata` fails (including a missing or stale `Cargo.lock`), when the producer
/// is not in its own metadata, or when a file cannot be read.
pub fn producer_inputs(producer_manifest: &Path, cwd: &Path) -> Result<ProducerInputs> {
    let metadata = cargo_metadata(producer_manifest)?;
    let workspace_root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .context("cargo metadata has no workspace_root")?;
    let workspace_root = PathBuf::from(workspace_root);
    let checksums = lock_checksums(&workspace_root.join("Cargo.lock"))?;
    let producer_dir = producer_manifest.parent().unwrap_or(Path::new("."));
    Ok(ProducerInputs {
        packages: closure_records(&metadata, producer_manifest, &checksums)?,
        manifest_tables: manifest_tables(&workspace_root.join("Cargo.toml"))?,
        config_files: implicit_config(&[("cwd", cwd), ("producer", producer_dir)], cargo_home().as_deref())?,
    })
}

/// `cargo metadata` for the producer's own workspace against its `Cargo.lock`, for every platform.
///
/// # Errors
///
/// Fails when cargo cannot be run or exits non-zero.
pub fn cargo_metadata(producer_manifest: &Path) -> Result<Value> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .arg("--manifest-path")
        .arg(producer_manifest)
        .output()
        .context("failed to run `cargo metadata`")?;
    ensure!(
        output.status.success(),
        "`cargo metadata` failed for {} (the producer workspace needs a committed, current Cargo.lock): {}",
        producer_manifest.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    serde_json::from_slice(&output.stdout).context("failed to parse `cargo metadata` output")
}

/// `(name, version, source) -> checksum` for every package in a `Cargo.lock`.
///
/// # Errors
///
/// Fails when the lock cannot be read or parsed.
pub fn lock_checksums(lock_path: &Path) -> Result<BTreeMap<(String, String, String), String>> {
    let text = std::fs::read_to_string(lock_path).with_context(|| format!("failed to read {}", lock_path.display()))?;
    let lock = text
        .parse::<toml::Table>()
        .with_context(|| format!("failed to parse {}", lock_path.display()))?;
    let mut checksums = BTreeMap::new();
    for package in lock
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = |name: &str| {
            package
                .get(name)
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        checksums.insert((field("name"), field("version"), field("source")), field("checksum"));
    }
    Ok(checksums)
}

/// Render the `pkg` records for the producer's dependency closure (normal and build edges).
///
/// # Errors
///
/// Fails when the producer is not a package of `metadata` or a path package cannot be digested.
pub fn closure_records(
    metadata: &Value,
    producer_manifest: &Path,
    checksums: &BTreeMap<(String, String, String), String>,
) -> Result<Vec<String>> {
    let array = |value: &Value, key: &str| value.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let packages = array(metadata, "packages");
    let by_id = packages
        .iter()
        .filter_map(|package| Some((package.get("id")?.as_str()?, package)))
        .collect::<BTreeMap<_, _>>();
    let nodes = metadata
        .get("resolve")
        .map(|resolve| array(resolve, "nodes"))
        .unwrap_or_default();
    let by_node = nodes
        .iter()
        .filter_map(|node| Some((node.get("id")?.as_str()?, node)))
        .collect::<BTreeMap<_, _>>();

    let wanted = producer_manifest
        .canonicalize()
        .with_context(|| format!("producer manifest {} does not exist", producer_manifest.display()))?;
    let root = packages
        .iter()
        .find(|package| {
            package
                .get("manifest_path")
                .and_then(Value::as_str)
                .and_then(|path| Path::new(path).canonicalize().ok())
                .is_some_and(|path| path == wanted)
        })
        .and_then(|package| package.get("id")?.as_str())
        .with_context(|| {
            format!(
                "{} is not a package of its own cargo metadata",
                producer_manifest.display()
            )
        })?;

    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = by_node.get(id) else { continue };
        for dep in array(node, "deps") {
            let normal_or_build = array(&dep, "dep_kinds")
                .iter()
                .any(|kind| matches!(kind.get("kind").and_then(Value::as_str), None | Some("build")));
            if let Some(dep_id) = dep.get("pkg").and_then(Value::as_str)
                && normal_or_build
                && let Some((key, _)) = by_id.get_key_value(dep_id)
            {
                stack.push(*key);
            }
        }
    }

    let mut records = Vec::with_capacity(seen.len());
    for id in seen {
        let Some(package) = by_id.get(id) else { continue };
        let text = |key: &str| package.get(key).and_then(Value::as_str).unwrap_or("");
        let mut features = by_node
            .get(id)
            .map(|node| array(node, "features"))
            .unwrap_or_default()
            .iter()
            .filter_map(|feature| feature.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        features.sort();
        let features = features.join(",");
        let source = package.get("source").and_then(Value::as_str);
        let record = match source {
            None => {
                let dir = Path::new(text("manifest_path"))
                    .parent()
                    .context("path package manifest has no parent directory")?;
                format!("path:{} tree={} features={features}", text("name"), tree_digest(dir)?)
            }
            Some(source) => {
                let checksum = checksums
                    .get(&(
                        text("name").to_string(),
                        text("version").to_string(),
                        source.to_string(),
                    ))
                    .map_or("", String::as_str);
                format!(
                    "{}@{}#{source}#{checksum} features={features}",
                    text("name"),
                    text("version")
                )
            }
        };
        records.push(record);
    }
    records.sort();
    Ok(records)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Canonical renderings of the workspace root manifest's tables that feed member builds:
/// `profile`, `patch`, `replace`, `workspace.package` (without `version`, which lock-step releases
/// change), `workspace.lints` and `workspace.dependencies` (without `version` on path dependencies,
/// for the same reason).
fn manifest_tables(manifest: &Path) -> Result<Vec<(String, String)>> {
    let text = std::fs::read_to_string(manifest).with_context(|| format!("failed to read {}", manifest.display()))?;
    let document = text
        .parse::<toml::Table>()
        .with_context(|| format!("failed to parse {}", manifest.display()))?;
    Ok(render_manifest_tables(&document))
}

fn render_manifest_tables(document: &toml::Table) -> Vec<(String, String)> {
    let workspace = document.get("workspace").and_then(toml::Value::as_table);
    let mut tables = Vec::new();
    for name in ["profile", "patch", "replace"] {
        tables.push((name.to_string(), document.get(name).cloned()));
    }
    let mut package = workspace.and_then(|ws| ws.get("package")).cloned();
    if let Some(toml::Value::Table(package)) = &mut package {
        package.remove("version");
    }
    tables.push(("workspace.package".to_string(), package));
    tables.push((
        "workspace.lints".to_string(),
        workspace.and_then(|ws| ws.get("lints")).cloned(),
    ));
    let mut dependencies = workspace.and_then(|ws| ws.get("dependencies")).cloned();
    if let Some(toml::Value::Table(dependencies)) = &mut dependencies {
        for (_, dependency) in dependencies.iter_mut() {
            if let toml::Value::Table(dependency) = dependency
                && dependency.contains_key("path")
            {
                dependency.remove("version");
            }
        }
    }
    tables.push(("workspace.dependencies".to_string(), dependencies));
    tables
        .into_iter()
        .map(|(name, value)| {
            let mut rendered = String::new();
            if let Some(value) = &value {
                render_toml(value, &mut rendered);
            }
            (name, rendered)
        })
        .collect()
}

/// Render a TOML value with table keys sorted, so key order in the file never matters.
fn render_toml(value: &toml::Value, out: &mut String) {
    match value {
        toml::Value::Table(table) => {
            out.push('{');
            for (key, value) in table.iter().collect::<BTreeMap<_, _>>() {
                let _ = write!(out, "{key:?}:");
                render_toml(value, out);
                out.push(',');
            }
            out.push('}');
        }
        toml::Value::Array(items) => {
            out.push('[');
            for item in items {
                render_toml(item, out);
                out.push(',');
            }
            out.push(']');
        }
        toml::Value::String(text) => {
            let _ = write!(out, "{text:?}");
        }
        other => out.push_str(&other.to_string()),
    }
}

/// `rustc -vV` of the toolchain cargo will use from the current directory.
///
/// # Errors
///
/// Fails when `rustc` cannot be run.
pub fn rustc_version() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("failed to run `rustc -vV`")?;
    ensure!(output.status.success(), "`rustc -vV` failed");
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The compiler identity inside `rustc -vV`: its `release:`, `commit-hash:` and `LLVM version:`
/// lines. `host:` is left out because the same compiler reports a different host on each build
/// machine. When none of the three lines is present the whole text minus `host:` is used.
#[must_use]
pub fn rustc_identity(version_output: &str) -> String {
    let lines = |keep: &dyn Fn(&str) -> bool| {
        version_output
            .lines()
            .map(str::trim)
            .filter(|line| keep(line))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let identity = lines(&|line| {
        line.starts_with("release:") || line.starts_with("commit-hash:") || line.starts_with("LLVM version:")
    });
    if identity.is_empty() {
        return lines(&|line| !line.starts_with("host:"));
    }
    identity
}

/// Parse repeated `--extra key=value` flags into sorted pairs.
///
/// # Errors
///
/// Fails on a flag without `=`, an empty key, or a repeated key.
pub fn parse_extras(raw: &[String]) -> Result<Vec<(String, String)>> {
    let mut extras = BTreeMap::new();
    for item in raw {
        let Some((key, value)) = item.split_once('=') else {
            bail!("--extra `{item}` must be `key=value`");
        };
        ensure!(!key.is_empty(), "--extra `{item}` has an empty key");
        ensure!(
            extras.insert(key.to_string(), value.to_string()).is_none(),
            "--extra key `{key}` given more than once"
        );
    }
    Ok(extras.into_iter().collect())
}

#[cfg(test)]
#[path = "fingerprint/tests.rs"]
mod tests;
