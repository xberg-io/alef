use super::version_core::read_version;
use super::version_swift::compute_sha256_hex;
use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::{Context, ensure};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    repository: String,
    source: String,
    version: String,
    archive: String,
    sha256: String,
    triples: Vec<String>,
    artifact_version: String,
}

pub(super) struct PreparedSwiftBundle {
    path: PathBuf,
    receipt_path: PathBuf,
    receipt: Receipt,
}

fn git(arguments: &[&str]) -> anyhow::Result<String> {
    let output = std::process::Command::new("git").args(arguments).output()?;
    ensure!(
        output.status.success(),
        "Swift prebuilt preparation requires a Git source: git {} failed",
        arguments[0]
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn candidates(config: &ResolvedCrateConfig) -> anyhow::Result<Vec<PathBuf>> {
    let directory = Path::new("dist/swift-artifactbundle");
    if !config.languages.contains(&Language::Swift) || !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory).context("reading Swift prebuilt staging")? {
        let path = entry?.path();
        let zip = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));
        let receipt = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".release.json"));
        if zip || receipt {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

pub(super) fn has_prebuilt(config: &ResolvedCrateConfig) -> anyhow::Result<bool> {
    Ok(!candidates(config)?.is_empty())
}

pub(super) fn require_no_prebuilt(config: &ResolvedCrateConfig) -> anyhow::Result<()> {
    ensure!(
        candidates(config)?.is_empty(),
        "Swift prebuilt files appeared after version-sync preparation; retry from clean source"
    );
    Ok(())
}

fn repository(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let url = config.try_github_repo().map_err(anyhow::Error::msg)?;
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let name = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("git@github.com:"));
    let name = name.context("Swift prebuilt receipt requires the configured GitHub repository identity")?;
    ensure!(
        name.split('/').count() == 2 && !name.split('/').any(str::is_empty),
        "invalid Swift repository identity"
    );
    Ok(name.to_string())
}

fn read_receipt(path: &Path) -> anyhow::Result<Receipt> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "Swift preparation receipt must be a regular file"
    );
    serde_json::from_slice(&std::fs::read(path)?).context("invalid Swift preparation receipt")
}

impl PreparedSwiftBundle {
    pub(super) fn discover(
        config: &ResolvedCrateConfig,
        current_version: &str,
        target_version: &str,
    ) -> anyhow::Result<Option<Self>> {
        let supplied = candidates(config)?;
        if supplied.is_empty() {
            return Ok(None);
        }
        let archive = format!("{}-rs.artifactbundle.zip", config.swift_module());
        let receipt_name = format!("{}-rs.artifactbundle.release.json", config.swift_module());
        let path = Path::new("dist/swift-artifactbundle").join(&archive);
        let receipt_path = Path::new("dist/swift-artifactbundle").join(receipt_name);
        let expected: BTreeSet<_> = [&path, &receipt_path].into_iter().cloned().collect();
        ensure!(
            supplied.into_iter().collect::<BTreeSet<_>>() == expected,
            "Swift prebuilt staging requires exactly one canonical ZIP and its preparation receipt: {archive}"
        );
        ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_file(),
            "Swift prebuilt ZIP must be a regular file"
        );
        let receipt = read_receipt(&receipt_path)?;
        validate_receipt_identity(config, &receipt, &archive, current_version, target_version)?;
        require_clean_source(config, &receipt.source)?;
        let bytes = std::fs::read(&path)?;
        ensure!(
            compute_sha256_hex(&bytes) == receipt.sha256,
            "Swift preparation digest mismatch"
        );
        verify_zip(&bytes, &config.swift_module(), &receipt)?;
        Ok(Some(Self {
            path,
            receipt_path,
            receipt,
        }))
    }

    pub(super) fn recheck(&self, config: &ResolvedCrateConfig) -> anyhow::Result<&str> {
        ensure!(
            git(&["rev-parse", "HEAD"])? == self.receipt.source,
            "Swift preparation HEAD changed during version sync"
        );
        ensure!(
            read_version(&config.version_from)? == self.receipt.version,
            "Swift preparation version changed during version sync"
        );
        ensure!(
            read_receipt(&self.receipt_path)? == self.receipt,
            "Swift preparation receipt changed during version sync"
        );
        ensure!(
            std::fs::symlink_metadata(&self.path)?.file_type().is_file(),
            "Swift prebuilt ZIP must remain a regular file"
        );
        ensure!(
            compute_sha256_hex(&std::fs::read(&self.path)?) == self.receipt.sha256,
            "Swift preparation ZIP changed during version sync"
        );
        Ok(&self.receipt.sha256)
    }
}

