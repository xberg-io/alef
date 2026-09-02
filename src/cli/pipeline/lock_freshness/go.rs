//! Fail a generation run whose generated `go.mod` is vouched for beside a committed `go.sum`
//! that no longer records a checksum for a required module version.
//!
//! Go is the one ecosystem in this module family with no version-range concept at all: `require`
//! always pins one exact version, and `go.sum` is not a resolver's recorded specifier copy but a
//! flat checksum ledger keyed on `(module, version)`. So the comparison here is neither a semver
//! range walk (cargo/composer/ruby) nor a text-equality check against a recorded copy (node/uv):
//! it is "does the ledger have an entry for the exact pin `go.mod` requires", which fails `go
//! build`/`go test` under the default `-mod=readonly` with "missing go.sum entry" the moment it
//! does not.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::{registered_unmarkable_manifest_dirs, registry_self_dependency};

/// One `go.mod` `require` pin whose sibling `go.sum` records no checksum entry for that exact
/// version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleGoSumFinding {
    /// The committed `go.sum` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `go.mod` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Module path as `go.mod` spells it.
    pub(crate) dependency: String,
    /// The exact version `go.mod` requires.
    pub(crate) requirement: String,
    /// Every version of `dependency` the sum ledger does record, for the report.
    pub(crate) locked_versions: Vec<String>,
}

/// Check every directory in which this run generated a `go.mod` for a committed `go.sum` missing
/// an entry for a required pin, returning the failure to record when one does.
///
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_go_sum_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
    check_generated_go_sum_freshness_tolerating_pending_publish(generated_paths, base_dir, None)
}

/// The shared collection step behind [`check_generated_go_sum_freshness`] and
/// [`check_generated_go_sum_freshness_tolerating_pending_publish`].
///
/// `go.mod` is `generated_header: false` (see `crate::e2e::codegen::go::E2eCodegen::generate`) --
/// scaffolded once and thereafter user-owned -- so it shares the exact structural blind spot
/// [`super::node`]'s doc comment documents for `package.json`, and is closed the same way:
/// [`registered_unmarkable_manifest_dirs`] extends `generated_paths` with every `go.mod`
/// directory the committed ownership record already knows about.
fn collect_generated_go_sum_findings(generated_paths: &HashSet<PathBuf>, base_dir: &Path) -> Vec<StaleGoSumFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("go.mod") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    directories.extend(registered_unmarkable_manifest_dirs(base_dir, "go.mod"));
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_go_sum_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated go.mod files against their committed go.sum"
    );
    findings
}

/// Same check as [`check_generated_go_sum_freshness`], except a finding fully explained by this
/// crate's own pending, not-yet-published registry-mode `test_apps` self-dependency is downgraded
/// to a `tracing::warn!` instead of failing the stage -- the Go sibling of
/// [`super::cargo::check_generated_lock_freshness_tolerating_pending_publish`]. See
/// [`registry_self_dependency`]'s doc for what "explained" means here and why it is deliberately
/// conservative.
///
/// `normalize` prepends a `v` when the configured version omits one:
/// `crate::e2e::codegen::go::GoE2eCodegen::generate`'s own registry-mode `go_version` resolution
/// prefers `[crates.e2e.registry.packages.go].version` verbatim (assumed pre-formatted with `v`)
/// and only falls back to `format!("v{v}")` when that is unset, so this exemption can, in the
/// narrower case where only a base-level (non-registry) version is configured, disagree with
/// what the generator actually wrote -- the same conservatism [`registry_self_dependency`]'s own
/// doc already documents for every language it serves, not a new gap specific to Go.
pub(crate) fn check_generated_go_sum_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_go_sum_findings(generated_paths, base_dir);
    if findings.is_empty() {
        return None;
    }
    let Some(self_dependency) = resolved_cfg.and_then(|cfg| {
        registry_self_dependency(
            cfg,
            "go",
            |package| package.name.clone().or_else(|| package.module.clone()),
            normalize_go_version,
        )
    }) else {
        return Some(anyhow::anyhow!(stale_go_sum_message(&findings)));
    };

    let (pending, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|finding| {
        finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
    });

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed go.sum entr(y/ies) below are missing for this crate's own version, which is not \
             on the module proxy yet -- expected after a version bump; resolves once the release \
             publishes:\n{}",
            pending.len(),
            stale_go_sum_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_go_sum_message(&real)))
    }
}

/// See [`check_generated_go_sum_freshness_tolerating_pending_publish`]'s doc.
fn normalize_go_version(raw: &str) -> String {
    if raw.starts_with('v') {
        raw.to_string()
    } else {
        format!("v{raw}")
    }
}

