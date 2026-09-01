//! Fail a generation run whose generated Rust manifest is vouched for beside a committed
//! `Cargo.lock` that can no longer resolve against it.
//!
//! ~keep alef: a consumer regenerated cleanly (`alef all --clean`, exit 0) and was then unable
//! to build the generated e2e crate at all: its committed `e2e/rust/Cargo.lock` pinned a
//! transitive registry dependency one minor behind what the crate's *path* dependency now
//! required, so `cargo metadata --locked` in that directory failed outright. Alef reported
//! nothing, because both mechanisms it had were keyed on the wrong fact:
//!
//! 1. [`crate::cli::pipeline::version_lockfiles::relock_lockfiles_beside_changed_manifests`]
//!    relocks only when *alef's own manifest bytes changed in this run*. The requirement that
//!    moved lived in a hand-written path dependency alef neither generates nor watches, so the
//!    generated manifest was byte-identical and the hook never fired. No amount of fixing the
//!    relock hook closes this: it is watching a file that did not change.
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
/// Retained as the unmodified control the `_tolerating_pending_publish` variant is
/// differentially tested against: the pending-publish exemption is only meaningful if the
/// plain check still fails on the same input. No production call site remains. ~keep
#[cfg(test)]
pub(crate) fn check_generated_lock_freshness(generated_paths: &HashSet<PathBuf>) -> Option<anyhow::Error> {
    // `workspace_root`/`canonical` are unused whenever `canonical` is `None`: the tolerating
    // variant returns before either is read, so this dummy root is never dereferenced. ~keep
    check_generated_lock_freshness_tolerating_pending_publish(generated_paths, Path::new("."), None)
}

/// The directories this run generated a `Cargo.toml` in, paired with every requirement in that
/// tree a committed `Cargo.lock` cannot satisfy — the shared collection step behind both
/// [`check_generated_lock_freshness`] and
/// [`check_generated_lock_freshness_tolerating_pending_publish`].
fn collect_generated_lock_findings(generated_paths: &HashSet<PathBuf>) -> Vec<StaleLockFinding> {
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
    findings
}

/// Same check as [`check_generated_lock_freshness`], except a finding fully explained by this
/// crate's own pending, not-yet-published release version is downgraded to a `tracing::warn!`
/// instead of failing the stage.
///
/// ~keep `alef generate`/`alef all` run this immediately after `sync_versions`'s own
/// `relock_cargo_lockfiles` -- which already treats exactly this disagreement as unresolvable and
/// best-effort (a warn-only skip, see that function's doc) -- and immediately before `alef
/// validate versions`, the actual release gate, which tolerates it too via
/// `checks_pass`/`check_release_lock_freshness`. Only this generation-time check disagreed,
/// hard-failing on the one disagreement every other stage in the same pipeline already treats as
/// expected and temporary: a `test_apps`/e2e manifest requiring this crate's own version at the
/// exact number being released, which cannot resolve until the release is published. That made it
/// structurally impossible for a version-bumped-but-unpublished repo to run `alef all` clean, with
/// no distinction from a genuine third-party lock drift (the `tower-http` incident this module's
/// own top-of-file doc was built for), which must still fail. Reuses
/// [`crate::cli::pipeline::version_lockfiles::explained_by_pending_publish`]
/// and [`crate::cli::commands::version_manifests::discover_cargo_locks`] -- the exact same
/// read-only classification the release gate already relies on -- rather than a second,
/// independently derived notion of "blocked", so the two can only ever agree on what counts as
/// pending.
pub(crate) fn check_generated_lock_freshness_tolerating_pending_publish(
    generated_paths: &HashSet<PathBuf>,
    workspace_root: &Path,
    canonical: Option<&str>,
) -> Option<anyhow::Error> {
    let findings = collect_generated_lock_findings(generated_paths);
    if findings.is_empty() {
        return None;
    }
    let Some(canonical) = canonical else {
        return Some(anyhow::anyhow!(stale_lock_message(&findings)));
    };

    let tracked = crate::cli::git::tracked_paths_under(workspace_root);
    let blocked: std::collections::HashMap<PathBuf, String> =
        crate::cli::commands::version_manifests::discover_cargo_locks(workspace_root, canonical, tracked.as_ref())
            .into_iter()
            .filter_map(|lock| lock.blocked_on_publish.map(|waiting_on| (lock.path, waiting_on)))
            .collect();

    let (pending, real): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .partition(|finding| super::super::version_lockfiles::explained_by_pending_publish(finding, &blocked));

    if !pending.is_empty() {
        tracing::warn!(
            "{} committed Cargo.lock pin(s) below require this crate's own version, which is not on the \
             registry yet -- expected after a version bump; resolves once the release publishes:\n{}",
            pending.len(),
            stale_lock_message(&pending)
        );
    }
    if real.is_empty() {
        None
    } else {
        Some(anyhow::anyhow!(stale_lock_message(&real)))
    }
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

/// A manifest queued for the [`reachable_requirements`] walk, paired with the feature activation
/// state the edge that reached it (a `path = ` dependency table, or the walk's own root) requests
/// — the input [`activated_optional_dependencies`] needs to resolve which of *this* manifest's
/// own optional dependencies are actually reachable.
struct QueuedManifest {
    path: PathBuf,
    requested_features: Vec<String>,
    default_features: bool,
}

/// Walk `root_manifest` and, transitively, every manifest it reaches through a `path = `
/// dependency, collecting the version requirements each one declares.
///
/// The walk crosses path dependencies because that is where the observed breakage lived: the
/// generated crate is its own workspace root and depends on the crate under test by path, so
/// every registry requirement that actually constrains its lock is written one manifest away.
fn reachable_requirements(root_manifest: &Path) -> Vec<DeclaredRequirement> {
    let mut requirements = Vec::new();
    // ~keep The root itself is never gated by an external edge -- it is the crate being built
    // directly, so its own default features are active exactly as `cargo metadata` (no
    // `--no-default-features`) would resolve them, and it requests nothing beyond that.
    let mut queue = vec![QueuedManifest {
        path: root_manifest.to_path_buf(),
        requested_features: Vec::new(),
        default_features: true,
    }];
    let mut visited: HashSet<PathBuf> = HashSet::new();
    while let Some(item) = queue.pop() {
        if visited.len() >= MAX_REACHABLE_MANIFESTS {
            tracing::warn!(
                root = %root_manifest.display(),
                limit = MAX_REACHABLE_MANIFESTS,
                "stopped walking path dependencies at the manifest limit; lock freshness for this \
                 crate was checked against a partial requirement set"
            );
            break;
        }
        let key = std::fs::canonicalize(&item.path).unwrap_or_else(|_| item.path.clone());
        if !visited.insert(key) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&item.path) else {
            continue;
        };
        let Ok(document) = toml::from_str::<toml::Value>(&text) else {
            continue;
        };
        // ~keep Only the crate alef generated contributes its dev-dependencies. Cargo does not
        // resolve a non-workspace path dependency's dev-dependencies at all, so reading them
        // would invent requirements the lock is never expected to satisfy.
        let is_root = item.path == root_manifest;
        let activated_optional_deps =
            activated_optional_dependencies(&document, &item.requested_features, item.default_features);
        collect_requirements(
            &item.path,
            &document,
            is_root,
            &activated_optional_deps,
            &mut requirements,
            &mut queue,
        );
    }
    requirements
}

