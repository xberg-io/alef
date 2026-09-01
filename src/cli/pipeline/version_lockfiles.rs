//! Relock every `Cargo.lock` that `alef validate versions` will check, immediately after
//! `sync_versions` rewrites the manifests those locks pin.
//!
//! ~keep alef #148: `sync_versions` bumped every `Cargo.toml` it owned but never refreshed the
//! sibling `Cargo.lock` files, so `alef validate versions` — which discovers lockfiles through
//! a separate, broader enumeration — found the stale pin and failed the release gate. Three
//! consumer releases, in three separate repos, were tagged and pushed with a stale lockfile,
//! failed validation, and never reached crates.io. Discovery
//! here is not re-derived: it calls the exact same
//! `crate::cli::commands::version_manifests::discover_cargo_locks` the validator uses, so the
//! write set and the validate set cannot drift into checking a different set of files again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use crate::cli::commands::version_manifests::discover_cargo_locks;
use crate::cli::git::tracked_paths_under;
use crate::core::backend::GeneratedFile;

use super::collect_alef_headered_paths;
use super::lock_freshness::{
    StaleLockFinding, check_generated_lock_freshness_tolerating_pending_publish, stale_lock_findings,
};

/// Run `cargo update --offline -w` in the directory of every discovered lockfile that is not
/// waiting on a pending release.
///
/// Locks `discover_cargo_locks` marks `blocked_on_publish` are skipped on purpose: they pin a
/// registry dependency at the version being released, so cargo cannot resolve that requirement
/// until the release is live on the registry — an offline update there would just fail (or do
/// nothing). `alef validate versions` already tolerates those rows (`checks_pass`), and this
/// relock step honors the same exemption rather than treating it as something to fix now.
pub(super) fn relock_cargo_lockfiles(canonical: &str) {
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tracked = tracked_paths_under(&workspace_root);
    if tracked.is_none() {
        warn!(
            "version-sync: cannot determine which files are git-tracked (not a git work tree, or `git` is \
             unavailable) — lockfile relock falls back to an unfiltered disk walk and may touch build-staging \
             copies"
        );
    }
    for lock in discover_cargo_locks(&workspace_root, canonical, tracked.as_ref()) {
        if lock.blocked_on_publish.is_some() {
            debug!(lock = %lock.path.display(), "version-sync: skipping relock — blocked on publish");
            continue;
        }
        let Some(dir) = lock.path.parent() else {
            continue;
        };
        info!("Relocking {} after version sync", lock.path.display());
        // Best-effort, matching every other lockfile-refresh command `sync_versions` runs (`pnpm
        // install`, `composer update`, `mix deps.get`): `relock_one`'s own `warn!` already
        // surfaced the failure, so there is nothing further to do here beyond letting version
        // sync continue with the rest of the workspace. See `relock_one`'s doc for why this
        // caller in particular must not escalate a resolver failure into a hard error. ~keep
        let _ = relock_one(dir, &lock.path, None);
    }
}

