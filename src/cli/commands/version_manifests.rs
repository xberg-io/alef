use super::validate_versions::VersionCheck;
use crate::core::config::{ResolvedCrateConfig, extras::Language};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Paths this discovery is allowed to consider, or `None` when git could not answer.
///
/// ~keep A disk walk cannot tell a consumer's committed manifest from a build tool's staged
/// copy of it (gem packaging mirrors `packages/ruby/ext/**` into `packages/ruby/tmp/`, and
/// the copy is gitignored), and both defects this gate reported against a consumer came from
/// that: the staged `Cargo.lock` raised mismatch rows of its own, and the staged `Cargo.toml`
/// — keyed by package name like every other — overwrote the live crate's entry in
/// `cargo_manifest_versions`, so the *tracked* lockfile was then compared against the stale
/// staged version and failed while literally reading the canonical version. Directory-name
/// blocklists cannot close this: `tmp`, `dist`, `build`, `stage` and friends are per-tool
/// names, whereas "not committed" is the property that actually distinguishes them.
/// `None` (no repository, or no `git` binary) falls back to the unfiltered walk: this command
/// only reports, so degrading to the previous behaviour beats examining nothing.
pub(crate) type TrackedPaths<'a> = Option<&'a HashSet<PathBuf>>;

pub(super) fn collect(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let tracked = crate::cli::git::tracked_paths_under(workspace_root);
    if tracked.is_none() {
        tracing::warn!(
            workspace_root = %workspace_root.display(),
            "cannot determine which files are git-tracked (not a git work tree, or `git` is unavailable) - \
             version discovery falls back to a plain disk walk and may report build-staging copies"
        );
    }
    let tracked = tracked.as_ref();
    let mut checks = Vec::new();
    collect_csproj_checks(config, workspace_root, canonical, tracked, &mut checks);
    collect_single_manifest_checks(config, workspace_root, canonical, &mut checks);
    collect_cargo_lock_checks(workspace_root, canonical, tracked, &mut checks);
    checks.sort_by(|left, right| left.label.cmp(&right.label));
    checks
}

fn collect_csproj_checks(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    tracked: TrackedPaths<'_>,
    checks: &mut Vec<VersionCheck>,
) {
    let directory = config.package_dir(Language::Csharp);
    let assembly_version = crate::core::version::to_dotnet_assembly_version(canonical);
    for path in glob_under(workspace_root, &directory, "**/*.csproj", tracked) {
        let Some(content) = std::fs::read_to_string(&path).ok() else {
            continue;
        };
        for field in ["Version", "AssemblyVersion", "FileVersion", "InformationalVersion"] {
            let Some(found) = read_xml_element(&content, field) else {
                continue;
            };
            // ~keep The generator stamps these two fields through `to_dotnet_assembly_version`
            // because .NET rejects SemVer prereleases in them, so comparing against raw
            // canonical would flag alef's own required output as a mismatch forever.
            // `Version` and `InformationalVersion` carry the full SemVer and compare raw.
            let expected = match field {
                "AssemblyVersion" | "FileVersion" => assembly_version.as_str(),
                _ => canonical,
            };
            push_check(
                checks,
                workspace_root,
                &path,
                Some(field),
                found,
                ExpectedVersion {
                    value: expected,
                    blocked_on_publish: None,
                },
            );
        }
    }
}

fn collect_single_manifest_checks(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
    canonical: &str,
    checks: &mut Vec<VersionCheck>,
) {
    let dart = workspace_root
        .join(config.package_dir(Language::Dart))
        .join("pubspec.yaml");
    if let Some(found) = read_prefixed_value(&dart, "version:") {
        push_check(
            checks,
            workspace_root,
            &dart,
            None,
            found,
            ExpectedVersion {
                value: canonical,
                blocked_on_publish: None,
            },
        );
    }

    let zig = workspace_root
        .join(config.package_dir(Language::Zig))
        .join("build.zig.zon");
    if let Some(found) = read_zig_version(&zig) {
        push_check(
            checks,
            workspace_root,
            &zig,
            None,
            found,
            ExpectedVersion {
                value: canonical,
                blocked_on_publish: None,
            },
        );
    }
}

