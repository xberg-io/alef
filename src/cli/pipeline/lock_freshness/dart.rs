//! Fail a generation run whose generated `pubspec.yaml` is vouched for beside a committed
//! `pubspec.lock` that no longer resolves against it.
//!
//! A Dart-specific stale-pin reader already existed internally in
//! `crate::cli::pipeline::version_lockfiles` (`stale_dart_pins`), but only to decide whether
//! `dart pub get` needs to run again after a version-sync pass -- a disagreement it found was
//! never surfaced as a finding a stage could fail on, unlike every other ecosystem in this
//! module family. This module closes that gap with its own independent reader (not a reuse of
//! `stale_dart_pins`, which has no per-manifest attribution to report and is scoped to a
//! version-sync `relock` decision, not a generation-time gate) so `alef generate`/`alef all` stop
//! exiting 0 over a Dart pin `dart pub get --offline` would reject exactly the way they already
//! do for Cargo, pnpm, and uv.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::{registered_unmarkable_manifest_dirs, registry_self_dependency};

/// Dependency buckets alef itself writes into a generated `pubspec.yaml` -- see
/// `crate::e2e::codegen::dart::project::render_pubspec`, which only ever populates
/// `dependencies`.
const DART_DEPENDENCY_BUCKETS: [&str; 2] = ["dependencies", "dev_dependencies"];

/// One `pubspec.yaml` version constraint whose sibling `pubspec.lock` pins no version that
/// satisfies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleDartLockFinding {
    /// The committed `pubspec.lock` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `pubspec.yaml` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Which dependency table the requirement was declared in.
    pub(crate) bucket: &'static str,
    /// Package name as `pubspec.yaml` spells it.
    pub(crate) dependency: String,
    /// The constraint text as written in `pubspec.yaml`.
    pub(crate) requirement: String,
    /// The version `pubspec.lock` pins for this package.
    pub(crate) locked_version: String,
}

/// Check every directory in which this run generated a `pubspec.yaml` for a committed
/// `pubspec.lock` that contradicts it, returning the failure to record when one does.
///
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_dart_lock_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
    check_generated_dart_lock_freshness_tolerating_pending_publish(generated_paths, base_dir, None)
}

/// The shared collection step behind [`check_generated_dart_lock_freshness`] and
/// [`check_generated_dart_lock_freshness_tolerating_pending_publish`].
///
/// `pubspec.yaml` is `generated_header: false` (see
/// `crate::e2e::codegen::dart::DartE2eCodegen::generate`) -- scaffolded once and thereafter
/// user-owned -- so it shares the exact structural blind spot [`super::node`]'s doc comment
/// documents for `package.json`, and is closed the same way:
/// [`registered_unmarkable_manifest_dirs`] extends `generated_paths` with every `pubspec.yaml`
/// directory the committed ownership record already knows about.
fn collect_generated_dart_lock_findings(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Vec<StaleDartLockFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("pubspec.yaml") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    directories.extend(registered_unmarkable_manifest_dirs(base_dir, "pubspec.yaml"));
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_dart_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated pubspec.yaml files against their committed pubspec.lock"
    );
    findings
}

/// Same check as [`check_generated_dart_lock_freshness`], except a finding fully explained by
/// this crate's own pending, not-yet-published registry-mode `test_apps` self-dependency is
/// downgraded to a `tracing::warn!` instead of failing the stage -- the Dart sibling of
/// [`super::cargo::check_generated_lock_freshness_tolerating_pending_publish`]. See
/// [`registry_self_dependency`]'s doc for what "explained" means here and why it is deliberately
/// conservative. `normalize` is the identity function: `render_pubspec`'s registry mode writes
/// the configured version verbatim with no cargo-to-pub.dev conversion (unlike Ruby's
/// `to_rubygems_prerelease`), matching the node check's own verbatim pass-through.
pub(crate) fn check_generated_dart_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_dart_lock_findings(generated_paths, base_dir);
    if findings.is_empty() {
        return None;
    }
    let Some(self_dependency) = resolved_cfg.and_then(|cfg| registry_self_dependency(cfg, "dart", str::to_string))
    else {
        return Some(anyhow::anyhow!(stale_dart_lock_message(&findings)));
    };

    let (pending, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|finding| {
        finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
    });

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed pubspec.lock pin(s) below require this crate's own version, which is not on \
             pub.dev yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_dart_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_dart_lock_message(&real)))
    }
}