/// The set of dependency keys (`[dependencies]` table aliases, not the `package = ` renamed
/// name) an optional dependency is activated under, given the features `requested` on this edge
/// plus `document`'s own default feature set when `default_features` is enabled.
///
/// ~keep Deliberately approximate, not a `cargo` feature resolver: it does not model
/// target-conditional feature edges, and a feature-array entry it cannot classify as a `dep:`
/// activation, a `dep/feature` (or weak `dep?/feature`) activation, or a plain named feature is
/// folded into BOTH interpretations (treated as an activated dependency key AND queued as a
/// feature to expand further) rather than dropped. Getting this wrong in the direction of
/// under-activating would silently resurrect the exact false positive
/// [`collect_one_requirement`]'s optional-dependency guard exists to remove -- a real
/// registry-sourced requirement dropping out of the check entirely -- which is a worse failure
/// mode here than over-activating a name that turns out not to be a dependency at all (harmless:
/// nothing looks it up).
fn activated_optional_dependencies(
    document: &toml::Value,
    requested: &[String],
    default_features: bool,
) -> HashSet<String> {
    let features_table = document.get("features").and_then(toml::Value::as_table);
    let mut activated_deps = HashSet::new();
    let mut queue: Vec<String> = requested.to_vec();
    if default_features {
        queue.push("default".to_string());
    }
    let mut visited_features: HashSet<String> = HashSet::new();
    while let Some(feature) = queue.pop() {
        if !visited_features.insert(feature.clone()) {
            continue;
        }
        let Some(entries) = features_table
            .and_then(|table| table.get(feature.as_str()))
            .and_then(toml::Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(entry) = entry.as_str() else { continue };
            if let Some(dep_key) = entry.strip_prefix("dep:") {
                activated_deps.insert(dep_key.to_string());
            } else if let Some((dep_key, _sub_feature)) = entry.split_once('/') {
                activated_deps.insert(dep_key.trim_end_matches('?').to_string());
            } else {
                activated_deps.insert(entry.to_string());
                queue.push(entry.to_string());
            }
        }
    }
    activated_deps
}

