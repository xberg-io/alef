//! Fail a generation run whose generated Rust manifest is vouched for beside a committed
//! `Cargo.lock` that can no longer resolve against it.
//!
//! ~keep alef: a consumer regenerated cleanly (`alef all --clean`, exit 0) and was then unable
//! to build the generated e2e crate at all: its committed `e2e/rust/Cargo.lock` pinned a
//! transitive registry dependency one minor behind what the crate's *path* dependency now
//! required, so `cargo metadata --locked` in that directory failed outright. Alef reported
//! nothing, because both mechanisms it had were keyed on the wrong fact:
//!
//! 1. [`super::version_lockfiles::relock_lockfiles_beside_changed_manifests`] relocks only when
//!    *alef's own manifest bytes changed in this run*. The requirement that moved lived in a
//!    hand-written path dependency alef neither generates nor watches, so the generated manifest
//!    was byte-identical and the hook never fired. No amount of fixing the relock hook closes
//!    this: it is watching a file that did not change.
//! 2. That relock is best-effort anyway (`cargo update --offline -w`, warn-only), so even when
//!    it does fire it can leave the lock stale and still exit 0.
//!
//! This module adds the missing observation rather than a third write path: after generation
//! completes, every directory holding a manifest this run generated is checked for a committed
//! lock that contradicts it, and a contradiction is recorded as a stage failure. Alef still
//! never authors a `Cargo.lock` — it only refuses to keep claiming a manifest is good when the
//! lock beside it says otherwise.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// Upper bound on path-dependency manifests walked from one generated manifest. A malformed or
/// adversarial tree of `path = ` links cannot make this walk unbounded; the visited set already
/// makes cycles terminate, this caps sheer breadth.
const MAX_REACHABLE_MANIFESTS: usize = 512;

/// Dependency tables a manifest can declare, in the order they are read.
const DEPENDENCY_TABLES: [&str; 3] = ["dependencies", "build-dependencies", "dev-dependencies"];

/// One version requirement reachable from a generated manifest that no version present in the
/// sibling `Cargo.lock` satisfies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleLockFinding {
    /// The committed lock that contradicts the requirement.
    pub(crate) lock: PathBuf,
    /// The manifest the requirement is written in — often a path dependency, not the generated
    /// manifest itself, which is exactly why "did alef rewrite this file" could not see it.
    pub(crate) declared_in: PathBuf,
    /// Package name as cargo resolves it (the `package = ` rename target when one is used).
    pub(crate) dependency: String,
    /// The requirement text as written.
    pub(crate) requirement: String,
    /// Every version of `dependency` the lock does pin, sorted, for the report.
    pub(crate) locked_versions: Vec<String>,
}

/// A single `name = req` pair read off some manifest in the reachable set.
struct DeclaredRequirement {
    manifest: PathBuf,
    name: String,
    requirement: String,
}

/// Check every directory in which this run generated a `Cargo.toml` for a committed
/// `Cargo.lock` that contradicts it, returning the failure to record when one does.
///
/// `generated_paths` is the run's own set of generated output paths, so the check covers exactly
/// the manifests alef vouches for and nothing else — a lock beside a manifest alef did not write
/// is none of its business.
pub(crate) fn check_generated_lock_freshness(generated_paths: &HashSet<PathBuf>) -> Option<anyhow::Error> {
    let mut directories = BTreeSet::new();
    for path in generated_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Some(dir) = path.parent() {
            directories.insert(dir.to_path_buf());
        }
    }
    let mut findings = Vec::new();
    for dir in &directories {
        findings.extend(stale_lock_findings(dir));
    }
    tracing::debug!(
        manifest_dirs = directories.len(),
        findings = findings.len(),
        "checked generated Rust manifests against their committed lockfiles"
    );
    if findings.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(stale_lock_message(&findings)))
}

