//! Fail a generation run whose generated `composer.json` is vouched for beside a committed
//! `composer.lock` that no longer resolves against it.
//!
//! Sibling of [`super::node`] and [`super::uv`], not a shared abstraction with either -- see the
//! node module's own doc comment for why forcing ecosystem-specific lock reading through one
//! function is how a later change to one silently drifts another. Composer, like Cargo, resolves
//! a *range* against a *pinned version* (unlike node/uv's direct text comparison against a
//! recorded specifier copy), so this reader implements just enough of Composer's constraint
//! syntax (exact, `^`, `~`, and the six comparison operators, one clause at a time) to judge the
//! forms alef's own generated `composer.json` and its own `require-dev` pins (`phpunit/phpunit`,
//! `guzzlehttp/guzzle`) actually use -- see [`composer_constraint_matches`]'s doc for exactly
//! which forms are judged and which are conservatively skipped.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::{registered_unmarkable_manifest_dirs, registry_self_dependency};

/// Dependency buckets a `composer.json` can declare a version-constrained package in.
const COMPOSER_DEPENDENCY_BUCKETS: [&str; 2] = ["require", "require-dev"];

/// One `composer.json` version constraint whose sibling `composer.lock` pins no version that
/// satisfies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleComposerLockFinding {
    /// The committed `composer.lock` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `composer.json` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Which dependency table the requirement was declared in.
    pub(crate) bucket: &'static str,
    /// Package name as `composer.json` spells it (`vendor/package`).
    pub(crate) dependency: String,
    /// The constraint text as written in `composer.json`.
    pub(crate) requirement: String,
    /// Every version of `dependency` the lock does pin, for the report.
    pub(crate) locked_versions: Vec<String>,
}

/// Check every directory in which this run generated a `composer.json` for a committed
/// `composer.lock` that contradicts it, returning the failure to record when one does.
///
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_composer_lock_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
    check_generated_composer_lock_freshness_tolerating_pending_publish(generated_paths, base_dir, None)
}

/// The shared collection step behind [`check_generated_composer_lock_freshness`] and
/// [`check_generated_composer_lock_freshness_tolerating_pending_publish`].
///
/// `composer.json` is `generated_header: false` (see
/// `crate::e2e::codegen::php::E2eCodegen::generate`) -- JSON has no comment syntax to carry an
/// `alef:hash:` marker -- so it shares the exact structural blind spot
/// [`super::node::collect_generated_node_lock_findings`]'s doc comment documents for
/// `package.json`, and is closed the same way: [`registered_unmarkable_manifest_dirs`] extends
/// `generated_paths` with every `composer.json` directory the committed ownership record already
/// knows about.
fn collect_generated_composer_lock_findings(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Vec<StaleComposerLockFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("composer.json") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    directories.extend(registered_unmarkable_manifest_dirs(base_dir, "composer.json"));
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_composer_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated composer.json files against their committed composer.lock"
    );
    findings
}

/// Same check as [`check_generated_composer_lock_freshness`], except a finding fully explained by
/// this crate's own pending, not-yet-published registry-mode `test_apps` self-dependency is
/// downgraded to a `tracing::warn!` instead of failing the stage -- the PHP sibling of
/// [`super::cargo::check_generated_lock_freshness_tolerating_pending_publish`]. See
/// [`registry_self_dependency`]'s doc for what "explained" means here and why it is deliberately
/// conservative.
///
/// ~keep In practice this exemption structurally never engages for alef's own generated
/// `test_apps/php/composer.json`: that manifest never declares the crate under test as a
/// Composer requirement at all (`crate::e2e::codegen::php::project::render_composer_json`'s
/// `Registry`-mode doc comment explains why -- the native extension is installed by PIE, and
/// `ext-<name>` is deliberately left out of `require` so Composer's platform resolver does not
/// go looking for it before install.sh has loaded it). The wrapper still exists, with the same
/// signature as its cargo/node/uv siblings, so every call site can treat all four/five
/// ecosystems uniformly rather than special-casing PHP as the one gate with no tolerating
/// variant; a hand-written `composer.json` that DOES declare a matching self-dependency would
/// still be exempted correctly, since nothing here assumes alef itself wrote that line.
pub(crate) fn check_generated_composer_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_composer_lock_findings(generated_paths, base_dir);
    if findings.is_empty() {
        return None;
    }
    let Some(self_dependency) = resolved_cfg.and_then(|cfg| registry_self_dependency(cfg, "php", str::to_string))
    else {
        return Some(anyhow::anyhow!(stale_composer_lock_message(&findings)));
    };

    let (pending, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|finding| {
        finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
    });

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed composer.lock pin(s) below require this crate's own version, which is not on \
             the registry yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_composer_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_composer_lock_message(&real)))
    }
}

