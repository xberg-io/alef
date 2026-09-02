//! Fail a generation run whose generated `pyproject.toml` is vouched for beside a committed
//! `uv.lock` that no longer records the same specifiers.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::registry_self_dependency;

/// One `pyproject.toml` `[project.dependencies]` entry whose sibling `uv.lock` records a different
/// specifier for the same package.
///
/// Unlike [`crate::cli::pipeline::lock_freshness::cargo::StaleLockFinding`] there is no
/// path-dependency walk (the requirement text and the generated manifest are one file, same as the
/// node check), and unlike
/// [`crate::cli::pipeline::lock_freshness::node::StaleNodeLockFinding`] the comparison is not
/// against another copy of the manifest's own text -- it is against a second, lock-recorded copy
/// of that same text. `uv.lock` carries a `[package.metadata] requires-dist` entry for the
/// project's own lock-file package that is populated verbatim from `pyproject.toml` at lock time
/// (confirmed against astral-sh/uv#17549, which is exactly a report of that recorded copy going
/// stale relative to the manifest). That is uv's own invalidation mechanism: `uv sync --locked` /
/// `uv lock --check` fail with "The lockfile ... needs to be updated" when the recorded copy no
/// longer matches, regardless of whether the currently locked *version* still happens to satisfy
/// the manifest's range. That last clause is why this check uses text equality (the node model)
/// rather than semver-range satisfaction (the cargo model): an open lower bound like
/// `pyrefly>=1.1.1` is satisfied by a lock still pinning `1.1.1` a month later even though uv
/// itself would call that lock stale the moment anything forces a re-resolve -- a range check
/// would stay silent on exactly the drift this exists to catch, while a text check catches it
/// whenever the recorded copy itself has moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleUvLockFinding {
    /// The committed `uv.lock` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `pyproject.toml` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Package name as `pyproject.toml` spells it.
    pub(crate) dependency: String,
    /// The specifier text as written in `pyproject.toml` (empty when the dependency is
    /// unconstrained).
    pub(crate) requirement: String,
    /// The specifier text `uv.lock` records for the same name.
    pub(crate) locked_requirement: String,
}

/// Check every directory in which this run generated a `pyproject.toml` for a committed `uv.lock`
/// whose recorded specifiers disagree with it, returning the failure to record when one does.
///
/// A third sibling beside
/// [`crate::cli::pipeline::lock_freshness::cargo::check_generated_lock_freshness`] and
/// [`crate::cli::pipeline::lock_freshness::node::check_generated_node_lock_freshness`], not a
/// shared abstraction -- see the node check's doc comment for why forcing ecosystem-specific lock
/// reading through one function is how a later change to one silently drifts another.
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_uv_lock_freshness(generated_paths: &HashSet<PathBuf>) -> Option<anyhow::Error> {
    check_generated_uv_lock_freshness_tolerating_pending_publish(generated_paths, None)
}

/// The shared collection step behind [`check_generated_uv_lock_freshness`] and
/// [`check_generated_uv_lock_freshness_tolerating_pending_publish`].
fn collect_generated_uv_lock_findings(generated_paths: &HashSet<PathBuf>) -> Vec<StaleUvLockFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("pyproject.toml") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_uv_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated pyproject.toml files against their committed uv.lock"
    );
    findings
}

/// Same check as [`check_generated_uv_lock_freshness`], except a finding fully explained by this
/// crate's own pending, not-yet-published registry-mode `test_apps` self-dependency is downgraded
/// to a `tracing::warn!` instead of failing the stage -- the uv sibling of
/// [`crate::cli::pipeline::lock_freshness::cargo::check_generated_lock_freshness_tolerating_pending_publish`].
/// See [`registry_self_dependency`]'s doc for what "explained" means here and why it is deliberately
/// conservative, and [`crate::e2e::codegen::python::config::normalize_python_version`] for why the
/// requirement text needs PEP 508 normalization before comparison (the html-to-markdown incident
/// this closes: `test_apps/python/pyproject.toml` requires `html-to-markdown>=3.12.0` while PyPI
/// still only has `3.11.6` published).
pub(crate) fn check_generated_uv_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_uv_lock_findings(generated_paths);
    if findings.is_empty() {
        return None;
    }
    let Some(self_dependency) = resolved_cfg.and_then(|cfg| {
        registry_self_dependency(
            cfg,
            "python",
            |package| package.name.clone(),
            crate::e2e::codegen::python::config::normalize_python_version,
        )
    }) else {
        return Some(anyhow::anyhow!(stale_uv_lock_message(&findings)));
    };

    let (pending, real): (Vec<_>, Vec<_>) = findings.into_iter().partition(|finding| {
        finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
    });

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed uv.lock pin(s) below require this crate's own version, which is not on the \
             registry yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_uv_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_uv_lock_message(&real)))
    }
}