/// Every requirement reachable from `manifest_dir/Cargo.toml` that the sibling
/// `manifest_dir/Cargo.lock` cannot satisfy.
///
/// Returns empty when either file is missing or unparseable: alef never authors a lockfile, so a
/// directory without one is a deliberate consumer choice, not a defect to report.
pub(crate) fn stale_lock_findings(manifest_dir: &Path) -> Vec<StaleLockFinding> {
    let manifest_path = manifest_dir.join("Cargo.toml");
    let lock_path = manifest_dir.join("Cargo.lock");
    if !manifest_path.is_file() {
        return Vec::new();
    }
    let Ok(lock_text) = std::fs::read_to_string(&lock_path) else {
        return Vec::new();
    };
    let locked = locked_versions(&lock_text);
    if locked.is_empty() {
        return Vec::new();
    }
    let mut findings = Vec::new();
    for declared in reachable_requirements(&manifest_path) {
        // ~keep The rule is deliberately one-sided: a requirement is reported only when its
        // package name IS pinned in the lock and NO pinned version satisfies it. A name absent
        // from the lock is never reported, because absence has many innocent explanations this
        // check is not equipped to tell apart from a real gap — cargo omits a path dependency's
        // dev-dependencies, a `[patch]`/`[replace]` entry can rewrite the resolved name, and a
        // renamed or platform-gated dependency can resolve to a name this reader did not derive.
        // Reporting absence would turn a healthy tree red; reporting a contradiction cannot,
        // because cargo itself refuses that lock. This check is therefore incomplete on purpose
        // and must stay that way: it is a guard against a false green, not a resolver.
        let Some(versions) = locked.get(&declared.name) else {
            continue;
        };
        let Ok(requirement) = semver::VersionReq::parse(&declared.requirement) else {
            continue;
        };
        if versions.iter().any(|version| requirement.matches(version)) {
            continue;
        }
        findings.push(StaleLockFinding {
            lock: lock_path.clone(),
            declared_in: declared.manifest.clone(),
            dependency: declared.name.clone(),
            requirement: declared.requirement.clone(),
            locked_versions: versions.iter().map(ToString::to_string).collect(),
        });
    }
    findings.sort_by(|left, right| {
        left.dependency
            .cmp(&right.dependency)
            .then_with(|| left.requirement.cmp(&right.requirement))
    });
    findings.dedup_by(|left, right| left.dependency == right.dependency && left.requirement == right.requirement);
    findings
}

/// `name -> every version pinned for it` from a `Cargo.lock`'s `[[package]]` array.
fn locked_versions(lock_text: &str) -> BTreeMap<String, Vec<semver::Version>> {
    let mut locked: BTreeMap<String, Vec<semver::Version>> = BTreeMap::new();
    let Some(packages) = toml::from_str::<toml::Value>(lock_text)
        .ok()
        .and_then(|value| value.get("package").and_then(toml::Value::as_array).cloned())
    else {
        return locked;
    };
    for package in packages {
        let (Some(name), Some(version)) = (
            package.get("name").and_then(toml::Value::as_str),
            package.get("version").and_then(toml::Value::as_str),
        ) else {
            continue;
        };
        if let Ok(parsed) = semver::Version::parse(version) {
            locked.entry(name.to_string()).or_default().push(parsed);
        }
    }
    for versions in locked.values_mut() {
        versions.sort();
    }
    locked
}

/// Walk `root_manifest` and, transitively, every manifest it reaches through a `path = `
/// dependency, collecting the version requirements each one declares.
///
/// The walk crosses path dependencies because that is where the observed breakage lived: the
/// generated crate is its own workspace root and depends on the crate under test by path, so
/// every registry requirement that actually constrains its lock is written one manifest away.
fn reachable_requirements(root_manifest: &Path) -> Vec<DeclaredRequirement> {
    let mut requirements = Vec::new();
    let mut queue = vec![root_manifest.to_path_buf()];
    let mut visited: HashSet<PathBuf> = HashSet::new();
    while let Some(manifest_path) = queue.pop() {
        if visited.len() >= MAX_REACHABLE_MANIFESTS {
            tracing::warn!(
                root = %root_manifest.display(),
                limit = MAX_REACHABLE_MANIFESTS,
                "stopped walking path dependencies at the manifest limit; lock freshness for this \
                 crate was checked against a partial requirement set"
            );
            break;
        }
        let key = std::fs::canonicalize(&manifest_path).unwrap_or_else(|_| manifest_path.clone());
        if !visited.insert(key) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let Ok(document) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        // ~keep Only the crate alef generated contributes its dev-dependencies. Cargo does not
        // resolve a non-workspace path dependency's dev-dependencies at all, so reading them
        // would invent requirements the lock is never expected to satisfy.
        let is_root = manifest_path == root_manifest;
        collect_requirements(&manifest_path, &document, is_root, &mut requirements, &mut queue);
    }
    requirements
}