/// Every `require` / `require-dev` constraint declared in `composer_dir/composer.json` that the
/// sibling `composer_dir/composer.lock` pins no version satisfying.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_composer_lock_findings(composer_dir: &Path) -> Vec<StaleComposerLockFinding> {
    let manifest_path = composer_dir.join("composer.json");
    let lock_path = composer_dir.join("composer.lock");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest_json) = serde_json::from_str::<serde_json::Value>(&manifest_text) else {
        return Vec::new();
    };
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&lock_text) else {
        return Vec::new();
    };
    let locked = locked_composer_versions(&lock_json);
    if locked.is_empty() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for bucket in COMPOSER_DEPENDENCY_BUCKETS {
        let Some(declared) = manifest_json.get(bucket).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, spec_value) in declared {
            let Some(requirement) = spec_value.as_str() else {
                continue;
            };
            // ~keep One-sided, matching `stale_lock_findings`'s cargo rule: a name absent from
            // the lock is never reported (composer platform pseudo-packages like `php` and
            // `ext-*` are never in `composer.lock` at all and fall out here naturally, with no
            // special-case skip needed).
            let Some(locked_versions) = locked.get(name.as_str()) else {
                continue;
            };
            let Some(all_satisfied) = locked_versions
                .iter()
                .map(|version| composer_constraint_matches(requirement, version))
                .collect::<Option<Vec<bool>>>()
            else {
                // A constraint or a locked version this reader cannot confidently judge --
                // skip rather than risk a false positive, matching the cargo check's
                // unparseable-requirement skip.
                continue;
            };
            if all_satisfied.iter().any(|matches| *matches) {
                continue;
            }
            findings.push(StaleComposerLockFinding {
                lock: lock_path.clone(),
                declared_in: manifest_path.clone(),
                bucket,
                dependency: name.clone(),
                requirement: requirement.to_string(),
                locked_versions: locked_versions.iter().map(ToString::to_string).collect(),
            });
        }
    }
    findings.sort_by(|left, right| {
        left.bucket
            .cmp(right.bucket)
            .then_with(|| left.dependency.cmp(&right.dependency))
    });
    findings
}