fn validate_receipt_identity(
    config: &ResolvedCrateConfig,
    receipt: &Receipt,
    archive: &str,
    current_version: &str,
    target_version: &str,
) -> anyhow::Result<()> {
    ensure!(receipt.schema == 1, "unsupported Swift preparation receipt schema");
    ensure!(
        receipt.repository == repository(config)?,
        "Swift preparation repository mismatch"
    );
    ensure!(receipt.archive == archive, "Swift preparation archive mismatch");
    ensure!(
        receipt.version == current_version && receipt.version == target_version,
        "Swift preparation version must match clean source and requested final version"
    );
    ensure!(
        receipt.source.len() == 40
            && receipt
                .source
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid Swift preparation source identity"
    );
    Ok(())
}

fn require_clean_source(config: &ResolvedCrateConfig, source: &str) -> anyhow::Result<()> {
    ensure!(
        source == git(&["rev-parse", "HEAD"])?,
        "Swift preparation source differs from current HEAD"
    );
    ensure!(
        git(&["status", "--porcelain", "--untracked-files=normal"])?.is_empty(),
        "Swift prebuilt reuse requires clean preparation source"
    );
    git(&["ls-files", "--error-unmatch", "--", &config.version_from])
        .context("Swift preparation version source must be tracked at HEAD")?;
    Ok(())
}

fn safe_member(name: &str) -> bool {
    !name.starts_with('/') && !name.contains('\\') && !name.split('/').any(|part| part == "..")
}

fn verify_zip(bytes: &[u8], module: &str, receipt: &Receipt) -> anyhow::Result<()> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("invalid Swift prebuilt ZIP")?;
    let root = format!("{module}-rs.artifactbundle");
    let mut names = BTreeSet::new();
    for index in 0..zip.len() {
        let mut member = zip.by_index(index)?;
        let name = member.name()?;
        ensure!(
            safe_member(&name) && name.split('/').next() == Some(&root),
            "unexpected Swift bundle entry"
        );
        ensure!(names.insert(name.to_string()), "duplicate Swift bundle entry");
        std::io::copy(&mut member, &mut std::io::sink()).context("Swift bundle member integrity validation failed")?;
    }
    let mut text = String::new();
    zip.by_name(&format!("{root}/info.json"))?.read_to_string(&mut text)?;
    let info: serde_json::Value = serde_json::from_str(&text)?;
    ensure!(
        info["schemaVersion"] == "1.0",
        "unsupported Swift artifact metadata schema"
    );
    let artifacts = info["artifacts"]
        .as_object()
        .context("missing Swift artifact identity")?;
    ensure!(
        artifacts.len() == 1 && artifacts.contains_key("RustBridgeBinary"),
        "unexpected Swift artifact identity"
    );
    let artifact = &artifacts["RustBridgeBinary"];
    ensure!(
        artifact["type"] == "staticLibrary",
        "Swift artifact must be a static library"
    );
    ensure!(
        artifact["version"].as_str() == Some(&receipt.artifact_version),
        "Swift artifact metadata version differs from receipt"
    );
    let variants = artifact["variants"]
        .as_array()
        .context("missing Swift artifact variants")?;
    let observed = verify_variants(&mut zip, &root, variants)?;
    let declared: BTreeSet<_> = receipt.triples.iter().cloned().collect();
    ensure!(
        !observed.is_empty() && declared.len() == receipt.triples.len() && observed == declared,
        "Swift artifact platform declarations differ from receipt"
    );
    Ok(())
}

fn verify_variants(
    zip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    root: &str,
    variants: &[serde_json::Value],
) -> anyhow::Result<BTreeSet<String>> {
    let mut triples = BTreeSet::new();
    for variant in variants {
        let path = variant["path"].as_str().context("missing Swift library path")?;
        ensure!(safe_member(path), "unsafe Swift library path");
        let member = zip.by_name(&format!("{root}/{path}"))?;
        ensure!(
            !member.is_dir() && member.size() > 0,
            "missing Swift native library payload"
        );
        let supported = variant["supportedTriples"]
            .as_array()
            .context("missing Swift supported triples")?;
        ensure!(!supported.is_empty(), "Swift variant has no supported triples");
        for triple in supported {
            let triple = triple.as_str().context("invalid Swift platform triple")?;
            ensure!(
                !triple.is_empty() && triples.insert(triple.to_string()),
                "duplicate or empty Swift platform triple"
            );
        }
    }
    Ok(triples)
}