/// Read one manifest's dependency tables — top level and every `[target.<cfg>.*]` variant —
/// pushing requirements onto `requirements` and path-dependency manifests onto `queue`.
fn collect_requirements(
    manifest_path: &Path,
    document: &toml::Value,
    include_dev: bool,
    requirements: &mut Vec<DeclaredRequirement>,
    queue: &mut Vec<PathBuf>,
) {
    let mut tables: Vec<&toml::Value> = vec![document];
    if let Some(targets) = document.get("target").and_then(toml::Value::as_table) {
        tables.extend(targets.values());
    }
    for table in tables {
        for section in DEPENDENCY_TABLES {
            if section == "dev-dependencies" && !include_dev {
                continue;
            }
            let Some(entries) = table.get(section).and_then(toml::Value::as_table) else {
                continue;
            };
            for (alias, spec) in entries {
                collect_one_requirement(manifest_path, alias, spec, requirements, queue);
            }
        }
    }
}

/// Resolve a single `alias = <spec>` entry into at most one requirement plus at most one further
/// manifest to walk.
fn collect_one_requirement(
    manifest_path: &Path,
    alias: &str,
    spec: &toml::Value,
    requirements: &mut Vec<DeclaredRequirement>,
    queue: &mut Vec<PathBuf>,
) {
    if let Some(requirement) = spec.as_str() {
        requirements.push(DeclaredRequirement {
            manifest: manifest_path.to_path_buf(),
            name: alias.to_string(),
            requirement: requirement.to_string(),
        });
        return;
    }
    let Some(table) = spec.as_table() else {
        return;
    };
    // ~keep An inherited entry can be either spelling `[workspace.dependencies]` accepts — the
    // bare string `dep = "1.26"` as often as the table form — so the string case has to be
    // handled here and not only at the top of this function. Reading only the table form is
    // silent: the member declares `{ workspace = true }`, no `version` is found beside it, and
    // the requirement drops out of the check entirely instead of erroring.
    let inherited = table
        .get("workspace")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
        .then(|| workspace_dependency_spec(manifest_path, alias))
        .flatten();
    let inherited_table = inherited.as_ref().and_then(toml::Value::as_table);
    let name = inherited_table
        .and_then(|entry| entry.get("package"))
        .or_else(|| table.get("package"))
        .and_then(toml::Value::as_str)
        .unwrap_or(alias);
    if let Some(relative) = table.get("path").and_then(toml::Value::as_str)
        && let Some(dir) = manifest_path.parent()
    {
        queue.push(normalize_lexically(&dir.join(relative).join("Cargo.toml")));
    }
    // ~keep A path or git dependency's pinned entry is not a registry version requirement: a
    // path package's locked version is read straight out of the manifest tree already walked
    // above, and a git dependency is locked by revision, not by the `version` field beside it.
    // Checking either adds no coverage for the defect this module exists for and both invent
    // false positives.
    let is_source_pinned = |entry: &toml::Table| entry.contains_key("path") || entry.contains_key("git");
    if is_source_pinned(table) || inherited_table.is_some_and(is_source_pinned) {
        return;
    }
    let requirement = match inherited.as_ref() {
        Some(value) => value
            .as_str()
            .or_else(|| value.get("version").and_then(toml::Value::as_str)),
        None => table.get("version").and_then(toml::Value::as_str),
    };
    let Some(requirement) = requirement else {
        return;
    };
    requirements.push(DeclaredRequirement {
        manifest: manifest_path.to_path_buf(),
        name: name.to_string(),
        requirement: requirement.to_string(),
    });
}

/// Collapse `.` and `..` components without touching the filesystem.
///
/// ~keep Lexical, not `canonicalize`: the walked path may not exist yet (a misconfigured `path =
/// `), and a symlink-resolved path is the wrong thing to print at an operator who has to open
/// the file. `..` is only popped when a real named component precedes it, so a path that escapes
/// its own root keeps the leading `..` rather than silently becoming a different path.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut components: Vec<std::path::Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if matches!(components.last(), Some(std::path::Component::Normal(_))) => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components.into_iter().collect()
}