/// `name -> every version pinned for it` from a `composer.lock`'s `packages` and `packages-dev`
/// arrays.
fn locked_composer_versions(lock: &serde_json::Value) -> BTreeMap<String, Vec<semver::Version>> {
    let mut locked: BTreeMap<String, Vec<semver::Version>> = BTreeMap::new();
    for bucket in ["packages", "packages-dev"] {
        let Some(entries) = lock.get(bucket).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for entry in entries {
            let (Some(name), Some(version)) = (
                entry.get("name").and_then(serde_json::Value::as_str),
                entry.get("version").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            if let Some(parsed) = parse_composer_version(version) {
                locked.entry(name.to_string()).or_default().push(parsed);
            }
        }
    }
    for versions in locked.values_mut() {
        versions.sort();
    }
    locked
}

/// Parse a Composer-style version string (`v1.2.3`, `1.2`, `1`) into a full `major.minor.patch`
/// [`semver::Version`], padding any missing trailing components with `0` -- Composer, unlike
/// Cargo, does not require every version string to carry all three components.
fn parse_composer_version(raw: &str) -> Option<semver::Version> {
    semver::Version::parse(&pad_to_three(raw.strip_prefix(['v', 'V']).unwrap_or(raw))).ok()
}

/// Pad a dot-separated numeric version (with an optional `-prerelease`/`+build` suffix) out to
/// exactly three leading components, leaving any suffix untouched.
fn pad_to_three(raw: &str) -> String {
    let split_at = raw.find(['-', '+']).unwrap_or(raw.len());
    let (numeric, suffix) = raw.split_at(split_at);
    let mut parts: Vec<&str> = numeric.split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    format!("{}{suffix}", parts.join("."))
}

/// Whether `version` satisfies a single Composer version constraint clause, or `None` when the
/// constraint (or `version` itself) is a form this reader does not confidently judge.
///
/// ~keep Deliberately narrow, matching the conservative one-sided philosophy the cargo/node/uv
/// readers in this module family already establish: judges an exact version (`1.2.3`, optionally
/// `v`-prefixed), a caret range (`^1.2.3`, identical semantics to Cargo's own default caret --
/// both narrow scope for a leading-zero major exactly the same way), a tilde range (`~1.2.3` /
/// `~1.2`, using COMPOSER's own tilde semantics, which differ from Cargo's for the two-component
/// form: `~1.2` means `>=1.2.0,<2.0.0` in Composer but `>=1.2.0,<1.3.0` in Cargo), and the six
/// comparison operators against a single version. Returns `None` (skip, never a false positive)
/// for anything wider than one clause -- comma-separated AND lists, `||` OR lists, hyphen ranges,
/// and `*` wildcards -- since alef's own generated constraints (an exact self-dependency pin, or
/// a single `^`/plain version for a `require-dev` tool) never need them, and guessing at OR/AND
/// composition risks reporting a healthy tree red.
fn composer_constraint_matches(constraint: &str, version: &semver::Version) -> Option<bool> {
    let trimmed = constraint.trim();
    if trimmed.is_empty() || trimmed.contains("||") || trimmed.contains(',') || trimmed.contains(' ') {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('^') {
        let req = semver::VersionReq::parse(&format!("^{}", pad_to_three(rest))).ok()?;
        return Some(req.matches(version));
    }
    if let Some(rest) = trimmed.strip_prefix('~') {
        return Some(composer_tilde_matches(rest, version));
    }
    for op in ["<=", ">=", "!=", "<", ">", "="] {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let target = parse_composer_version(rest.trim())?;
            return Some(match op {
                "<=" => *version <= target,
                ">=" => *version >= target,
                "!=" => *version != target,
                "<" => *version < target,
                ">" => *version > target,
                "=" => *version == target,
                _ => unreachable!("op set above is exhaustive for this match"),
            });
        }
    }
    let target = parse_composer_version(trimmed)?;
    Some(*version == target)
}

/// Composer's own tilde semantics: `~1.2.3` bumps the last written component
/// (`>=1.2.3,<1.3.0`), but `~1.2` (only two components written) bumps the FIRST instead
/// (`>=1.2.0,<2.0.0`) -- the one place Composer's tilde disagrees with Cargo's, which bumps the
/// minor either way. See [`composer_constraint_matches`]'s doc for why this needs its own
/// implementation rather than delegating to `semver::VersionReq`'s built-in tilde operator.
fn composer_tilde_matches(rest: &str, version: &semver::Version) -> bool {
    let written_components = rest.split('.').filter(|part| !part.is_empty()).count();
    let Some(lower) = parse_composer_version(rest) else {
        return false;
    };
    if *version < lower {
        return false;
    }
    let upper = if written_components >= 3 {
        semver::Version::new(lower.major, lower.minor + 1, 0)
    } else {
        semver::Version::new(lower.major + 1, 0, 0)
    };
    *version < upper
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_composer_lock_message(findings: &[StaleComposerLockFinding]) -> String {
    let mut message = format!(
        "{} committed composer.lock pin(s) cannot satisfy a requirement from a composer.json alef \
         generated; `composer install` (and `composer validate --strict` in CI) will fail in these \
         directories:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` in {} ({}), but the lock pins only {}. Fix with: composer \
             update {} --working-dir {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.bucket,
            finding.locked_versions.join(", "),
            finding.dependency,
            finding.lock.parent().unwrap_or(Path::new(".")).display(),
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in composer.json -- a lockfile cannot record an exception \
         to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "php_tests.rs"]
mod tests;