/// One `Cargo.lock` this workspace's version surface includes, discovered the same way for
/// both the read side (`alef validate versions`, [`collect_cargo_lock_checks`]) and the write
/// side (`sync-versions`' relock step, `crate::cli::pipeline::version_lockfiles`).
///
/// ~keep alef #148: version-sync bumped every `Cargo.toml` a lockfile pinned without ever
/// refreshing that lockfile, so `validate versions` — reading a DIFFERENT, broader set of
/// lockfiles derived independently — found the drift and failed the release gate. The two
/// sides sharing this one discovery function is what makes that divergence structurally
/// impossible rather than merely tested against.
pub(crate) struct DiscoveredCargoLock {
    pub(crate) path: PathBuf,
    /// Parsed `[[package]]` entries, empty when the lockfile could not be read or parsed.
    pub(crate) packages: Vec<toml::Value>,
    /// `name@version` this lock is waiting on, when it pins a registry dependency at the
    /// version currently being released and cannot resolve until that release is published.
    pub(crate) blocked_on_publish: Option<String>,
}

/// Every git-tracked `Cargo.lock` under `workspace_root`, excluding vendored/staged/
/// unregistered copies (see [`ignored_path`]), paired with whether it can be refreshed before
/// publish. See [`DiscoveredCargoLock`] for why this is the single shared enumeration.
pub(crate) fn discover_cargo_locks(
    workspace_root: &Path,
    canonical: &str,
    tracked: TrackedPaths<'_>,
) -> Vec<DiscoveredCargoLock> {
    let submodules = registered_submodule_paths(workspace_root);
    let manifests = cargo_manifest_versions(workspace_root, canonical, &submodules, tracked);
    let mut discovered = Vec::new();
    for lock_path in glob_under(workspace_root, "", "**/Cargo.lock", tracked) {
        if ignored_path(workspace_root, &lock_path, &submodules) {
            continue;
        }
        let Some(content) = std::fs::read_to_string(&lock_path).ok() else {
            continue;
        };
        let Some(packages) = toml::from_str::<toml::Value>(&content)
            .ok()
            .and_then(|value| value.get("package").and_then(toml::Value::as_array).cloned())
        else {
            continue;
        };
        let blocked_on_publish = unpublished_dependency(&lock_path, &packages, &manifests);
        discovered.push(DiscoveredCargoLock {
            path: lock_path,
            packages,
            blocked_on_publish,
        });
    }
    discovered
}

fn collect_cargo_lock_checks(
    workspace_root: &Path,
    canonical: &str,
    tracked: TrackedPaths<'_>,
    checks: &mut Vec<VersionCheck>,
) {
    let manifests = cargo_manifest_versions(
        workspace_root,
        canonical,
        &registered_submodule_paths(workspace_root),
        tracked,
    );
    for lock in discover_cargo_locks(workspace_root, canonical, tracked) {
        for package in &lock.packages {
            if package.get("source").is_some() {
                continue;
            }
            let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(found) = package.get("version").and_then(toml::Value::as_str) else {
                continue;
            };
            let Some(expected) = manifests.get(name) else {
                continue;
            };
            push_check(
                checks,
                workspace_root,
                &lock.path,
                Some(name),
                found.to_string(),
                ExpectedVersion {
                    value: expected,
                    blocked_on_publish: lock.blocked_on_publish.as_deref(),
                },
            );
        }
    }
}