/// Every `dependencies` / `dev_dependencies` constraint declared in
/// `pubspec_dir/pubspec.yaml` that the sibling `pubspec_dir/pubspec.lock` pins no version
/// satisfying.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_dart_lock_findings(pubspec_dir: &Path) -> Vec<StaleDartLockFinding> {
    let manifest_path = pubspec_dir.join("pubspec.yaml");
    let lock_path = pubspec_dir.join("pubspec.lock");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest_yaml) = serde_saphyr::from_str::<serde_json::Value>(&manifest_text) else {
        return Vec::new();
    };
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let Ok(lock_yaml) = serde_saphyr::from_str::<serde_json::Value>(&lock_text) else {
        return Vec::new();
    };
    let locked = locked_pubspec_versions(&lock_yaml);
    if locked.is_empty() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for bucket in DART_DEPENDENCY_BUCKETS {
        let Some(declared) = manifest_yaml.get(bucket).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, spec_value) in declared {
            // ~keep A mapping value (`{ path: ... }`, `{ git: ... }`, `{ hosted: ... }`) is a
            // source-pinned or explicitly-hosted dependency, not a plain version constraint --
            // the Dart analogue of the cargo check's own path/git exclusion. Only a bare scalar
            // string is a version constraint at all.
            let Some(requirement) = spec_value.as_str() else {
                continue;
            };
            // `any` is an explicit "no constraint" marker, never a real requirement to compare.
            if requirement == "any" {
                continue;
            }
            // ~keep One-sided, matching every other reader in this module family: a package
            // absent from the lock is never reported.
            let Some(locked_version) = locked.get(name.as_str()) else {
                continue;
            };
            match dart_constraint_matches(requirement, locked_version) {
                Some(true) => {}
                Some(false) => findings.push(StaleDartLockFinding {
                    lock: lock_path.clone(),
                    declared_in: manifest_path.clone(),
                    bucket,
                    dependency: name.clone(),
                    requirement: requirement.to_string(),
                    locked_version: locked_version.clone(),
                }),
                // A constraint this reader cannot confidently judge -- skip rather than risk a
                // false positive, matching the cargo/composer/ruby checks' unparseable skip.
                None => {}
            }
        }
    }
    findings.sort_by(|left, right| {
        left.bucket
            .cmp(right.bucket)
            .then_with(|| left.dependency.cmp(&right.dependency))
    });
    findings
}

/// `name -> version` for every package `pubspec.lock`'s `packages` map pins.
fn locked_pubspec_versions(lock: &serde_json::Value) -> BTreeMap<String, String> {
    let Some(packages) = lock.get("packages").and_then(serde_json::Value::as_object) else {
        return BTreeMap::new();
    };
    let mut locked = BTreeMap::new();
    for (name, entry) in packages {
        if let Some(version) = entry.get("version").and_then(serde_json::Value::as_str) {
            locked.insert(name.clone(), version.trim_matches('"').to_string());
        }
    }
    locked
}

/// Whether `locked` satisfies a single Dart pub version constraint clause, or `None` when the
/// constraint (or `locked` itself) is a form this reader does not confidently judge.
///
/// ~keep Deliberately narrow, matching the conservative one-sided philosophy the other readers in
/// this module family establish: judges a bare exact version, the four ordering comparison
/// operators (`>`, `>=`, `<`, `<=`), and a caret range (`^1.2.3`, pub's own "compatible release"
/// operator, semantically identical to Cargo's default caret including the leading-zero
/// narrowing). Returns `None` for a space-separated compound range (`">=1.2.3 <2.0.0"`, pub's
/// own AND syntax) and for `=` prefixed with anything else unrecognized -- alef's own generated
/// `pubspec.yaml` only ever writes a bare exact pin or a single passed-through operator (see
/// `render_pubspec`'s own doc comment), so a compound range only ever appears in a hand-edited
/// file this reader has no obligation to fully understand.
fn dart_constraint_matches(constraint: &str, locked: &str) -> Option<bool> {
    let trimmed = constraint.trim();
    if trimmed.is_empty() || trimmed.contains(' ') {
        return None;
    }
    let locked_version = semver::Version::parse(&pad_to_three(locked)).ok()?;
    if let Some(rest) = trimmed.strip_prefix('^') {
        let req = semver::VersionReq::parse(&format!("^{rest}")).ok()?;
        return Some(req.matches(&locked_version));
    }
    for op in ["<=", ">=", "<", ">"] {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let target = semver::Version::parse(&pad_to_three(rest.trim())).ok()?;
            return Some(match op {
                "<=" => locked_version <= target,
                ">=" => locked_version >= target,
                "<" => locked_version < target,
                ">" => locked_version > target,
                _ => unreachable!("op set above is exhaustive for this match"),
            });
        }
    }
    let target = semver::Version::parse(&pad_to_three(trimmed)).ok()?;
    Some(locked_version == target)
}

/// Pad a dot-separated numeric version out to exactly three leading components -- pub, like
/// Composer and RubyGems, does not require every version string to carry all three.
fn pad_to_three(raw: &str) -> String {
    let mut parts: Vec<&str> = raw.split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_dart_lock_message(findings: &[StaleDartLockFinding]) -> String {
    let mut message = format!(
        "{} committed pubspec.lock pin(s) cannot satisfy a requirement from a pubspec.yaml alef \
         generated; `dart pub get --offline` (and `dart pub get --enforce-lockfile`) will fail in these \
         directories:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` in {} ({}), but the lock pins {}. Fix with: dart pub \
             upgrade {} --directory {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.bucket,
            finding.locked_version,
            finding.dependency,
            finding.lock.parent().unwrap_or(Path::new(".")).display(),
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in pubspec.yaml -- a lockfile cannot record an exception \
         to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "dart_tests.rs"]
mod tests;