/// The `[workspace.dependencies] <alias>` entry a `{ workspace = true }` dependency inherits.
///
/// Searches upward from `manifest_path` for the nearest ancestor manifest carrying a
/// `[workspace]` table and reads the alias out of it. Returns `None` when no such ancestor
/// exists or the alias is absent, which leaves the dependency unchecked — the one-sided rule in
/// [`stale_lock_findings`] applies here too: an unresolved inheritance must never be reported.
fn workspace_dependency_spec(manifest_path: &Path, alias: &str) -> Option<toml::Value> {
    // ~keep Starts at the manifest's own directory, not its parent: a root crate that is also
    // the workspace root declares `[workspace.dependencies]` in the very file whose
    // `{ workspace = true }` entry is being resolved, which is the most common shape of all.
    let mut directory = manifest_path.parent();
    while let Some(current) = directory {
        let candidate = current.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&candidate)
            && let Ok(document) = toml::from_str::<toml::Value>(&text)
            && let Some(workspace) = document.get("workspace")
        {
            return workspace
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .and_then(|table| table.get(alias))
                .cloned();
        }
        directory = current.parent();
    }
    None
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
fn stale_lock_message(findings: &[StaleLockFinding]) -> String {
    let mut message = format!(
        "{} committed Cargo.lock pin(s) cannot satisfy a requirement reachable from a manifest \
         alef generated. `cargo metadata --locked` (and every `cargo build --locked` / CI job) \
         will fail in these directories even though generation itself succeeded. Alef does not \
         author lockfiles, so this is reported rather than rewritten:",
        findings.len()
    );
    for finding in findings {
        message.push_str(&format!(
            "\n  - {}: `{}` is required as `{}` by {}, but the lock pins only {}. Fix with: cargo \
             update --manifest-path {} -p {}",
            finding.lock.display(),
            finding.dependency,
            finding.requirement,
            finding.declared_in.display(),
            finding.locked_versions.join(", "),
            finding
                .lock
                .parent()
                .unwrap_or(Path::new("."))
                .join("Cargo.toml")
                .display(),
            finding.dependency,
        ));
    }
    message.push_str(
        "\nIf a pin is intentionally held back, resolve it in the manifest that declares the \
         requirement — a lockfile cannot record an exception to its own resolution.",
    );
    message
}

/// Dependency buckets alef itself writes into a generated `package.json` -- see
/// `crate::e2e::codegen::typescript::config::render_package_json` (and its wasm counterpart),
/// which only ever populate `dependencies` / `devDependencies`. Checking a bucket alef never
/// writes would find nothing but a hand-authored drift this module has no business reporting.
const NODE_DEPENDENCY_BUCKETS: [&str; 2] = ["dependencies", "devDependencies"];

/// One `package.json` specifier whose sibling `pnpm-lock.yaml` records a different specifier for
/// the same dependency name and bucket.
///
/// Unlike [`StaleLockFinding`], there is no path-dependency walk: the requirement text and the
/// generated manifest are the same file, so the comparison is direct.
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
/// Mirrors [`check_generated_lock_freshness`] one call away in the same module rather than
/// sharing machinery with it: the cargo check's job is a transitive path-dependency walk followed
/// by semver resolution against a lock it must reconstruct, while this check's job is a direct
/// text comparison against a manifest alef already has in hand. The two are different problems
/// wearing similar names, and forcing them through one abstraction is how a later change to
/// either drifts the other -- see the `avoid-duplication` rule and the `two-generators-disagree`
/// pattern this repo is watching for. ~keep
///
/// `generated_paths` alone is structurally blind to a whole class of `package.json`: any emitted
/// `generated_header: false` (`crates/*-wasm/package.json`, `crates/*-node/package.json` and its
/// per-platform `npm/<platform>/package.json` siblings, ...) never carries an `alef:hash:` marker
/// -- JSON has no comment syntax to hold one -- so [`GeneratedFile::carries_alef_marker`] is
/// always `false` for it and [`super::generate::stampable_output_paths`] filters it out of
/// `current_gen_paths` before this function ever sees it. That is not "this run happened not to
/// touch it": it is every run, forever, for every manifest of that shape, from the day it is
/// first scaffolded. [`registered_unmarkable_manifest_dirs`] closes the gap by also consulting the
/// committed ownership record, which already tracks exactly these paths for an unrelated reason
/// (the write-time ownership guard). See that function's doc for why this is a general
/// registration rather than a wasm-specific carve-out. ~keep
pub(crate) fn check_generated_node_lock_freshness(
    generated_paths: &HashSet<PathBuf>,
    base_dir: &Path,
) -> Option<anyhow::Error> {
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
    if findings.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(stale_node_lock_message(&findings)))
}