/// The `name@version` this lockfile is waiting on, when its own drift cannot be resolved in-tree.
///
/// ~keep A stale lockfile is normally a chore: run cargo and commit the result. It is *not* when
/// the lockfile's own manifest depends on a crate this workspace builds, takes it from the
/// registry rather than by path, and asks for exactly the canonical version — the release being
/// prepared. Cargo cannot resolve that requirement until the release is published, so the
/// lockfile is pinned to the last published version and every drift row it produces is
/// unresolvable until publish rather than something a developer forgot. The two are
/// indistinguishable in the output otherwise, which is what made a genuinely-blocked row read as
/// noise. Only a registry-sourced entry at a *different* version counts: once the release lands,
/// the entry matches and this returns `None` on its own.
fn unpublished_dependency(
    lock_path: &Path,
    packages: &[toml::Value],
    manifests: &HashMap<String, String>,
) -> Option<String> {
    let content = std::fs::read_to_string(lock_path.with_file_name("Cargo.toml")).ok()?;
    let manifest = toml::from_str::<toml::Value>(&content).ok()?;
    for (name, required) in registry_dependencies_on_local_crates(&manifest, manifests) {
        let Some(resolved) = packages
            .iter()
            .find(|package| package.get("name").and_then(toml::Value::as_str) == Some(name.as_str()))
        else {
            continue;
        };
        if resolved.get("source").is_none() {
            continue;
        }
        let resolved_version = resolved
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        if resolved_version != required {
            return Some(format!("{name}@{required}"));
        }
    }
    None
}

/// Dependencies of `manifest` that name a crate built in this workspace, come from the registry
/// (no `path`), and pin exactly the version that workspace currently declares for it.
fn registry_dependencies_on_local_crates(
    manifest: &toml::Value,
    manifests: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let mut requirements = Vec::new();
    for table in dependency_tables(manifest) {
        for (key, value) in table {
            let name = value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key.as_str())
                .to_string();
            let Some(local_version) = manifests.get(&name) else {
                continue;
            };
            if value.get("path").is_some() {
                continue;
            }
            let requirement = match value {
                toml::Value::String(requirement) => Some(requirement.as_str()),
                other => other.get("version").and_then(toml::Value::as_str),
            };
            let Some(requirement) = requirement else {
                continue;
            };
            if exact_requirement(requirement) == local_version.as_str() {
                requirements.push((name, local_version.clone()));
            }
        }
    }
    requirements
}

/// Strip the comparator off a single-version requirement so `=1.2.3`, `^1.2.3` and `1.2.3` all
/// compare equal to the version they pin. Anything more elaborate (ranges, wildcards) simply
/// fails the equality test it feeds and is treated as not pinning the pending release.
fn exact_requirement(requirement: &str) -> &str {
    requirement.trim().trim_start_matches(['=', '^', '~']).trim()
}

fn dependency_tables(manifest: &toml::Value) -> Vec<&toml::Table> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut sources = vec![manifest];
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        sources.extend(targets.values());
    }
    let mut tables = Vec::new();
    for source in sources {
        for kind in KINDS {
            if let Some(table) = source.get(kind).and_then(toml::Value::as_table) {
                tables.push(table);
            }
        }
    }
    tables
}

fn cargo_manifest_versions(
    workspace_root: &Path,
    canonical: &str,
    submodules: &HashSet<PathBuf>,
    tracked: TrackedPaths<'_>,
) -> HashMap<String, String> {
    let mut versions = HashMap::new();
    for path in glob_under(workspace_root, "", "**/Cargo.toml", tracked) {
        if ignored_path(workspace_root, &path, submodules) {
            continue;
        }
        let Some(content) = std::fs::read_to_string(path).ok() else {
            continue;
        };
        let Some(package) = toml::from_str::<toml::Value>(&content)
            .ok()
            .and_then(|value| value.get("package").and_then(toml::Value::as_table).cloned())
        else {
            continue;
        };
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let version = package
            .get("version")
            .and_then(toml::Value::as_str)
            .unwrap_or(canonical);
        versions.insert(name.to_string(), version.to_string());
    }
    versions
}

fn glob_under(workspace_root: &Path, directory: &str, suffix: &str, tracked: TrackedPaths<'_>) -> Vec<PathBuf> {
    let root = glob::Pattern::escape(&workspace_root.to_string_lossy());
    let directory = directory.trim_matches(['/', '\\']);
    let pattern = if directory.is_empty() {
        format!("{root}/{suffix}")
    } else {
        format!("{root}/{directory}/{suffix}")
    };
    glob::glob(&pattern)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|path| tracked.is_none_or(|tracked| tracked.contains(path)))
        .collect()
}

