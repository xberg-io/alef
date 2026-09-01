//! Fail a generation run whose generated `Gemfile` is vouched for beside a committed
//! `Gemfile.lock` that no longer resolves against it.
//!
//! Sibling of [`super::php`], not a shared abstraction with it -- RubyGems and Composer both
//! resolve a *range* against a *pinned version* (the cargo model, not node/uv's direct text
//! comparison), but their constraint syntaxes diverge (RubyGems has no `^`; its `~>` pessimistic
//! operator bumps a different component than Composer's `~` for some inputs), so each gets its
//! own small reader rather than a shared one two consumers would inevitably pull in different
//! directions. See [`ruby_constraint_matches`]'s doc for exactly which forms are judged.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::{registered_unmarkable_manifest_dirs, registry_self_dependency};

/// One `Gemfile` version constraint whose sibling `Gemfile.lock` pins no version that satisfies
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleGemfileLockFinding {
    /// The committed `Gemfile.lock` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `Gemfile` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Gem name as `Gemfile` spells it.
    pub(crate) dependency: String,
    /// The constraint text as written in the `Gemfile`'s `gem` call.
    pub(crate) requirement: String,
    /// The version `Gemfile.lock` pins for this gem in its `GEM specs:` block.
    pub(crate) locked_version: String,
}

/// Check every directory in which this run generated a `Gemfile` for a committed
/// `Gemfile.lock` that contradicts it, returning the failure to record when one does.
///
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_gemfile_lock_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
    check_generated_gemfile_lock_freshness_tolerating_pending_publish(generated_paths, base_dir, None)
}

/// The shared collection step behind [`check_generated_gemfile_lock_freshness`] and
/// [`check_generated_gemfile_lock_freshness_tolerating_pending_publish`].
///
/// `Gemfile` is `generated_header: false` (see `crate::e2e::codegen::ruby::E2eCodegen::generate`)
/// -- scaffolded once and thereafter user-owned, per the generated-vs-user-maintained boundary --
/// so it shares the exact structural blind spot [`super::node`]'s doc comment documents for
/// `package.json`, and is closed the same way: [`registered_unmarkable_manifest_dirs`] extends
/// `generated_paths` with every `Gemfile` directory the committed ownership record already knows
/// about.
fn collect_generated_gemfile_lock_findings(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Vec<StaleGemfileLockFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("Gemfile") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    directories.extend(registered_unmarkable_manifest_dirs(base_dir, "Gemfile"));
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_gemfile_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated Gemfile files against their committed Gemfile.lock"
    );
    findings
}

/// Same check as [`check_generated_gemfile_lock_freshness`], except a finding fully explained by
/// this crate's own pending, not-yet-published registry-mode `test_apps` self-dependency is
/// downgraded to a `tracing::warn!` instead of failing the stage -- the Ruby sibling of
/// [`super::cargo::check_generated_lock_freshness_tolerating_pending_publish`]. See
/// [`registry_self_dependency`]'s doc for what "explained" means here and why it is deliberately
/// conservative.
///
/// `normalize` mirrors `crate::e2e::codegen::ruby::project::render_gemfile`'s own rendering
/// exactly: an already-operator-prefixed version passes through unchanged (the explicit
/// `alef.toml` escape hatch), otherwise
/// [`crate::core::version::to_rubygems_prerelease`] converts cargo's dash-form prerelease
/// (`1.8.0-rc.2`) to the RubyGems-accepted dotted form (`1.8.0.pre.rc.2`) that `render_gemfile`
/// itself writes.
pub(crate) fn check_generated_gemfile_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_gemfile_lock_findings(generated_paths, base_dir);
    if findings.is_empty() {
        return None;
    }
    let Some(self_dependency) =
        resolved_cfg.and_then(|cfg| registry_self_dependency(cfg, "ruby", normalize_gemfile_requirement))
    else {
        return Some(anyhow::anyhow!(stale_gemfile_lock_message(&findings)));
    };

    let (pending, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|finding| {
        finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
    });

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed Gemfile.lock pin(s) below require this crate's own version, which is not on \
             the registry yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_gemfile_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_gemfile_lock_message(&real)))
    }
}