/// Every `[project.dependencies]` specifier declared in `pyproject_dir/pyproject.toml` that the
/// sibling `pyproject_dir/uv.lock` records a different specifier for.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_uv_lock_findings(pyproject_dir: &Path) -> Vec<StaleUvLockFinding> {
    let manifest_path = pyproject_dir.join("pyproject.toml");
    let lock_path = pyproject_dir.join("uv.lock");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let Ok(manifest_text) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest_toml) = toml::from_str::<toml::Value>(&manifest_text) else {
        return Vec::new();
    };
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let Ok(lock_toml) = toml::from_str::<toml::Value>(&lock_text) else {
        return Vec::new();
    };
    let Some(project) = manifest_toml.get("project") else {
        return Vec::new();
    };
    let Some(project_name) = project.get("name").and_then(toml::Value::as_str) else {
        return Vec::new();
    };
    let Some(dependencies) = project.get("dependencies").and_then(toml::Value::as_array) else {
        return Vec::new();
    };
    let locked = locked_uv_requirements(&lock_toml, project_name);
    if locked.is_empty() {
        return Vec::new();
    }
    // ~keep A name present in `[tool.uv.sources]` has its resolution overridden -- path, git, URL,
    // workspace member, or an explicit alternate index -- exactly the forms `parse_pep508_requirement`
    // below cannot see from the dependency string alone, since `render_pyproject`'s `Local` mode
    // writes the bare unconstrained name here and puts the actual source in this table. Comparing
    // any of these against the lock's registry-shaped specifier text would be a false positive.
    let overridden = uv_source_override_names(&manifest_toml);

    let mut findings = Vec::new();
    for entry in dependencies {
        let Some(raw) = entry.as_str() else { continue };
        let Some((name, requirement)) = parse_pep508_requirement(raw) else {
            continue;
        };
        let normalized = normalize_pep503_name(&name);
        if overridden.contains(&normalized) {
            continue;
        }
        // ~keep One-sided, matching the rule in `stale_lock_findings` and `stale_node_lock_findings`:
        // a name the lock's recorded copy never mentions is never reported. Here that also covers a
        // dependency `requires_dist_map` deliberately dropped for carrying a `marker`/`extra` key --
        // both mean this reader could not derive a single unconditional specifier for the name, which
        // is a reason to stay silent, not a reason to guess.
        let Some(locked_requirement) = locked.get(&normalized) else {
            continue;
        };
        if locked_requirement.trim() == requirement.trim() {
            continue;
        }
        findings.push(StaleUvLockFinding {
            lock: lock_path.clone(),
            declared_in: manifest_path.clone(),
            dependency: name,
            requirement,
            locked_requirement: locked_requirement.clone(),
        });
    }
    findings.sort_by(|left, right| left.dependency.cmp(&right.dependency));
    findings
}

/// `name -> specifier text` `uv.lock` recorded for the project's own dependencies.
///
/// Tries the project-lock shape first: the `[[package]]` entry whose (PEP 503 normalized) `name`
/// matches `pyproject.toml`'s own `[project.name]`, reading its `[package.metadata] requires-dist`.
/// Falls back to the standalone-script-lock shape's top-level `[manifest] requirements` -- the same
/// `{ name, specifier }` entries, just not attached to a per-package `[[package]]` table, because a
/// PEP 723 script lock has no installable project to attach metadata to. Alef only ever generates a
/// `pyproject.toml` with a real `[project]` table, so in practice the first shape is what fires; the
/// fallback exists so a `uv.lock` this reader does not fully control is read safely rather than
/// assumed. Either shape returning nothing yields an empty map, which is safe by construction:
/// [`stale_uv_lock_findings`]'s one-sided rule treats "absent from this map" identically to "name
/// not in the lock", never as a contradiction. ~keep
fn locked_uv_requirements(lock: &toml::Value, project_name: &str) -> BTreeMap<String, String> {
    let normalized_project = normalize_pep503_name(project_name);
    let root_requires_dist = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .and_then(|packages| {
            packages.iter().find(|package| {
                package
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .is_some_and(|name| normalize_pep503_name(name) == normalized_project)
            })
        })
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("requires-dist"))
        .and_then(toml::Value::as_array);
    if let Some(entries) = root_requires_dist {
        let map = requires_dist_map(entries);
        if !map.is_empty() {
            return map;
        }
    }
    lock.get("manifest")
        .and_then(|manifest| manifest.get("requirements"))
        .and_then(toml::Value::as_array)
        .map(|entries| requires_dist_map(entries))
        .unwrap_or_default()
}