/// Retry a relock for every discovered lock still reporting `blocked_on_publish`, regardless of
/// whether this `sync_versions` invocation itself rewrote any manifest.
///
/// ~keep alef #1528: [`relock_cargo_lockfiles`] above only runs when the caller's own
/// `any_cargo_toml_modified` is true, and `blocked_on_publish` is re-derived fresh, on every
/// call, purely from whatever the *current* lock and manifest disagree on -- it has no memory of
/// when that disagreement first appeared. Skipping it at bump time is correct (an offline update
/// cannot resolve a version that has not published yet), but that correctness is also the trap:
/// once the bump run's own manifest write lands, every later `sync_versions` call — including the
/// one `alef generate` fires automatically on every ordinary regen — finds nothing left to change
/// and never calls `relock_cargo_lockfiles` again, so a lock left `blocked_on_publish` the day it
/// was bumped stays reported that way forever, long after the release it was waiting on actually
/// published. This measured against four consumer repos: `test_apps/rust`'s own self-dependency
/// pin was the one stale directory every affected repo shared. This pass is unconditional
/// specifically to close that gap: cheap when nothing is blocked (one discovery walk, zero
/// `cargo` invocations), and a genuine retry — not a second permanent skip — for anything that
/// is. [`relock_one`]'s existing best-effort handling already absorbs a lock that is still,
/// correctly, unresolvable.
pub(super) fn retry_blocked_lockfiles(canonical: &str) {
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let tracked = tracked_paths_under(&workspace_root);
    for lock in discover_cargo_locks(&workspace_root, canonical, tracked.as_ref()) {
        let Some(waiting_on) = lock.blocked_on_publish.as_deref() else {
            continue;
        };
        let Some(dir) = lock.path.parent() else {
            continue;
        };
        info!(
            lock = %lock.path.display(),
            waiting_on,
            "version-sync: retrying relock for a lock previously blocked on a pending release"
        );
        // Best-effort, same rationale as `relock_cargo_lockfiles` above. ~keep
        let _ = relock_one(dir, &lock.path, Some(waiting_on));
    }
}