// ~keep `vendor` (Cargo/Go-style vendoring) and `deps` (Mix's fetched-dependency
// cache) hold frozen copies of a crate's Cargo.toml pulled in at whatever version
// was current when they were vendored/fetched. `cargo_manifest_versions` keys its
// map by package name alone, so in a consumer repo a vendored `<crate>`/`<crate>_nif`
// manifest silently overwrote the live one's entry and poisoned every Cargo.lock
// comparison for that name repo-wide — not just the vendored copy's own lock.
fn ignored_path(workspace_root: &Path, path: &Path, submodules: &HashSet<PathBuf>) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("target" | ".git" | ".alef-cache" | "vendor" | "deps")
        )
    }) || is_inside_unregistered_checkout(workspace_root, relative, submodules)
}

// ~keep A linked `git worktree` and a submodule are indistinguishable by their root
// marker — both carry a `.git` FILE pointing at the owning repository's gitdir (a
// nested full clone carries a `.git` directory). They are not interchangeable here:
// `.gitmodules` registers a submodule as a declared part of this repo, so its
// manifests belong in the version map, while an unregistered checkout under the tree
// is an independent worktree sitting at its own commit whose manifests are unrelated
// to this repo's version consistency — and, mid-regeneration, can differ between two
// runs of the same command. Only the *nearest* enclosing root matters, so this walks
// ancestors outward from `relative` instead of every path down from the workspace.
fn is_inside_unregistered_checkout(workspace_root: &Path, relative: &Path, submodules: &HashSet<PathBuf>) -> bool {
    let mut ancestor = workspace_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            // ~keep The final component is the manifest/lockfile itself, never a checkout root.
            break;
        }
        ancestor.push(component);
        if !submodules.contains(&ancestor) && ancestor.join(".git").exists() {
            return true;
        }
    }
    false
}

fn registered_submodule_paths(workspace_root: &Path) -> HashSet<PathBuf> {
    let mut registered = HashSet::new();
    let Ok(content) = std::fs::read_to_string(workspace_root.join(".gitmodules")) else {
        return registered;
    };
    let mut inside_submodule_section = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside_submodule_section = line.starts_with("[submodule");
            continue;
        }
        if !inside_submodule_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            registered.insert(workspace_root.join(value));
        }
    }
    registered
}

fn read_xml_element(content: &str, element: &str) -> Option<String> {
    let open = format!("<{element}>");
    let close = format!("</{element}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)?;
    Some(content[start..start + end].trim().to_string())
}

fn read_prefixed_value(path: &Path, prefix: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|line| {
        line.strip_prefix(prefix)
            .map(|value| value.split('#').next().unwrap_or(value).trim().to_string())
    })
}

fn read_zig_version(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()?.lines().find_map(|line| {
        line.trim()
            .strip_prefix(".version")?
            .split_once('=')
            .map(|(_, value)| value.trim().trim_end_matches(',').trim_matches('"').to_string())
    })
}

/// The version a manifest is expected to carry, and whether a mismatch is blocked on publish
/// rather than failing outright. Grouped only to keep [`push_check`]'s parameter list short.
struct ExpectedVersion<'a> {
    value: &'a str,
    blocked_on_publish: Option<&'a str>,
}

fn push_check(
    checks: &mut Vec<VersionCheck>,
    workspace_root: &Path,
    path: &Path,
    field: Option<&str>,
    found: String,
    expected: ExpectedVersion<'_>,
) {
    let mut label = path
        .strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if let Some(field) = field {
        label.push('#');
        label.push_str(field);
    }
    let matches = found == expected.value;
    checks.push(VersionCheck {
        label,
        matches,
        found: Some(found),
        blocked_on_publish: expected.blocked_on_publish.filter(|_| !matches).map(str::to_string),
    });
}

#[cfg(test)]
#[path = "version_manifests/tests.rs"]
mod tests;