/// The `features = [...]` / `default-features` an edge (a `path = ` dependency table, possibly
/// combined with its `{ workspace = true }` inherited entry) requests on the manifest it points
/// to.
///
/// ~keep `features` unions the inherited and local arrays -- and `default-features` prefers the
/// local table -- because Cargo lets a member augment (never replace) a workspace-inherited
/// dependency's `features` while overriding its `default-features` at the usage site.
fn edge_feature_request(table: &toml::Table, inherited_table: Option<&toml::Table>) -> (Vec<String>, bool) {
    let mut features: Vec<String> = inherited_table
        .and_then(|entry| entry.get("features"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .chain(
            table
                .get("features")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten(),
        )
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect();
    features.sort();
    features.dedup();
    let default_features = table
        .get("default-features")
        .or_else(|| inherited_table.and_then(|entry| entry.get("default-features")))
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    (features, default_features)
}

/// Read one manifest's dependency tables — top level and every `[target.<cfg>.*]` variant —
/// pushing requirements onto `requirements` and path-dependency manifests onto `queue`.
fn collect_requirements(
    manifest_path: &Path,
    document: &toml::Value,
    include_dev: bool,
    activated_optional_deps: &HashSet<String>,
    requirements: &mut Vec<DeclaredRequirement>,
    queue: &mut Vec<QueuedManifest>,
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
                collect_one_requirement(manifest_path, alias, spec, activated_optional_deps, requirements, queue);
            }
        }
    }
}

/// Resolve `alias`'s `{ workspace = true }` inherited entry, when it has one, and the name cargo
/// actually resolves the dependency to (the `package = ` rename target when one is used).
///
/// ~keep An inherited entry can be either spelling `[workspace.dependencies]` accepts — the bare
/// string `dep = "1.26"` as often as the table form — so the string case has to be handled here
/// and not only in the table branch below. Reading only the table form is silent: the member
/// declares `{ workspace = true }`, no `version` is found beside it, and the requirement drops
/// out of the check entirely instead of erroring.
fn resolve_dependency_identity(
    manifest_path: &Path,
    alias: &str,
    table: &toml::Table,
) -> (Option<toml::Value>, String) {
    let inherited = table
        .get("workspace")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
        .then(|| workspace_dependency_spec(manifest_path, alias))
        .flatten();
    let name = inherited
        .as_ref()
        .and_then(toml::Value::as_table)
        .and_then(|entry| entry.get("package"))
        .or_else(|| table.get("package"))
        .and_then(toml::Value::as_str)
        .unwrap_or(alias)
        .to_string();
    (inherited, name)
}

/// Resolve a single `alias = <spec>` entry into at most one requirement plus at most one further
/// manifest to walk.
fn collect_one_requirement(
    manifest_path: &Path,
    alias: &str,
    spec: &toml::Value,
    activated_optional_deps: &HashSet<String>,
    requirements: &mut Vec<DeclaredRequirement>,
    queue: &mut Vec<QueuedManifest>,
) {
    if let Some(requirement) = spec.as_str() {
        // A bare string entry (`dep = "1.26"`) has no `optional` key to set -- only the table
        // form can declare a dependency optional -- so this is always required.
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
    let (inherited, name) = resolve_dependency_identity(manifest_path, alias, table);
    let inherited_table = inherited.as_ref().and_then(toml::Value::as_table);
    // ~keep alef #A4 (tower-http incident): `optional` is read only off the LOCAL table, never
    // the inherited `[workspace.dependencies]` entry -- Cargo requires it be declared per member,
    // since a workspace-level default would make every member's activation identical. Activation
    // is checked against `alias` (the `[dependencies]` table key), not `name` (the resolved,
    // possibly `package = `-renamed identity): Cargo's `dep:`/`dep/feature` feature syntax always
    // refers to the dependency key, never the renamed package.
    let is_optional = table.get("optional").and_then(toml::Value::as_bool).unwrap_or(false);
    if is_optional && !activated_optional_deps.contains(alias) {
        return;
    }
    let (requested_features, default_features) = edge_feature_request(table, inherited_table);
    if let Some(relative) = table.get("path").and_then(toml::Value::as_str)
        && let Some(dir) = manifest_path.parent()
    {
        queue.push(QueuedManifest {
            path: normalize_lexically(&dir.join(relative).join("Cargo.toml")),
            requested_features,
            default_features,
        });
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
        name,
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
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn stale_lock_message(findings: &[StaleLockFinding]) -> String {
    let mut message = format!(
        "{} committed Cargo.lock pin(s) cannot satisfy a requirement from a manifest alef generated; \
         `cargo metadata --locked` and `cargo build --locked` will fail in these directories:",
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
        "\nA pin held back on purpose belongs in the manifest that declares the requirement -- a lockfile \
         cannot record an exception to its own resolution.",
    );
    message
}

#[cfg(test)]
#[path = "cargo_tests.rs"]
mod tests;