/// Directories holding a `file_name` manifest that [`GeneratedFile::carries_alef_marker`] can
/// never certify, because the format has no comment syntax to carry an `alef:hash:` marker at all
/// (`generated_header: false` JSON, principally `package.json`). A lock-freshness gate keyed only
/// on this run's in-memory `current_gen_paths` -- itself filtered by that same marker predicate,
/// see [`super::generate::stampable_output_paths`] -- structurally never examines these paths, in
/// any run, which is the gap this function exists to close.
///
/// Reads the committed ownership record ([`crate::cli::cache::read_committed_owned_paths`],
/// `.alef-ownership.toml`) instead: it is the durable, general-purpose list of every path alef has
/// authorised itself to own *precisely because* it cannot carry a marker -- populated by
/// `write_scaffold_files_report`'s own write guard the first time it creates such a file, for
/// every unmarkable manifest kind alef emits, not only `package.json` for wasm. Filtering that
/// list by `file_name` extends `generated_paths` with every alef-managed manifest of that name
/// the registry already knows about, including one this particular run did not touch (a
/// `--crate`-scoped run, or a language skipped by the per-language cache) -- which is strictly
/// more correct for a freshness check than "only what this run happened to regenerate": the drift
/// this gate exists to catch does not require this run to have written the manifest, only for the
/// manifest and its sibling lock to disagree right now.
///
/// General by construction: nothing here names `wasm` or `node`. `crates/*-node/package.json` is
/// `generated_header: false` for the identical reason `crates/*-wasm/package.json` is and was
/// found to share this exact blind spot while auditing it, and both are closed by the same call
/// with no per-backend special case. A future unmarkable manifest this registry starts tracking
/// -- a PHP `composer.json`-vs-`composer.lock` gate, should one ever be added -- would read from
/// this identical list rather than inventing its own. ~keep
fn registered_unmarkable_manifest_dirs(base_dir: &Path, file_name: &str) -> BTreeSet<PathBuf> {
    crate::cli::cache::read_committed_owned_paths(base_dir)
        .iter()
        .map(|relative| base_dir.join(relative))
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some(file_name))
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect()
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

/// One `pyproject.toml` `[project.dependencies]` entry whose sibling `uv.lock` records a different
/// specifier for the same package.
///
/// Unlike [`StaleLockFinding`] there is no path-dependency walk (the requirement text and the
/// generated manifest are one file, same as the node check), and unlike [`StaleNodeLockFinding`]
/// the comparison is not against another copy of the manifest's own text -- it is against a
/// second, lock-recorded copy of that same text. `uv.lock` carries a `[package.metadata]
/// requires-dist` entry for the project's own lock-file package that is populated verbatim from
/// `pyproject.toml` at lock time (confirmed against astral-sh/uv#17549, which is exactly a report
/// of that recorded copy going stale relative to the manifest). That is uv's own invalidation
/// mechanism: `uv sync --locked` / `uv lock --check` fail with "The lockfile ... needs to be
/// updated" when the recorded copy no longer matches, regardless of whether the currently locked
/// *version* still happens to satisfy the manifest's range. That last clause is why this check
/// uses text equality (the node model) rather than semver-range satisfaction (the cargo model): an
/// open lower bound like `pyrefly>=1.1.1` is satisfied by a lock still pinning `1.1.1` a month
/// later even though uv itself would call that lock stale the moment anything forces a re-resolve
/// -- a range check would stay silent on exactly the drift this exists to catch, while a text
/// check catches it whenever the recorded copy itself has moved.
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
/// A third sibling beside [`check_generated_lock_freshness`] and
/// [`check_generated_node_lock_freshness`], not a shared abstraction -- see the `~keep` comment on
/// the node check's doc comment for why forcing ecosystem-specific lock reading through one
/// function is how a later change to one silently drifts another.
pub(crate) fn check_generated_uv_lock_freshness(generated_paths: &HashSet<PathBuf>) -> Option<anyhow::Error> {
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
    if findings.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(stale_uv_lock_message(&findings)))
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
fn stale_uv_lock_message(findings: &[StaleUvLockFinding]) -> String {
    let mut message = format!(
        "{} committed uv.lock specifier(s) disagree with a pyproject.toml alef generated. `uv sync \
         --locked` (and every CI job that runs it with the default frozen lockfile) will fail with \
         \"The lockfile at `uv.lock` needs to be updated\" even though generation itself succeeded. \
         Alef does not author lockfiles, so this is reported rather than rewritten:",
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
        "\nIf a pin is intentionally held back, resolve it in pyproject.toml -- a lockfile cannot \
         record an exception to its own resolution.",
    );
    message
}

/// Render the operator-facing failure: what disagrees, where each side said it, and the command
/// that reconciles them.
fn stale_node_lock_message(findings: &[StaleNodeLockFinding]) -> String {
    let mut message = format!(
        "{} committed pnpm-lock.yaml specifier(s) disagree with a package.json alef generated. \
         `pnpm install` (and every CI job that runs it with the default frozen lockfile) will \
         fail with ERR_PNPM_OUTDATED_LOCKFILE even though generation itself succeeded. Alef does \
         not author lockfiles, so this is reported rather than rewritten:",
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
        "\nIf a pin is intentionally held back, resolve it in package.json -- a lockfile cannot \
         record an exception to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "lock_freshness_tests.rs"]
mod tests;