/// `name -> specifier text` out of one `requires-dist` / `[manifest] requirements` array.
///
/// Skips any entry carrying a `marker` or `extra` key: those are conditionally-applicable copies
/// (a platform-gated dependency, or an optional-dependency-group member) that do not correspond
/// 1:1 with an unconditional `[project.dependencies]` entry, so including them risks comparing the
/// wrong recorded copy against the manifest.
fn requires_dist_map(entries: &[toml::Value]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for entry in entries {
        let Some(table) = entry.as_table() else { continue };
        if table.contains_key("marker") || table.contains_key("extra") {
            continue;
        }
        let Some(name) = table.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let specifier = table.get("specifier").and_then(toml::Value::as_str).unwrap_or("");
        map.insert(normalize_pep503_name(name), specifier.to_string());
    }
    map
}

/// Names declared in `[tool.uv.sources]`, PEP 503 normalized.
///
/// Every entry in this table overrides where uv resolves that name from -- see the `~keep` comment
/// at its call site in `stale_uv_lock_findings`.
fn uv_source_override_names(manifest_toml: &toml::Value) -> HashSet<String> {
    manifest_toml
        .get("tool")
        .and_then(|tool| tool.get("uv"))
        .and_then(|uv| uv.get("sources"))
        .and_then(toml::Value::as_table)
        .map(|table| table.keys().map(|name| normalize_pep503_name(name)).collect())
        .unwrap_or_default()
}

/// Split a PEP 508 dependency string into `(name, specifier)`, or `None` when the form is not one
/// this reader can safely compare.
///
/// ~keep Excluded: an environment marker (`;`) makes the requirement conditional on something this
/// reader does not evaluate; a direct reference (`@`, a URL or local path) is pinned by content, not
/// by a registry range; extras (`[...]`) change what the name resolves to without changing the name
/// text itself, and this reader does not need to parse them correctly to know it should not compare
/// them. A form outside all three is a bare `name` optionally followed by a version specifier, which
/// is exactly what `requires_dist_map` also expects.
fn parse_pep508_requirement(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(';') || trimmed.contains('@') || trimmed.contains('[') {
        return None;
    }
    let name_len = trimmed
        .find(|character: char| !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')))
        .unwrap_or(trimmed.len());
    if name_len == 0 {
        return None;
    }
    let (name, specifier) = trimmed.split_at(name_len);
    Some((name.to_string(), specifier.trim().to_string()))
}

/// PEP 503 name normalization: lowercase, with every run of `-`, `_`, `.` collapsed to one `-`.
/// `pyproject.toml` and `uv.lock` are not guaranteed to spell the same package identically (`uv`
/// itself accepts `pytest_asyncio` and `pytest-asyncio` as the same dependency), so every name
/// compared or used as a map key in this module goes through this first.
fn normalize_pep503_name(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    let mut previous_was_separator = false;
    for character in name.chars() {
        if matches!(character, '-' | '_' | '.') {
            if !previous_was_separator {
                normalized.push('-');
            }
            previous_was_separator = true;
        } else {
            normalized.push(character.to_ascii_lowercase());
            previous_was_separator = false;
        }
    }
    normalized
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_uv_lock_message(findings: &[StaleUvLockFinding]) -> String {
    let mut message = format!(
        "{} committed uv.lock specifier(s) disagree with a pyproject.toml alef generated; `uv sync \
         --locked` (and frozen-lockfile CI jobs) will fail with \"The lockfile at `uv.lock` needs to be \
         updated\":",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` by {}, but the lock records `{}`. Fix with: uv lock \
             --project {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.locked_requirement,
            finding.lock.parent().unwrap_or(Path::new(".")).display(),
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in pyproject.toml -- a lockfile cannot record an exception \
         to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "uv_tests.rs"]
mod tests;