/// See [`check_generated_gemfile_lock_freshness_tolerating_pending_publish`]'s doc: mirrors
/// `render_gemfile`'s own registry-mode rendering exactly.
fn normalize_gemfile_requirement(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if trimmed.starts_with(['~', '>', '<', '=', '!']) {
        raw.to_string()
    } else {
        crate::core::version::to_rubygems_prerelease(raw)
    }
}

/// Every `gem` constraint declared in `gemfile_dir/Gemfile` that the sibling
/// `gemfile_dir/Gemfile.lock` pins no version satisfying.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_gemfile_lock_findings(gemfile_dir: &Path) -> Vec<StaleGemfileLockFinding> {
    let manifest_path = gemfile_dir.join("Gemfile");
    let lock_path = gemfile_dir.join("Gemfile.lock");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let locked = locked_gemfile_versions(&lock_text);
    if locked.is_empty() {
        return Vec::new();
    }

    let mut findings = Vec::new();
    for (name, requirement) in declared_gem_constraints(&manifest_text) {
        // ~keep One-sided, matching the cargo/composer readers' rule: a gem absent from the
        // lock's `specs:` block is never reported.
        let Some(locked_version) = locked.get(&name) else {
            continue;
        };
        match ruby_constraint_matches(&requirement, locked_version) {
            Some(true) => {}
            Some(false) => findings.push(StaleGemfileLockFinding {
                lock: lock_path.clone(),
                declared_in: manifest_path.clone(),
                dependency: name,
                requirement,
                locked_version: locked_version.to_string(),
            }),
            // A constraint this reader cannot confidently judge -- skip rather than risk a
            // false positive, matching the cargo/composer checks' unparseable-requirement skip.
            None => {}
        }
    }
    findings.sort_by(|left, right| left.dependency.cmp(&right.dependency));
    findings
}

/// `name -> requirement` for every plain, single-string `gem "name", "requirement"` call in a
/// `Gemfile` -- deliberately not a Ruby parser. Alef only ever generates one such line per
/// `Gemfile` (`render_gemfile`), so a line-oriented reader is sufficient; a `path:`/`git:`
/// dependency (no bare string requirement argument) and a bare `gem "name"` with no requirement
/// at all are both skipped, matching the cargo check's own path/git and no-requirement skips.
fn declared_gem_constraints(gemfile_text: &str) -> Vec<(String, String)> {
    let mut declared = Vec::new();
    for line in gemfile_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("gem ").or_else(|| trimmed.strip_prefix("gem(")) else {
            continue;
        };
        let quoted: Vec<&str> = rest.split(['\'', '"']).collect();
        // A quoted string alternates in at odd indices: `gem 'name', 'req'` splits on quote
        // chars into ["gem ", "name", ", ", "req", ...] once the "gem " prefix above is
        // stripped, so indices 1 and 3 are the two quoted tokens when both are present.
        let Some(name) = quoted.get(1) else { continue };
        let Some(requirement) = quoted.get(3) else { continue };
        // A third quoted token starting with a bare word followed by `:` (`path:`, `git:`) is
        // a keyword argument, not a version requirement -- skip it the same way the cargo
        // check skips a path/git dependency's `version` field.
        if rest.contains("path:") || rest.contains("git:") {
            continue;
        }
        declared.push((name.to_string(), requirement.to_string()));
    }
    declared
}

/// `name -> version` for every gem pinned in a `Gemfile.lock`'s `GEM`/`specs:` block.
///
/// Reads only the top-level pinned entries (four-space indent, `name (version)`), not their own
/// nested transitive dependencies (six-space indent) -- `Gemfile` only ever declares a
/// requirement on a top-level entry, so a transitive-only gem can never be looked up by
/// [`stale_gemfile_lock_findings`]'s one-sided rule anyway.
fn locked_gemfile_versions(lock_text: &str) -> BTreeMap<String, String> {
    let mut locked = BTreeMap::new();
    let mut in_gem_specs = false;
    for line in lock_text.lines() {
        if line == "GEM" {
            in_gem_specs = false;
            continue;
        }
        if !line.starts_with(' ') {
            // An unindented, non-empty line ends whichever top-level section was open.
            in_gem_specs = false;
            continue;
        }
        if line == "  specs:" {
            in_gem_specs = true;
            continue;
        }
        if !in_gem_specs {
            continue;
        }
        // Exactly four leading spaces: a top-level pinned entry. Six leading spaces is one of
        // that entry's own transitive dependencies, and is not read here.
        let Some(entry) = line.strip_prefix("    ").filter(|rest| !rest.starts_with(' ')) else {
            continue;
        };
        let Some((name, rest)) = entry.split_once(" (") else {
            continue;
        };
        let Some(version) = rest.strip_suffix(')') else {
            continue;
        };
        locked.insert(name.to_string(), version.to_string());
    }
    locked
}

