//! Fail a generation run whose generated `package.json` is vouched for beside a committed
//! `pnpm-lock.yaml` that no longer records the same specifiers.
//!
//! Sibling of [`crate::cli::pipeline::lock_freshness::cargo`], not a shared abstraction with it —
//! see this module's own doc comments below for why forcing ecosystem-specific lock reading
//! through one function is how a later change to one silently drifts another.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use super::shared::{RegistrySelfDependency, registered_unmarkable_manifest_dirs, registry_self_dependency};

/// Dependency buckets alef itself writes into a generated `package.json` -- see
/// `crate::e2e::codegen::typescript::config::render_package_json` (and its wasm counterpart),
/// which only ever populate `dependencies` / `devDependencies`. Checking a bucket alef never
/// writes would find nothing but a hand-authored drift this module has no business reporting.
const NODE_DEPENDENCY_BUCKETS: [&str; 2] = ["dependencies", "devDependencies"];

/// Every `[crates.e2e...packages.<lang>]` key whose e2e codegen can emit a `package.json` --
/// see `crate::e2e::codegen::typescript::config::render_package_json` (`"node"`) and its wasm
/// counterpart in `crate::e2e::codegen::wasm` (`"wasm"`). Both land in their own `test_apps/*`
/// directory with their own `pnpm-lock.yaml`, so [`collect_generated_node_lock_findings`] walks
/// both, and this check's pending-publish exemption must resolve a self-dependency identity for
/// both too -- see the doc comment on
/// [`check_generated_node_lock_freshness_tolerating_pending_publish`] for the incident this
/// closes. Not derived by pattern-matching a `-wasm` name suffix on purpose: that would be a
/// second, independent guess at the same fact `registry_self_dependency` already computes
/// authoritatively from `[crates.e2e.registry.packages.<lang>]`. Extending this list is the only
/// change needed the day a third lang starts emitting `package.json` into this ecosystem.
const PACKAGE_JSON_EMITTING_LANGS: [&str; 2] = ["node", "wasm"];

/// One `package.json` specifier whose sibling `pnpm-lock.yaml` records a different specifier for
/// the same dependency name and bucket.
///
/// Unlike [`crate::cli::pipeline::lock_freshness::cargo::StaleLockFinding`], there is no
/// path-dependency walk: the requirement text and the generated manifest are the same file, so the
/// comparison is direct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleNodeLockFinding {
    /// The committed `pnpm-lock.yaml` that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The `package.json` alef generated the requirement in.
    pub(crate) declared_in: PathBuf,
    /// Which dependency table the requirement was declared in.
    pub(crate) bucket: &'static str,
    /// Package name as `package.json` spells it.
    pub(crate) dependency: String,
    /// The specifier text as written in `package.json`.
    pub(crate) requirement: String,
    /// The specifier text the lock records for the same name and bucket.
    pub(crate) locked_requirement: String,
}

/// Check every directory in which this run generated a `package.json` for a committed
/// `pnpm-lock.yaml` whose recorded specifiers disagree with it, returning the failure to record
/// when one does.
///
/// Mirrors [`crate::cli::pipeline::lock_freshness::cargo::check_generated_lock_freshness`] one
/// call away in the same module family rather than sharing machinery with it: the cargo check's
/// job is a transitive path-dependency walk followed by semver resolution against a lock it must
/// reconstruct, while this check's job is a direct text comparison against a manifest alef already
/// has in hand. The two are different problems wearing similar names, and forcing them through one
/// abstraction is how a later change to either drifts the other -- see the `avoid-duplication`
/// rule and the `two-generators-disagree` pattern this repo is watching for. ~keep
///
/// `generated_paths` alone is structurally blind to a whole class of `package.json`: any emitted
/// `generated_header: false` (`crates/*-wasm/package.json`, `crates/*-node/package.json` and its
/// per-platform `npm/<platform>/package.json` siblings, ...) never carries an `alef:hash:` marker
/// -- JSON has no comment syntax to hold one -- so [`GeneratedFile::carries_alef_marker`] is
/// always `false` for it and [`crate::cli::pipeline::generate::stampable_output_paths`] filters it
/// out of `current_gen_paths` before this function ever sees it. That is not "this run happened
/// not to touch it": it is every run, forever, for every manifest of that shape, from the day it
/// is first scaffolded. [`registered_unmarkable_manifest_dirs`] closes the gap by also consulting
/// the committed ownership record, which already tracks exactly these paths for an unrelated
/// reason (the write-time ownership guard). See that function's doc for why this is a general
/// registration rather than a wasm-specific carve-out. ~keep
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_node_lock_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
    check_generated_node_lock_freshness_tolerating_pending_publish(generated_paths, base_dir, None)
}

/// The shared collection step behind [`check_generated_node_lock_freshness`] and
/// [`check_generated_node_lock_freshness_tolerating_pending_publish`] -- see the former's doc
/// comment for the `registered_unmarkable_manifest_dirs` rationale.
fn collect_generated_node_lock_findings(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Vec<StaleNodeLockFinding> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("package.json") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    let registered_dirs = registered_unmarkable_manifest_dirs(base_dir, "package.json");
    let registered_only = registered_dirs.difference(&directories).count();
    directories.extend(registered_dirs);
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_node_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        registered_only_dirs = registered_only,
        findings = findings.len(),
        "checked generated package.json files against their committed pnpm-lock.yaml"
    );
    findings
}