/// Relock the `Cargo.lock` sitting beside a nested, alef-generated `Cargo.toml` (a Ruby, R, or
/// Elixir native-extension manifest -- never a root workspace member) immediately after this
/// write actually changed that manifest's content on disk, and confirm the refresh actually
/// worked before reporting success.
///
/// [`relock_cargo_lockfiles`] above only ever runs from `sync_versions`, the version-bump
/// pipeline. But a nested binding-crate manifest is `generated_header: true` and gets rewritten
/// on every ordinary `alef build`/`alef generate`/`alef scaffold` too -- completely independent
/// of a version bump, whenever a dependency constraint in it changes (a template dependency
/// version bump, an added feature, a config edit). Nothing relocked the sibling lockfile on
/// that path, so `cargo check --locked` against the freshly regenerated manifest could fail
/// immediately with no version bump involved at all -- the manifest widened or tightened a
/// requirement the lockfile's existing pin no longer satisfies. Scoped to `changed_paths`
/// (never a full-tree walk like `relock_cargo_lockfiles`) so a routine build only ever pays for
/// the manifests it actually rewrote, not every lockfile in the repo. ~keep
///
/// ~keep alef #A9: `relock_one`'s own resolver failure used to be the end of the story -- a
/// `warn!` this function had no way to see, let alone act on. That let a manifest change land on
/// disk with the exact disagreement `cargo metadata --locked`/`cargo build --locked` would go on
/// to fail on, while this function returned as if nothing had happened. Re-running the same
/// `stale_lock_findings`-backed check `alef generate`'s own post-write freshness gate uses --
/// rather than trusting `relock_one`'s exit code alone -- is what makes this non-vacuous: `cargo
/// update` exiting 0 is not proof the specific requirement this manifest just changed is now
/// satisfied, only that *some* resolution succeeded. Tolerating-variant, not the plain check: a
/// manifest's own dual-form self-dependency on the crate under release (`{ version = "...", path
/// = "..." }`) resolves locally via `path` regardless of publish status, so this should rarely
/// engage in practice for a native-extension manifest, but a hand-added registry-only edge must
/// still be exempted the same way `alef generate`'s own check exempts it. `canonical` is threaded
/// through by [`super::generate::reconcile_managed_scaffold_manifests`], the only production
/// caller that has a resolved crate version on hand; every other caller passes `None` and keeps
/// today's best-effort contract by not propagating this `Result` at all. ~keep
pub(super) fn relock_lockfiles_beside_changed_manifests(
    changed_paths: &HashSet<PathBuf>,
    workspace_root: &Path,
    canonical: Option<&str>,
) -> anyhow::Result<()> {
    // ~keep Every changed manifest is relocked and checked before any failure is reported, rather
    // than returning on the first stale one. The incident this fix exists for had THREE stale
    // locks in one tree (`e2e/rust`, the Elixir NIF, and the Ruby extension); a first-failure
    // return would have relocked one, named one, and left the other two untouched -- so the
    // operator fixes one, re-runs a full generate, meets the second, and pays for the same round
    // trip three times, having been told each time that there was one problem. A partial report
    // that reads like a complete one is the same defect class this whole check exists to remove.
    let mut failures: Vec<String> = Vec::new();
    for path in changed_paths {
        if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        let Some(dir) = path.parent() else {
            continue;
        };
        let lock_path = dir.join("Cargo.lock");
        if !lock_path.exists() {
            continue;
        }
        info!("Relocking {} after its generated manifest changed", lock_path.display());
        // `relock_one` already logged the specifics on failure; this caller's job is to find out
        // whether that failure -- or, in principle, an incomplete success -- left the manifest's
        // own requirement still unsatisfied, not to re-report the same failure a second way. ~keep
        let _ = relock_one(dir, &lock_path, None);
        let just_this_manifest: HashSet<PathBuf> = std::iter::once(path.clone()).collect();
        if let Some(error) =
            check_generated_lock_freshness_tolerating_pending_publish(&just_this_manifest, workspace_root, canonical)
        {
            failures.push(format!(
                "relocking {} after {} changed did not leave it satisfying the manifest: {error:#}",
                lock_path.display(),
                path.display()
            ));
        }
    }

    if failures.is_empty() {
        return Ok(());
    }
    // Sorted so the same three stale locks produce the same message every run: `changed_paths` is
    // a `HashSet`, whose iteration order varies between runs, and an error text that reshuffles
    // itself is one a reader cannot diff against the previous run. ~keep
    failures.sort();
    Err(anyhow::anyhow!(
        "{} generated lockfile(s) are still unsatisfied after relocking:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    ))
}

/// ~keep A generated Dart e2e manifest can stay byte-identical while its generated path
/// dependency changes an exact transitive pin. Refresh when the manifest changed or its lock no
/// longer satisfies a declared pin; changed-path filtering alone cannot see this dependency edge.
///
/// ~keep alef #A6: stdout/stderr are captured (`.output()`), never inherited (`.status()`), and
/// attributed through [`log_dart_relock_output`] instead. `dart pub get`'s own routine chatter
/// (e.g. "N packages have newer versions incompatible with dependency constraints") otherwise
/// lands directly in alef's log with no level prefix, reading as alef's own unattributed output.
pub(super) fn relock_dart_lockfiles_beside_generated_manifests(
    files: &[GeneratedFile],
    base_dir: &Path,
    changed_paths: &HashSet<PathBuf>,
) {
    relock_dart_lockfiles_with(files, base_dir, changed_paths, |directory, mode| {
        let output = std::process::Command::new("dart")
            .args(dart_relock_args(mode))
            .current_dir(directory)
            .output()?;
        log_dart_relock_output(directory, mode, &output);
        Ok(CargoStatus::from_exit_status(output.status))
    });
}

/// Attribute `dart pub get`'s own stdout/stderr into alef's log rather than letting it inherit
/// alef's stdio raw. Logged at `debug` on success (routine third-party chatter an operator did
/// not ask to see) and `warn` on failure, where the existing failure-path logging below already
/// names the directory and command.
fn log_dart_relock_output(directory: &Path, mode: DartRelockMode, output: &std::process::Output) {
    let level_log = |stream: &str, text: &str| {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if output.status.success() {
            debug!(directory = %directory.display(), ?mode, stream, "{text}");
        } else {
            warn!(directory = %directory.display(), ?mode, stream, "{text}");
        }
    };
    level_log("stdout", &String::from_utf8_lossy(&output.stdout));
    level_log("stderr", &String::from_utf8_lossy(&output.stderr));
}

fn relock_dart_lockfiles_with<F>(files: &[GeneratedFile], base_dir: &Path, changed_paths: &HashSet<PathBuf>, mut run: F)
where
    F: FnMut(&Path, DartRelockMode) -> std::io::Result<CargoStatus>,
{
    let declared_package_versions = declared_dart_package_versions(files);
    let directories: HashSet<PathBuf> = files
        .iter()
        .filter(|file| file.path.file_name().and_then(|name| name.to_str()) == Some("pubspec.yaml"))
        .filter_map(|file| base_dir.join(&file.path).parent().map(Path::to_path_buf))
        .filter(|directory| directory.join("pubspec.lock").is_file())
        .filter(|directory| {
            changed_paths.contains(&directory.join("pubspec.yaml")) || dart_lock_has_stale_declared_pin(directory)
        })
        .collect();

    for directory in directories {
        if dart_lock_blocked_on_publish(&directory, &declared_package_versions) {
            debug!(
                directory = %directory.display(),
                "version-sync: skipping Dart relock — blocked on publish"
            );
            continue;
        }
        info!(directory = %directory.display(), "Relocking Dart dependencies for generated pubspec");
        match attempt_dart_relock_with(|mode| run(&directory, mode)) {
            Ok(DartRelockMode::Offline) => {}
            Ok(DartRelockMode::Online) => info!(
                directory = %directory.display(),
                "Relocked Dart dependencies online after the offline attempt failed"
            ),
            Err(DartRelockFailure::OfflineCommand(error)) => {
                warn!(directory = %directory.display(), %error, "could not run dart pub get; pubspec.lock may be stale");
            }
            Err(DartRelockFailure::OnlineCommand { offline_code, error }) => warn!(
                directory = %directory.display(),
                ?offline_code,
                %error,
                "dart pub get failed offline and the online retry could not start; pubspec.lock may be stale"
            ),
            Err(DartRelockFailure::BothResolvers {
                offline_code,
                online_code,
            }) => warn!(
                directory = %directory.display(),
                ?offline_code,
                ?online_code,
                "dart pub get failed offline and online; pubspec.lock may be stale"
            ),
        }
    }
}

/// One `name = req` pin declared (directly, or through resolving a `path:` dependency's own
/// version) by a Dart manifest, paired with whether it came from that path resolution -- a path
/// dependency resolves locally and can never be blocked on a registry publish, so
/// [`dart_lock_blocked_on_publish`] must never exempt one.
struct DartPin {
    name: String,
    requirement: String,
    from_path: bool,
}

/// `name -> version` for every Dart package this run generated a `pubspec.yaml` for -- the Dart
/// analogue of `cargo_manifest_versions`/`registry_self_dependency`, built without threading
/// `ResolvedCrateConfig` through this call: whatever `pubspec.yaml`s this run actually wrote
/// already carry exactly the "this workspace's own package, this workspace's own current
/// version" pairs a pending-publish check needs.
fn declared_dart_package_versions(files: &[GeneratedFile]) -> HashMap<String, String> {
    let mut versions = HashMap::new();
    for file in files {
        if file.path.file_name().and_then(|name| name.to_str()) != Some("pubspec.yaml") {
            continue;
        }
        let Ok(document) = serde_saphyr::from_str::<serde_json::Value>(&file.content) else {
            continue;
        };
        let (Some(name), Some(version)) = (
            document.get("name").and_then(serde_json::Value::as_str),
            document.get("version").and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        versions.insert(name.to_string(), version.to_string());
    }
    versions
}

/// Every declared pin in `directory`'s `pubspec.yaml` the sibling `pubspec.lock` does not
/// currently satisfy -- the shared read behind both [`dart_lock_has_stale_declared_pin`] and
/// [`dart_lock_blocked_on_publish`].
fn stale_dart_pins(directory: &Path) -> Vec<DartPin> {
    let Ok(lock_text) = std::fs::read_to_string(directory.join("pubspec.lock")) else {
        return Vec::new();
    };
    let Ok(lock) = serde_saphyr::from_str::<serde_json::Value>(&lock_text) else {
        return Vec::new();
    };
    let Some(packages) = lock.get("packages").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    declared_dart_pins(&directory.join("pubspec.yaml"), &mut HashSet::new())
        .into_iter()
        .filter(|pin| {
            let locked = packages
                .get(&pin.name)
                .and_then(|package| package.get("version"))
                .and_then(serde_json::Value::as_str);
            locked.is_none_or(|version| !dart_version_matches(&pin.requirement, version))
        })
        .collect()
}

fn dart_lock_has_stale_declared_pin(directory: &Path) -> bool {
    !stale_dart_pins(directory).is_empty()
}

/// Whether every currently-stale pin in `directory`'s Dart lock is fully explained by this
/// workspace's own pending, not-yet-published release -- the Dart sibling of Cargo's
/// `blocked_on_publish`/[`relock_cargo_lockfiles`] pre-check and Node's `registry_self_dependency`
/// tolerance.
///
/// ~keep alef #A6 (tslp incident): `test_apps/dart/pubspec.yaml` pins
/// `tree_sitter_language_pack: 1.16.1` as a plain registry dependency -- not yet published to
/// pub.dev -- so `dart pub get` fails identically offline and online with "version solving
/// failed", both attempts wasted on a state this run already knows is expected and temporary.
/// `dart pub get` resolves a manifest's whole dependency graph in one pass, so ANY unresolvable
/// pin blocks every other pin in the same file too; a pin only counts as blocked here when it is
/// NOT derived from a `path:` dependency (which resolves locally and is never blocked on a
/// registry) and its declared `(name, requirement)` exactly matches one of `declared_package_versions`
/// -- this workspace's own package, at its own current version.
fn dart_lock_blocked_on_publish(directory: &Path, declared_package_versions: &HashMap<String, String>) -> bool {
    let stale = stale_dart_pins(directory);
    !stale.is_empty()
        && stale
            .iter()
            .all(|pin| !pin.from_path && declared_package_versions.get(&pin.name) == Some(&pin.requirement))
}

fn declared_dart_pins(path: &Path, visited: &mut HashSet<PathBuf>) -> Vec<DartPin> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(document) = serde_saphyr::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let mut pins = Vec::new();
    for bucket in ["dependencies", "dev_dependencies"] {
        append_dart_dependency_pins(&document, bucket, path, visited, &mut pins);
    }
    pins
}

fn append_dart_dependency_pins(
    document: &serde_json::Value,
    bucket: &str,
    manifest: &Path,
    visited: &mut HashSet<PathBuf>,
    pins: &mut Vec<DartPin>,
) {
    let Some(dependencies) = document.get(bucket).and_then(serde_json::Value::as_object) else {
        return;
    };
    for (name, specification) in dependencies {
        if let Some(requirement) = specification.as_str() {
            if requirement != "any" {
                pins.push(DartPin {
                    name: name.to_string(),
                    requirement: requirement.to_string(),
                    from_path: false,
                });
            }
            continue;
        }
        append_dart_path_dependency_pins(name, specification, manifest, visited, pins);
    }
}

fn append_dart_path_dependency_pins(
    name: &str,
    specification: &serde_json::Value,
    manifest: &Path,
    visited: &mut HashSet<PathBuf>,
    pins: &mut Vec<DartPin>,
) {
    let Some(relative) = specification.get("path").and_then(serde_json::Value::as_str) else {
        return;
    };
    let dependency_manifest = manifest
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(relative)
        .join("pubspec.yaml");
    if let Ok(dependency_text) = std::fs::read_to_string(&dependency_manifest)
        && let Ok(dependency) = serde_saphyr::from_str::<serde_json::Value>(&dependency_text)
        && let Some(version) = dependency.get("version").and_then(serde_json::Value::as_str)
    {
        pins.push(DartPin {
            name: name.to_string(),
            requirement: version.to_string(),
            from_path: true,
        });
    }
    pins.extend(declared_dart_pins(&dependency_manifest, visited));
}

fn dart_version_matches(requirement: &str, locked: &str) -> bool {
    let Ok(locked) = semver::Version::parse(locked) else {
        return false;
    };
    if let Ok(exact) = semver::Version::parse(requirement) {
        return exact == locked;
    }
    semver::VersionReq::parse(requirement).map_or(true, |constraint| constraint.matches(&locked))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DartRelockMode {
    Offline,
    Online,
}

#[derive(Debug)]
enum DartRelockFailure {
    OfflineCommand(std::io::Error),
    OnlineCommand {
        offline_code: Option<i32>,
        error: std::io::Error,
    },
    BothResolvers {
        offline_code: Option<i32>,
        online_code: Option<i32>,
    },
}

fn attempt_dart_relock_with<F>(mut run: F) -> Result<DartRelockMode, DartRelockFailure>
where
    F: FnMut(DartRelockMode) -> std::io::Result<CargoStatus>,
{
    let offline = run(DartRelockMode::Offline).map_err(DartRelockFailure::OfflineCommand)?;
    if offline.successful {
        return Ok(DartRelockMode::Offline);
    }
    let online = run(DartRelockMode::Online).map_err(|error| DartRelockFailure::OnlineCommand {
        offline_code: offline.code,
        error,
    })?;
    if online.successful {
        Ok(DartRelockMode::Online)
    } else {
        Err(DartRelockFailure::BothResolvers {
            offline_code: offline.code,
            online_code: online.code,
        })
    }
}

fn dart_relock_args(mode: DartRelockMode) -> &'static [&'static str] {
    match mode {
        DartRelockMode::Offline => &["pub", "get", "--offline"],
        DartRelockMode::Online => &["pub", "get"],
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelockMode {
    Offline,
    Online,
}

#[derive(Clone, Copy, Debug)]
struct CargoStatus {
    successful: bool,
    code: Option<i32>,
}

impl CargoStatus {
    fn from_exit_status(status: std::process::ExitStatus) -> Self {
        Self {
            successful: status.success(),
            code: status.code(),
        }
    }

    #[cfg(test)]
    fn success() -> Self {
        Self {
            successful: true,
            code: Some(0),
        }
    }

    #[cfg(test)]
    fn failed(code: Option<i32>) -> Self {
        Self {
            successful: false,
            code,
        }
    }
}

#[derive(Debug)]
enum RelockFailure {
    OfflineCommand(std::io::Error),
    OnlineCommand {
        offline_code: Option<i32>,
        error: std::io::Error,
    },
    BothResolvers {
        offline_code: Option<i32>,
        online_code: Option<i32>,
    },
}

fn attempt_relock_with<F>(mut run: F) -> Result<RelockMode, RelockFailure>
where
    F: FnMut(RelockMode) -> std::io::Result<CargoStatus>,
{
    let offline = run(RelockMode::Offline).map_err(RelockFailure::OfflineCommand)?;
    if offline.successful {
        return Ok(RelockMode::Offline);
    }

    let online = run(RelockMode::Online).map_err(|error| RelockFailure::OnlineCommand {
        offline_code: offline.code,
        error,
    })?;
    if online.successful {
        return Ok(RelockMode::Online);
    }

    Err(RelockFailure::BothResolvers {
        offline_code: offline.code,
        online_code: online.code,
    })
}

fn relock_args(mode: RelockMode) -> &'static [&'static str] {
    match mode {
        RelockMode::Offline => &["update", "--offline", "-w"],
        RelockMode::Online => &["update", "-w"],
    }
}

/// Best-effort, like the other lockfile-refresh commands `sync_versions` already runs
/// (`pnpm install`, `composer update`, `mix deps.get`): a missing `cargo` binary or a lockfile
/// that cannot resolve must not abort the whole version sync. Try the local registry cache first,
/// then retry with registry access when that cache cannot satisfy a newly generated constraint.
///
/// `waiting_on` is `Some("name@version")` only when the caller already knows, before calling,
/// that this lock is [`retry_blocked_lockfiles`]'s deliberate retry of a lock still waiting on
/// its own pending release -- never a fresh, previously-unexamined lock. ~keep alef #A7: both
/// resolvers failing is the EXPECTED outcome for that retry, not a surprise, so it is logged at
/// `info` ("still waiting on ...") instead of the loud `warn` this function otherwise emits, which
/// names a `cargo check --locked` remedy the caller cannot run until the release publishes. Every
/// other caller passes `None`: a lock nothing already flagged as blocked failing both resolvers
/// is still a genuinely unexplained drift and must stay loud.
///
/// Returns `Err` for every failure branch below except the known-pending-publish retry, which
/// stays `Ok` -- that outcome is expected and temporary, not a defect. ~keep alef #A9: this
/// function used to return nothing at all, so every caller's `relock_one(...)` was a statement,
/// not an expression -- there was no way to tell a resolver failure from a success without
/// re-parsing this function's own log output. [`relock_lockfiles_beside_changed_manifests`] is
/// the one caller that now acts on this `Result`; the version-sync callers above intentionally
/// keep discarding it (`let _ = ...`), preserving their own documented best-effort contract.
fn relock_one(dir: &Path, lock_path: &Path, waiting_on: Option<&str>) -> anyhow::Result<()> {
    let outcome = attempt_relock_with(|mode| {
        std::process::Command::new("cargo")
            .args(relock_args(mode))
            .current_dir(dir)
            .status()
            .map(CargoStatus::from_exit_status)
    });

    match outcome {
        Ok(RelockMode::Offline) => Ok(()),
        Ok(RelockMode::Online) => {
            info!(
                lock = %lock_path.display(),
                "Relocked with registry access after the offline attempt failed"
            );
            Ok(())
        }
        Err(RelockFailure::OfflineCommand(error)) => {
            warn!(
                lock = %lock_path.display(),
                %error,
                "could not run cargo update for this lockfile; it may still be stale against its manifest"
            );
            Err(anyhow::anyhow!(
                "could not run cargo update for {}: {error}",
                lock_path.display()
            ))
        }
        Err(RelockFailure::OnlineCommand { offline_code, error }) => {
            warn!(
                lock = %lock_path.display(),
                ?offline_code,
                %error,
                "cargo update failed offline, then the registry-enabled retry could not run; the lockfile may \
                 still be stale against its manifest"
            );
            Err(anyhow::anyhow!(
                "cargo update failed offline (exit {offline_code:?}) for {}, and the registry-enabled retry \
                 could not run: {error}",
                lock_path.display()
            ))
        }
        Err(RelockFailure::BothResolvers {
            offline_code,
            online_code,
        }) => {
            if let Some(waiting_on) = waiting_on {
                info!(
                    lock = %lock_path.display(),
                    waiting_on,
                    ?offline_code,
                    ?online_code,
                    "still waiting on {waiting_on} to publish; the lockfile cannot resolve until then"
                );
                Ok(())
            } else {
                warn!(
                    lock = %lock_path.display(),
                    ?offline_code,
                    ?online_code,
                    "cargo update -w failed both offline and with registry access; the lockfile may still be \
                     stale against its manifest. Resolve the dependency conflict in that directory before \
                     running `cargo check --locked`"
                );
                Err(anyhow::anyhow!(
                    "cargo update -w failed both offline (exit {offline_code:?}) and with registry access (exit \
                     {online_code:?}) for {}",
                    lock_path.display()
                ))
            }
        }
    }
}

/// Whether `finding` is explained by a lock [`discover_cargo_locks`] already reports
/// `blocked_on_publish`: the disagreement is exactly this crate's own pending, not-yet-published
/// version, the one case `validate_versions::checks_pass` already tolerates. Any OTHER
/// disagreement reachable from the same lock -- a stale third-party pin reached through a
/// hand-written path dependency, [`super::lock_freshness`]'s own founding `tower-http` incident --
/// is not explained by this and must still fail the gate.
pub(super) fn explained_by_pending_publish(finding: &StaleLockFinding, blocked: &HashMap<PathBuf, String>) -> bool {
    let Some(waiting_on) = blocked.get(&finding.lock) else {
        return false;
    };
    waiting_on.split('@').next() == Some(finding.dependency.as_str())
}

/// Fail `alef validate versions` -- the release gate the `consumer-release-gates` skill has every
/// consumer run before tagging -- on a committed `Cargo.lock` that cannot resolve a requirement
/// reachable from a manifest alef generated, unless that exact disagreement is this release's own
/// pending version still waiting to publish.
///
/// ~keep alef #1528: [`super::check_generated_lock_freshness`] already detects this drift
/// correctly (consumers praised it), but it only ever runs from inside `alef generate`/`alef
/// all`. A drift whose cause lives entirely in a hand-written dependency alef neither generates
/// nor watches (see [`super::lock_freshness`]'s module doc -- the `tower-http` incident this
/// module was built for) never touches a byte alef owns, so nothing prompts anyone to re-run a
/// regen before cutting the next release, and the diagnostic never gets a chance to fire: the
/// affected consumer repos tagged and pushed with the stale lock already committed, and `cargo
/// build --locked` was the first thing to notice, in CI, after the fact. Reusing the exact same
/// read-only `stale_lock_findings` here -- never a second, independently-derived lock-freshness
/// rule -- means the two call sites can only ever agree on what counts as stale; the sole
/// difference is that a pending-release row (which `stale_lock_findings` alone cannot tell apart
/// from a genuinely abandoned one) is cross-checked against `discover_cargo_locks`'s
/// `blocked_on_publish` and tolerated here, exactly as `validate_versions::checks_pass` already
/// tolerates it.
pub(crate) fn check_release_lock_freshness(workspace_root: &Path, canonical: &str) -> Option<anyhow::Error> {
    let tracked = tracked_paths_under(workspace_root);
    let blocked: HashMap<PathBuf, String> = discover_cargo_locks(workspace_root, canonical, tracked.as_ref())
        .into_iter()
        .filter_map(|lock| lock.blocked_on_publish.map(|waiting_on| (lock.path, waiting_on)))
        .collect();

    let mut manifest_dirs: HashSet<PathBuf> = HashSet::new();
    for path in collect_alef_headered_paths(workspace_root) {
        if path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            continue;
        }
        if let Some(dir) = path.parent() {
            manifest_dirs.insert(dir.to_path_buf());
        }
    }

    let mut findings: Vec<StaleLockFinding> = Vec::new();
    for dir in &manifest_dirs {
        findings.extend(
            stale_lock_findings(dir)
                .into_iter()
                .filter(|finding| !explained_by_pending_publish(finding, &blocked)),
        );
    }
    if findings.is_empty() {
        return None;
    }
    Some(anyhow::anyhow!(release_lock_message(&findings)))
}

/// Render the operator-facing failure, mirroring [`super::lock_freshness`]'s own message shape
/// (dependency, requirement, lock, remedy) so the two checks read as one family rather than two
/// independently-worded errors for the same underlying defect.
///
/// Reported, never rewritten: generation itself succeeded and alef does not author lockfiles,
/// so the fix is a command the operator runs in their own tree. ~keep
fn release_lock_message(findings: &[StaleLockFinding]) -> String {
    let mut message = format!(
        "{} committed Cargo.lock pin(s) cannot satisfy a requirement from a manifest alef generated, and \
         not because of this release's own not-yet-published version; `cargo metadata --locked` and `cargo \
         build --locked` will fail in these directories once this release is tagged and pushed:",
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
    message
}

#[cfg(test)]
#[path = "version_lockfiles_tests.rs"]
mod tests;

// Declared here rather than nested under `lock_freshness_tests.rs`/`lock_freshness.rs`: both of
// those are already over the 1,000-line file-modularization cap and must not grow further; this
// module's fixtures only need `check_generated_lock_freshness_tolerating_pending_publish` and
// `explained_by_pending_publish`, both already in scope here. ~keep
#[cfg(test)]
#[path = "lock_freshness_pending_publish_collision_tests.rs"]
mod pending_publish_collision_tests;