/// Every `require` pin declared in `go_mod_dir/go.mod` (that is not itself covered by a
/// `replace` directive) that the sibling `go_mod_dir/go.sum` records no checksum entry for.
///
/// Returns empty when either file is missing: alef never authors `go.sum`, so a directory
/// without one is a deliberate consumer choice (or a run before the first `go mod download`),
/// not a defect to report.
pub(crate) fn stale_go_sum_findings(go_mod_dir: &Path) -> Vec<StaleGoSumFinding> {
    let manifest_path = go_mod_dir.join("go.mod");
    let lock_path = go_mod_dir.join("go.sum");
    let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let (requires, replaced) = parse_go_mod_requires(&manifest_text);
    let locked = locked_go_sum_versions(&lock_text);
    if locked.is_empty() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for (module, version) in requires {
        // ~keep A `replace` directive resolves the module locally (alef's own e2e generator
        // only ever emits a local-path replace, never a registry-to-registry one), so it is
        // never expected to have a `go.sum` checksum entry at all -- the Go analogue of the
        // cargo check's own `is_source_pinned` path/git exclusion.
        if replaced.contains(&module) {
            continue;
        }
        // ~keep One-sided, matching every other reader in this module family: a module absent
        // from `go.sum` entirely is never reported. A brand new `require` before the first `go
        // mod download`/`go mod tidy` looks identical to a reader that failed to find it, and
        // Go's own module graph pruning (go.dev/ref/mod#graph-pruning) can legitimately omit an
        // indirect dependency's checksum in some configurations -- only a module the ledger DOES
        // track, at a version it does NOT, is unambiguous evidence `go build -mod=readonly` will
        // reject.
        let Some(versions) = locked.get(&module) else {
            continue;
        };
        if versions.contains(&version) {
            continue;
        }
        findings.push(StaleGoSumFinding {
            lock: lock_path.clone(),
            declared_in: manifest_path.clone(),
            dependency: module,
            requirement: version,
            locked_versions: versions.iter().cloned().collect(),
        });
    }
    findings.sort_by(|left, right| left.dependency.cmp(&right.dependency));
    findings
}

/// `(module -> required version)` pairs from every `require` line or block, paired with the set
/// of module paths a `replace` directive covers.
///
/// Reads both go.mod `require` shapes: the single-line form (`require module version`) and the
/// parenthesized block form (`require (\n\tmodule version\n)`) alef's own
/// `crate::e2e::codegen::go::render_go_mod` writes for both the direct dependency and its pinned
/// `// indirect` transitive block. A trailing `// indirect` comment is stripped before reading
/// the version field.
fn parse_go_mod_requires(go_mod_text: &str) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut requires = BTreeMap::new();
    let mut replaced = BTreeSet::new();
    let mut in_require_block = false;
    for raw_line in go_mod_text.lines() {
        let line = raw_line.split("//").next().unwrap_or(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if in_require_block {
            if line == ")" {
                in_require_block = false;
                continue;
            }
            if let Some((module, version)) = parse_module_version_pair(line) {
                requires.insert(module, version);
            }
            continue;
        }
        if line == "require (" {
            in_require_block = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            if let Some((module, version)) = parse_module_version_pair(rest) {
                requires.insert(module, version);
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ")
            && let Some((module, _)) = rest.split_once("=>")
        {
            replaced.insert(module.trim().to_string());
        }
    }
    (requires, replaced)
}

/// Split a `module version` pair on the first run of whitespace.
fn parse_module_version_pair(text: &str) -> Option<(String, String)> {
    let mut parts = text.split_whitespace();
    let module = parts.next()?;
    let version = parts.next()?;
    Some((module.to_string(), version.to_string()))
}

/// `module -> every version the sum ledger records a checksum for` (either the module content
/// hash or its `/go.mod` hash line -- either is sufficient evidence the module at that exact
/// version was, at some point, resolved and recorded, which is all this presence check needs).
fn locked_go_sum_versions(go_sum_text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut locked: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in go_sum_text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(module), Some(version_field)) = (fields.next(), fields.next()) else {
            continue;
        };
        let version = version_field.strip_suffix("/go.mod").unwrap_or(version_field);
        locked
            .entry(module.to_string())
            .or_default()
            .insert(version.to_string());
    }
    locked
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_go_sum_message(findings: &[StaleGoSumFinding]) -> String {
    let mut message = format!(
        "{} committed go.sum ledger entr(y/ies) missing a checksum for a require pin in a go.mod alef \
         generated; `go build -mod=readonly` / `go test -mod=readonly` (the CI default) will fail with \
         \"missing go.sum entry\":",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required at `{}` by {}, but the ledger records only {}. Fix with: cd {} \
             && go mod download {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.locked_versions.join(", "),
            finding.lock.parent().unwrap_or(Path::new(".")).display(),
            finding.dependency,
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in go.mod -- a checksum ledger cannot record an exception \
         to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "go_tests.rs"]
mod tests;