/// Same check as [`check_generated_node_lock_freshness`], except every finding in a lockfile
/// whose own self-dependency row is fully explained by this crate's pending, not-yet-published
/// registry-mode `test_apps` self-dependency is downgraded to a `tracing::warn!` instead of
/// failing the stage -- the npm sibling of
/// [`crate::cli::pipeline::lock_freshness::cargo::check_generated_lock_freshness_tolerating_pending_publish`].
/// See [`registry_self_dependency`]'s doc for what "explained" means here and why it is
/// deliberately conservative.
///
/// ~keep alef #A5: tolerance is scoped to the whole LOCK, not just the self-dependency row that
/// explains it. A pending self-dependency (`@xberg-io/tree-sitter-language-pack@1.16.1`, not yet
/// published) makes the *entire* `pnpm-lock.yaml` unresolvable -- pnpm cannot even attempt to
/// relock the file to pick up an unrelated sibling drift (`@types/node`, `vitest`, `rollup`)
/// until that self-dependency publishes, so `pnpm install --lockfile-only` fails immediately with
/// `ERR_PNPM_NO_MATCHING_VERSION` on the self-dependency before it ever reaches the siblings. A
/// per-finding partition that hard-fails those siblings therefore prescribes a remedy the
/// operator cannot run yet. Once the release actually publishes, `collect_generated_node_lock_findings`
/// stops reporting the self-dependency row at all (the specifiers agree), so this exemption
/// narrows back down to nothing on its own -- it never permanently hides a lock's other findings.
///
/// ~keep alef #A6: one self-dependency identity is not enough. `collect_generated_node_lock_findings`
/// walks every `test_apps/*` directory holding an alef-generated `package.json`, and that
/// includes BOTH the `"node"` test_app (`@xberg-io/html-to-markdown`) AND the `"wasm"` test_app
/// (`@xberg-io/html-to-markdown-wasm`) -- two different `[crates.e2e...packages.<lang>]` rows,
/// two different published npm packages, each with its own pending-publish window. Resolving
/// only `"node"`'s self-dependency left the wasm test_app's identical pending-self-version row
/// unrecognised: it does not match the node identity by name, so it fell through to `real` and
/// hard-failed generation on a specifier the release itself cannot satisfy until it ships. Every
/// lang in [`PACKAGE_JSON_EMITTING_LANGS`] gets its own [`registry_self_dependency`] call, and a
/// finding is pending if it matches ANY of them.
pub(crate) fn check_generated_node_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
    resolved_cfg: Option<&crate::core::config::ResolvedCrateConfig>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_node_lock_findings(generated_paths, base_dir);
    if findings.is_empty() {
        return None;
    }
    let self_dependencies: Vec<RegistrySelfDependency> = resolved_cfg
        .map(|cfg| {
            PACKAGE_JSON_EMITTING_LANGS
                .into_iter()
                .filter_map(|lang| registry_self_dependency(cfg, lang, |package| package.name.clone(), str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if self_dependencies.is_empty() {
        return Some(anyhow::anyhow!(stale_node_lock_message(&findings)));
    }

    let pending_locks: HashSet<PathBuf> = findings
        .iter()
        .filter(|finding| {
            self_dependencies.iter().any(|self_dependency| {
                finding.dependency == self_dependency.name && finding.requirement == self_dependency.requirement
            })
        })
        .map(|finding| finding.lock.clone())
        .collect();
    let (pending, real): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .partition(|finding| pending_locks.contains(&finding.lock));

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed pnpm-lock.yaml pin(s) below require this crate's own version, which is not on the \
             registry yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_node_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_node_lock_message(&real)))
    }
}

/// Every `dependencies` / `devDependencies` specifier declared in `package_json_dir/package.json`
/// that the sibling `package_json_dir/pnpm-lock.yaml` records a different specifier for.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_node_lock_findings(package_json_dir: &Path) -> Vec<StaleNodeLockFinding> {
    let manifest_path = package_json_dir.join("package.json");
    let lock_path = package_json_dir.join("pnpm-lock.yaml");
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
    let Ok(lock_yaml) = serde_saphyr::from_str::<serde_json::Value>(&lock_text) else {
        return Vec::new();
    };

    let mut findings = Vec::new();
    for bucket in NODE_DEPENDENCY_BUCKETS {
        let locked = locked_node_specifiers(&lock_yaml, bucket);
        if locked.is_empty() {
            continue;
        }
        let Some(declared) = manifest_json.get(bucket).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for (name, spec_value) in declared {
            let Some(requirement) = spec_value.as_str() else {
                continue;
            };
            if !is_checkable_node_specifier(requirement) {
                continue;
            }
            // ~keep One-sided, matching `stale_lock_findings`'s rule above: a name absent from
            // the lock's bucket is never reported. Absence here is not only the ordinary
            // ambiguity (a dependency just added and not yet installed is indistinguishable from
            // one this reader failed to find) but also a hedge against misreading the lockfile's
            // own shape -- `locked_node_specifiers` falls back between the `importers.".".*`
            // (lockfileVersion 9+) and flat (lockfileVersion 6 and earlier) layouts, and a
            // lockfile in some third shape neither fallback anticipated looks identical to "name
            // absent". Reporting absence would turn an unfamiliar lockfile shape into a false
            // failure; only a contradiction pnpm's own frozen-lockfile check would reject is a
            // finding.
            let Some(locked_requirement) = locked.get(name.as_str()) else {
                continue;
            };
            if !is_checkable_node_specifier(locked_requirement) {
                continue;
            }
            if locked_requirement.trim() == requirement.trim() {
                continue;
            }
            findings.push(StaleNodeLockFinding {
                lock: lock_path.clone(),
                declared_in: manifest_path.clone(),
                bucket,
                dependency: name.clone(),
                requirement: requirement.to_string(),
                locked_requirement: locked_requirement.clone(),
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

/// `name -> specifier text` pnpm recorded for one dependency bucket of the lock's own project.
///
/// Tries the workspace-aware `importers.".".{bucket}` shape (lockfileVersion 9+) first, falling
/// back to the flat `{bucket}` shape a non-workspace lockfileVersion 6 (and earlier) project
/// uses. `package_json_dir` is where alef wrote the manifest, so if `pnpm-lock.yaml` sits beside
/// it at all, that lock's own root importer key is always `.` -- there is no ambiguity to resolve
/// there, only which of the two on-disk layouts this particular pnpm version chose. A lockfile
/// this reader does not recognize (an importer keyed by something other than `.`, or truly no
/// `dependencies`/`devDependencies` at all) yields an empty map, which is safe by construction:
/// [`stale_node_lock_findings`]'s one-sided rule treats "absent from this map" identically to
/// "name not in the lock", never as a contradiction. ~keep
fn locked_node_specifiers(lock: &serde_json::Value, bucket: &str) -> BTreeMap<String, String> {
    let table = lock
        .get("importers")
        .and_then(|importers| importers.get("."))
        .and_then(|root| root.get(bucket))
        .or_else(|| lock.get(bucket))
        .and_then(serde_json::Value::as_object);
    let Some(table) = table else {
        return BTreeMap::new();
    };
    let mut specifiers = BTreeMap::new();
    for (name, value) in table {
        // ~keep Every lockfileVersion that records a `specifier` field at all (5.4+, which
        // covers both the 6 and 9 shapes this reader targets) puts the package.json text here
        // verbatim; a bare `name: version` entry from an older lockfile has no `specifier` key
        // and is silently skipped rather than misread as a version-only requirement.
        let Some(specifier) = value.get("specifier").and_then(serde_json::Value::as_str) else {
            continue;
        };
        specifiers.insert(name.to_string(), specifier.to_string());
    }
    specifiers
}

/// Whether `specifier` is a form a direct text comparison against the lock's recorded specifier
/// can safely judge.
///
/// ~keep Excluded: `npm:` aliases (the lock records the aliased package's own specifier, not this
/// one), `workspace:` and `catalog:` (resolved through a workspace root this check never reads),
/// `file:`/`link:` (see `src/snippets/session/fingerprint.rs`'s module doc: a locally linked
/// dependency's resolved content, and potentially its recorded specifier text, can move for
/// reasons a text diff here cannot verify), git specifiers in every spelling pnpm accepts (a git
/// dependency can gain a resolved commit or semver hint in the lock that was never in
/// package.json), and anything containing a bare `/` (a GitHub `owner/repo` shorthand or a local
/// path, neither a registry range). A mismatch in any of these forms is not reliable evidence of
/// drift, so the entry is skipped rather than risked as a false positive.
fn is_checkable_node_specifier(specifier: &str) -> bool {
    let trimmed = specifier.trim();
    if trimmed.is_empty() {
        return false;
    }
    const UNCHECKABLE_PREFIXES: [&str; 8] = [
        "npm:",
        "workspace:",
        "catalog:",
        "file:",
        "link:",
        "git+",
        "git:",
        "github:",
    ];
    if UNCHECKABLE_PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix)) {
        return false;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return false;
    }
    !trimmed.contains('/')
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_node_lock_message(findings: &[StaleNodeLockFinding]) -> String {
    let mut message = format!(
        "{} committed pnpm-lock.yaml specifier(s) disagree with a package.json alef generated; `pnpm \
         install --frozen-lockfile` (the CI default) will fail with ERR_PNPM_OUTDATED_LOCKFILE:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is `{}` in {} ({}), but the lock records `{}`. Fix with: pnpm install \
             --lockfile-only -C {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.bucket,
            finding.locked_requirement,
            finding.lock.parent().unwrap_or(Path::new(".")).display(),
        ));
    }
    message.push_str(
        "\nA pin held back on purpose belongs in package.json -- a lockfile cannot record an exception \
         to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "node_tests.rs"]
mod tests;