/// Whether `locked` satisfies a single RubyGems version constraint clause, or `None` when the
/// constraint (or `locked` itself) is a form this reader does not confidently judge.
///
/// ~keep Deliberately narrow, matching the conservative one-sided philosophy the other readers in
/// this module family establish: judges a bare exact version (implicit `=`), the six comparison
/// operators (`=`, `!=`, `>`, `>=`, `<`, `<=`), and the pessimistic `~>` operator using RubyGems'
/// own semantics (truncate the last written component and bump the new last one -- `~> 2.2.3`
/// means `>=2.2.3,<2.3.0`, `~> 2.2` means `>=2.2.0,<3.0.0`). Returns `None` for a comma-separated
/// multi-clause requirement (`gem "x", ">= 1.0", "< 2.0"`, two separate call arguments Ruby AND-s
/// together) and for any version string this reader cannot parse as a plain `major.minor.patch`
/// (in particular a RubyGems dotted-prerelease pin like `1.8.0.pre.rc.2`, which
/// `to_rubygems_prerelease` produces and which is not valid semver) -- `render_gemfile` only ever
/// writes ONE requirement per `gem` call in practice, and a prerelease pin drifting is exactly the
/// kind of judgment call this reader declines to make rather than risk guessing wrong.
fn ruby_constraint_matches(constraint: &str, locked: &str) -> Option<bool> {
    let trimmed = constraint.trim();
    if trimmed.is_empty() || trimmed.contains(',') {
        return None;
    }
    let locked_version = semver::Version::parse(&pad_to_three(locked)).ok()?;
    if let Some(rest) = trimmed.strip_prefix("~>") {
        return Some(ruby_pessimistic_matches(rest.trim(), &locked_version));
    }
    for op in ["<=", ">=", "!=", "<", ">", "="] {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let target = semver::Version::parse(&pad_to_three(rest.trim())).ok()?;
            return Some(match op {
                "<=" => locked_version <= target,
                ">=" => locked_version >= target,
                "!=" => locked_version != target,
                "<" => locked_version < target,
                ">" => locked_version > target,
                "=" => locked_version == target,
                _ => unreachable!("op set above is exhaustive for this match"),
            });
        }
    }
    let target = semver::Version::parse(&pad_to_three(trimmed)).ok()?;
    Some(locked_version == target)
}

/// Pad a dot-separated numeric version out to exactly three leading components -- RubyGems, like
/// Composer, does not require every version string to carry all three (`~> 1.2` and `gem "x",
/// "1"` are both valid), but [`semver::Version::parse`] does.
fn pad_to_three(raw: &str) -> String {
    let mut parts: Vec<&str> = raw.split('.').collect();
    while parts.len() < 3 {
        parts.push("0");
    }
    parts.join(".")
}

/// See [`ruby_constraint_matches`]'s doc for RubyGems' pessimistic-operator semantics.
fn ruby_pessimistic_matches(rest: &str, locked: &semver::Version) -> bool {
    let written_components = rest.split('.').filter(|part| !part.is_empty()).count();
    let Ok(lower) = semver::Version::parse(&pad_to_three(rest)) else {
        return false;
    };
    if *locked < lower {
        return false;
    }
    let upper = if written_components >= 3 {
        semver::Version::new(lower.major, lower.minor + 1, 0)
    } else {
        semver::Version::new(lower.major + 1, 0, 0)
    };
    *locked < upper
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_gemfile_lock_message(findings: &[StaleGemfileLockFinding]) -> String {
    let mut message = format!(
        "{} committed Gemfile.lock pin(s) cannot satisfy a requirement from a Gemfile alef generated; \
         `bundle install --deployment` (and `bundle check`) will fail in these directories:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` by {}, but the lock pins {}. Fix with: bundle update {} \
             --gemfile {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.locked_version,
            finding.dependency,
            finding.declared_in.display(),
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in the Gemfile -- a lockfile cannot record an exception to \
         its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "ruby_tests.rs"]
mod tests;
