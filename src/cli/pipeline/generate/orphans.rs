use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Conventional package roots that hold TypeScript-facing output for the
/// wasm/node targets but are not returned by `package_dir` (which resolves to
/// each target's Rust crate directory). They are swept only on unfiltered runs
/// so orphaned TS artifacts are reclaimed without a per-target directory.
const UNFILTERED_TS_ROOTS: [&str; 2] = ["packages/wasm", "packages/typescript"];

/// Candidate roots for `sweep_orphans` in a `generate` / `all` run.
///
/// The returned paths are candidates only — callers filter out non-existent
/// directories before sweeping (kept out of this function so it stays pure and
/// unit-testable).
///
/// Unfiltered runs (`filtered == false`, i.e. no `--lang`) sweep every
/// configured language's output directory plus the conventional
/// wasm/typescript package roots, so orphans anywhere in the binding tree are
/// reclaimed. The keep set then contains every language's files, so nothing
/// valid is deleted.
///
/// Filtered runs (`--lang <subset>`) must NOT touch output belonging to
/// languages outside the subset. On a filtered run the keep set only contains
/// the requested languages' files, so sweeping another language's root would
/// delete its still-valid generated output (the data-loss bug in #178).
/// Filtered runs therefore restrict roots to the requested languages' own
/// directories and skip the unconditional wasm/typescript roots. Orphans in an
/// unrequested language's tree are reclaimed on the next unfiltered run.
pub fn generate_sweep_roots(
    languages: &[Language],
    filtered: bool,
    config: &ResolvedCrateConfig,
    base_dir: &Path,
) -> Vec<PathBuf> {
    let mut roots: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for &lang in languages {
        roots.insert(base_dir.join(config.package_dir(lang)));
        if let Some(out) = config.output_for(&lang.to_string()) {
            roots.insert(base_dir.join(out));
        }
    }
    if !filtered {
        for root in UNFILTERED_TS_ROOTS {
            roots.insert(base_dir.join(root));
        }
    }
    roots.into_iter().collect()
}

pub fn targeted_e2e_sweep_roots(
    output_paths: &[PathBuf],
    e2e_output_root: &Path,
    snippet_output_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = std::collections::BTreeSet::new();
    for path in output_paths {
        if snippet_output_root.is_some_and(|snippet_root| path.starts_with(snippet_root)) {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(e2e_output_root) {
            let mut components = relative.components();
            let Some(language) = components.next() else {
                continue;
            };
            let Some(owned_subtree) = components.next() else {
                continue;
            };
            if components.next().is_none() {
                continue;
            }
            roots.insert(
                e2e_output_root
                    .join(language.as_os_str())
                    .join(owned_subtree.as_os_str()),
            );
        }
    }
    roots.into_iter().collect()
}

/// Delete alef-generated files under `roots` whose absolute path is not
/// present in `keep`. A file is considered alef-owned only when it has both a
/// recognized Alef marker near its start and an `alef:hash:` line. User code,
/// fixtures, scaffolded manifests, and lockfiles are left untouched.
///
/// This sweeps orphans left behind when categories or fixtures are removed
/// from the generation set (e.g. a category that produced 0 test functions
/// for the current binding surface). Without this pass, those files linger
/// on disk with stale `alef:hash:` headers and `alef verify` reports them
/// as stale forever.
///
/// Empty parent directories left behind after deletion are removed in a
/// best-effort second pass.
pub fn sweep_orphans(
    roots: &[std::path::PathBuf],
    keep: &std::collections::HashSet<std::path::PathBuf>,
) -> anyhow::Result<usize> {
    let mut removed = 0usize;
    let mut touched_dirs: std::collections::BTreeSet<std::path::PathBuf> = std::collections::BTreeSet::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(it) => it,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if file_type.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if matches!(
                        name,
                        ".git"
                            | "target"
                            | "node_modules"
                            | "vendor"
                            | "_build"
                            | "deps"
                            | ".venv"
                            | "venv"
                            | "build"
                            | "dist"
                            | "Pods"
                    ) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if keep.contains(&path) {
                    continue;
                }
                if !path_is_alef_owned(&path) {
                    continue;
                }
                if let Err(err) = std::fs::remove_file(&path) {
                    debug!("  sweep skip (remove failed): {} ({err})", path.display());
                    continue;
                }
                debug!("  swept orphan: {}", path.display());
                if let Some(parent) = path.parent() {
                    touched_dirs.insert(parent.to_path_buf());
                }
                removed += 1;
            }
        }
    }
    let mut dirs: Vec<_> = touched_dirs.into_iter().collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for dir in dirs {
        let _ = std::fs::remove_dir(&dir);
    }
    if removed > 0 {
        info!("Swept {removed} orphan generated file(s)");
    }
    Ok(removed)
}

/// Delete alef-emitted files listed in `previous_paths` whose absolute path is
/// no longer present in `keep`, restricted to `allowed_roots`; then, for
/// `disk_scan_roots` only, additionally reclaim any alef-marked file found on
/// disk under those roots that is absent from `keep` — with no dependency on
/// `previous_paths` at all.
///
/// A path from `previous_paths` is reclaimed by either of two independent
/// routes (see [`path_is_reclaimable`]): content that still carries alef's
/// `alef:hash:` marker (the original, content-based check — unchanged), or
/// membership in the narrow, explicit [`UNMARKABLE_ALEF_MANIFESTS`] allowlist
/// for manifests that structurally cannot carry a marker at all (plain-JSON
/// files such as `composer.json`/`package.json`). For that second route,
/// `previous_paths` membership itself is the ownership evidence: every caller
/// sources it from alef's own cache (`{stage}.manifest` / `{lang}.manifest`),
/// which nothing but alef's own writers ever populate, so reaching this
/// function with a matching path already proves alef wrote it on the run that
/// produced that manifest.
///
/// Reclaiming an unmarkable manifest also reclaims its package-manager-owned
/// lockfile sibling in the same directory (`composer.lock`, `pnpm-lock.yaml`,
/// ...) if present — see [`reclaim_lockfile_siblings`]. Alef never authors a
/// lockfile itself, so a lockfile is *never* reclaimed on its own provenance,
/// only as a cascade of its manifest being reclaimed first. The disk-scan route
/// below never triggers this cascade: it has no manifest-reclaim event to
/// cascade from, and a lockfile never carries a marker of its own.
///
/// This sweeps orphans left behind when categories or fixtures are removed from
/// the generation set, and when a scaffold manifest's emit condition changes
/// (e.g. a co-located/split layout toggle stops emitting a second
/// `composer.json`). Without the unmarkable-manifest route, such files linger on
/// disk forever, since `content_is_alef_owned` can never match a file that never
/// carried a marker in the first place.
///
/// The `previous_paths` route is bounded by bookkeeping: a path that fell out of
/// every manifest written since alef stopped emitting it can never appear in
/// `previous_paths` again, and is therefore permanently unreachable by that route
/// alone, however correct the bookkeeping is from this point forward. The
/// `disk_scan_roots` route exists for exactly that already-lost case: the
/// `alef:hash:` marker on disk is itself durable ownership proof and needs no
/// history. Because it has no bookkeeping fallback, a caller must only place a
/// root in `disk_scan_roots` when it can prove `keep` is *complete* for
/// everything currently on disk under that root this run — e.g. a language
/// `pipeline::generate` actually regenerated this run, not one its per-language
/// cache skipped, nor one left ungenerated by a partial-failure run (see
/// `generation.rs`'s `is_lang_cached` skip and `all_commands.rs`'s
/// `e2e_stage_error`/`docs_stage_error` deferred-failure handling for concrete
/// cases where `keep` under-counts). A root outside that guarantee must stay out
/// of `disk_scan_roots` — pass `&[]` to disable this route entirely, which
/// reproduces the exact prior behavior. Unlike the `previous_paths` route, this
/// one never consults [`UNMARKABLE_ALEF_MANIFESTS`]: a disk scan has no
/// `previous_paths`-membership evidence to substitute for the missing marker, so
/// an unmarked file is always left alone, however stale it looks.
///
/// A caller-supplied `disk_scan_roots` entry is still only a *candidate* --  this
/// function re-verifies, per root, that both `previous_paths` and `keep` actually
/// contain at least one entry under it before scanning. On a real tree this
/// caught a defect the caller-side "actually regenerated this run" contract alone
/// cannot see: several backends record only their own Rust crate source path in
/// this stage's bookkeeping and never their language-side generated tree at all,
/// so `previous_paths` (and in the same run, `keep`) can be non-empty overall
/// while containing zero entries under that specific root -- a root a scan would
/// otherwise treat as merely "nothing to reclaim" is in fact "this backend's
/// bookkeeping has never once vouched for anything here," which a marker alone
/// cannot tell apart from a real orphan. A root that fails this check is skipped
/// with a `warn!`, never scanned. Passing a candidate through this check also
/// requires the file to be **git-tracked** ([`git_tracked_paths_under`]): a build
/// tool can stage a copy of a real generated file at a mirrored path under the
/// same owned root (e.g. a gem-staging directory), and the copy inherits the
/// marker, so content is not sufficient on its own -- only alef's own committed
/// output is git-tracked, a disposable staged copy is not. ~keep
pub fn sweep_manifest_orphans(
    previous_paths: &[PathBuf],
    keep: &std::collections::HashSet<PathBuf>,
    allowed_roots: &[PathBuf],
    disk_scan_roots: &[PathBuf],
) -> anyhow::Result<usize> {
    let mut removed = 0;
    for path in previous_paths {
        if keep.contains(path) || !allowed_roots.iter().any(|root| path.starts_with(root)) || !path.is_file() {
            continue;
        }
        if !path_is_reclaimable(path) {
            continue;
        }
        std::fs::remove_file(path).with_context(|| format!("failed to remove orphan {}", path.display()))?;
        debug!("  swept manifest orphan: {}", path.display());
        removed += 1;
        removed += reclaim_lockfile_siblings(path, keep)?;
    }
    for root in disk_scan_roots {
        if !root.exists() {
            continue;
        }
        let manifest_entries_under_root = previous_paths.iter().filter(|path| path.starts_with(root)).count();
        let keep_entries_under_root = keep.iter().filter(|path| path.starts_with(root)).count();
        if manifest_entries_under_root == 0 || keep_entries_under_root == 0 {
            report_unscannable_root(
                root,
                manifest_entries_under_root,
                keep_entries_under_root,
                previous_paths.len(),
            );
            continue;
        }
        let Some(tracked) = git_tracked_paths_under(root) else {
            tracing::warn!(
                "disk-scan orphan reclaim skipped for {}: could not determine which files under this root are \
                 git-tracked (not inside a git work tree, or `git` is unavailable)",
                root.display()
            );
            continue;
        };
        // `collect_alef_headered_paths` refuses git-ignored paths but only guarantees the marker,
        // and it deliberately still returns untracked files -- it also serves
        // `finalize_hashes_sweeping`'s re-stamp pass, which must reach a file before the consumer
        // has committed it. Both gaps are closed here, because deletion answers to a stricter bar
        // than rewriting: `path_is_alef_owned` restores the marker-plus-hash requirement the rest
        // of this function reclaims by, so a file this very run wrote moments ago but has not yet
        // reached `finalize_hashes` is never mistaken for an orphan, and `tracked` narrows
        // not-ignored to actually-committed, so nothing uncommitted can be deleted. ~keep
        let candidates: Vec<_> = collect_alef_headered_paths(root)
            .into_iter()
            .filter(|path| !keep.contains(path) && tracked.contains(path) && path_is_alef_owned(path))
            .collect();
        if !candidates.is_empty() {
            report_disk_scan_candidates(root, &candidates);
        }
    }
    // The disk-scan loop above only inspects a root a caller opted into via `disk_scan_roots`,
    // and several callers (scaffold, README/docs, e2e, test_apps -- see their call sites) always
    // pass `&[]` there, deliberately, because that route's reclaim is a stronger, riskier
    // operation those stages do not want. But the underlying observation -- a root this run
    // recorded kept output under, with the previous-run manifest recording nothing under it at
    // all -- can mean two different things, and this loop must not render them identically
    // (alef-task #557's original version collapsed them, and fired on every legitimate first
    // run for a crate with no prior manifest bookkeeping at all -- see the `previous_paths.is_empty()`
    // branch below).
    //
    // `previous_paths` is the caller's FULL baseline for this call, not pre-filtered to any one
    // root: every production call site merges it from one or more `{stage}.manifest` /
    // `{lang}.manifest` files (`cache::read_stage_paths` / `read_lang_manifest` /
    // `read_scaffold_manifest`), each of which returns an empty `Vec` both when the file is
    // absent (no previous run ever recorded this stage/language) and when it is present but
    // records nothing -- those readers cannot and do not distinguish the two on their own (see
    // their doc comments). So `previous_paths.is_empty()` (checked across ALL roots, before the
    // per-root filter below) answers a strictly different question than
    // `manifest_entries_under_root == 0`: it is true only when NO manifest this call consulted
    // recorded a single path anywhere, which is exactly the first-run/migration state -- there
    // is no previous baseline to have a bookkeeping gap in. The real defect this warning exists
    // to catch (alef#158, alef-task #557) instead shows up as `previous_paths` recording entries
    // under OTHER roots while this one root is empty -- proof a previous manifest DID exist and
    // simply never covered this root. ~keep
    for root in allowed_roots {
        if disk_scan_roots.contains(root) {
            continue;
        }
        let manifest_entries_under_root = previous_paths.iter().filter(|path| path.starts_with(root)).count();
        if manifest_entries_under_root > 0 {
            continue;
        }
        let keep_entries_under_root = keep.iter().filter(|path| path.starts_with(root)).count();
        if keep_entries_under_root == 0 {
            continue;
        }
        if previous_paths.is_empty() {
            // No manifest consulted by this call recorded ANYTHING, under any root -- there is no
            // previous-run baseline at all, so this is not a bookkeeping gap, it is the expected
            // shape of a crate's first `alef generate`/`alef all` run under this bookkeeping (a
            // fresh checkout, a wiped `.alef/` cache, or truly the first run ever). `debug`, not
            // `warn`: this self-resolves the moment this run's own manifest is written. If the
            // SAME root still has zero manifest entries on the NEXT run (once `previous_paths` is
            // no longer globally empty), the `warn!` branch below fires instead.
            tracing::debug!(
                "no previous-run manifest for {}: this run recorded {keep_entries_under_root} kept file(s) under \
                 this root, and every manifest this call consulted has zero entries anywhere -- expected on a \
                 first run with no prior bookkeeping. Self-resolves once this run's manifest is written",
                root.display()
            );
            continue;
        }
        tracing::warn!(
            "orphan-reclaim bookkeeping gap for {}: this run recorded {keep_entries_under_root} kept file(s) \
             under this root, but the previous-run manifest recorded none. Orphan reclaim can never run here \
             (nothing to compare `keep` against), so a file a backend stops emitting under this root would \
             never be removed. Fix this root's manifest bookkeeping",
            root.display()
        );
    }
    // This function previously returned its count with no logging at any level, at any call
    // site (every caller discards the `Ok(usize)` through `?`), which made "did the sweep run
    // and find nothing" and "did the sweep not run at all" indistinguishable from the log --
    // the exact ambiguity that made a caller-side wiring defect (a previous-run baseline
    // clobbered before this function ever saw it) look, from the log alone, like this
    // function's own fault. ~keep
    if removed > 0 {
        info!("Swept {removed} manifest orphan(s)");
    }
    Ok(removed)
}

/// Explain, at the severity the evidence actually supports, why a caller-supplied
/// `disk_scan_roots` candidate was refused. **The refusal itself is identical either way** — a
/// root this function cannot vouch for is never scanned — only the classification differs.
///
/// `previous_total` is `previous_paths.len()` across ALL roots, before any per-root filter, and a
/// zero there answers a strictly different question than `manifest_entries_under_root == 0`.
/// Every production call site merges `previous_paths` from
/// `{stage}-ownership` manifests via `cache::read_stage_paths`, which returns an empty `Vec` both
/// when the manifest file is absent and when it is present but records nothing — the reader cannot
/// distinguish the two (see its doc). So a globally empty `previous_paths` means no manifest this
/// call consulted recorded a single path anywhere: there is no previous-run baseline for a
/// bookkeeping gap to exist in. That is the ordinary shape of `alef cache clear` followed by a
/// regen, a fresh checkout, or the first run after the `{stage}-ownership` manifests were
/// introduced — not a defect, and it fires for *every* existing root at once, which is the
/// signature to recognise it by.
///
/// It self-resolves on the next run, and the mechanism is the caller's, not this function's: both
/// disk-scan call sites (`bin_cli::all_commands`' `all-bindings-{lang}-ownership` and
/// `bin_cli::core_commands::generate`'s `generate-{lang}-ownership`) write that manifest back for
/// every configured language immediately after this call returns, deliberately after the sweep so
/// it never observes this run's own writes as "previous" state. Once written, `previous_paths` is
/// no longer globally empty on the following run, and a root that is *still* unrecorded then takes
/// the `warn!` branch below — which is exactly the real defect (a backend recording only its Rust
/// crate source path and never its own language-output tree) at unchanged severity.
///
/// This mirrors the same split `sweep_manifest_orphans`' `allowed_roots` loop already makes for its
/// "orphan-reclaim bookkeeping gap" warning. That loop got the carve-out and this one did not, so a
/// consumer's clean regen reported 20+ roots as untrustworthy backend bookkeeping when the only
/// thing missing was the cache they had just cleared. ~keep
fn report_unscannable_root(root: &Path, manifest_entries: usize, keep_entries: usize, previous_total: usize) {
    if previous_total == 0 {
        tracing::debug!(
            "disk-scan orphan reclaim skipped for {}: no previous-run manifest exists at all ({keep_entries} keep \
             entry(s) under this root this run, zero manifest entries anywhere). Expected after `alef cache \
             clear`, on a fresh checkout, or on the first run after the ownership manifests were introduced. \
             Self-resolves once this run writes its own ownership manifests, immediately after this sweep",
            root.display()
        );
        return;
    }
    tracing::warn!(
        "disk-scan orphan reclaim skipped for {}: {manifest_entries} manifest entry(s) and {keep_entries} keep \
         entry(s) under this root, though the previous-run manifest does exist and recorded {previous_total} \
         path(s) overall. A backend whose own bookkeeping has never vouched for anything under its output root \
         cannot tell a real orphan from output it never recorded, so a stale file here is left in place until \
         that backend's path tracking is fixed",
        root.display()
    );
}

/// True when `path` has enough ownership evidence to be reclaimed by
/// [`sweep_manifest_orphans`]. See that function's doc for the two routes.
///
/// Route 2 (unmarkable-manifest) intentionally does **not** inspect content, so
/// a hand-edit to e.g. `composer.json` at a path alef has decided to stop
/// writing is reclaimed right along with it — same as any other orphan. This is
/// a deliberate call, not an oversight: a manifest alef *still* writes this run
/// never reaches this function at all (the `keep.contains(path)` check in
/// `sweep_manifest_orphans` filters it out first), so the only files route 2 can
/// ever touch are ones alef's own current-run intent has already disowned. See
/// the accompanying report for the full trade-off discussion. ~keep
fn path_is_reclaimable(path: &Path) -> bool {
    let has_marker = std::fs::read_to_string(path).is_ok_and(|content| content_is_alef_owned(&content));
    has_marker || is_unmarkable_alef_manifest(path)
}

/// Manifest filenames alef scaffolds in a format that cannot carry a marker
/// comment at all (plain JSON: no `#`, no `//`), and therefore emits with
/// `generated_header: false` — see `scaffold_php`'s `composer.json` and
/// `scaffold_node`'s `package.json`. Restricting this list to formats that are
/// *structurally* incapable of the marker — rather than every
/// `generated_header: false` scaffold file — keeps this route from silently
/// widening to cover files that merely opted out of a marker for unrelated
/// reasons (e.g. `pubspec.yaml`, which is YAML and could carry a `#` marker).
/// Extend deliberately, one verified format at a time: check the candidate's
/// actual `generated_header` value and content format in
/// `src/scaffold/languages/*.rs` before adding it here.
///
/// **Scope: the orphan-reclaim path only.** An earlier version of this comment claimed the list
/// was "also consulted by `write.rs`'s `existing_file_is_alef_owned`", the never-overwrite guard.
/// No such function exists anywhere in this crate -- the only occurrence of that name was this
/// comment. The write guard consults [`crate::cli::cache::is_scaffold_owned_path`] instead, and
/// this list has no bearing on it. Corrected because the claim was quoted as if it described real
/// behaviour: a doc comment describing a mechanism that does not exist is worse than none, since
/// it is indistinguishable from a specification. ~keep
const UNMARKABLE_ALEF_MANIFESTS: &[&str] = &["composer.json", "package.json"];

pub(super) fn is_unmarkable_alef_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| UNMARKABLE_ALEF_MANIFESTS.contains(&name))
}

/// Manifest filename -> the lockfile name(s) a package manager writes beside it.
/// Alef never authors any of these lockfiles itself (they come from `composer
/// update`, `pnpm install`, `npm install`, `yarn install`, run either by a
/// developer directly or by alef's own built-in `lint`/`update` pipelines),
/// so a lockfile can never independently satisfy [`path_is_reclaimable`] — it
/// never carries a marker and never appears in `previous_paths` on its own
/// provenance. It is reclaimed *only* as a cascade of its manifest being
/// reclaimed: once alef confirms (by removing `composer.json` here) that it no
/// longer wants that manifest at a given path, a `composer.lock` beside it is
/// installing a manifest that no longer exists, so it goes with it. There is no
/// route by which this module reclaims a lockfile independently of its
/// manifest. ~keep
const MANIFEST_LOCKFILE_SIBLINGS: &[(&str, &[&str])] = &[
    ("composer.json", &["composer.lock"]),
    ("package.json", &["package-lock.json", "pnpm-lock.yaml", "yarn.lock"]),
];

fn reclaim_lockfile_siblings(manifest_path: &Path, keep: &std::collections::HashSet<PathBuf>) -> anyhow::Result<usize> {
    let Some(manifest_name) = manifest_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(0);
    };
    let Some(dir) = manifest_path.parent() else {
        return Ok(0);
    };
    let mut removed = 0;
    for entry in MANIFEST_LOCKFILE_SIBLINGS {
        if entry.0 != manifest_name {
            continue;
        }
        for &lockfile_name in entry.1 {
            let lockfile_path = dir.join(lockfile_name);
            if keep.contains(&lockfile_path) || !lockfile_path.is_file() {
                continue;
            }
            std::fs::remove_file(&lockfile_path)
                .with_context(|| format!("failed to remove orphan lockfile {}", lockfile_path.display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Collect every alef-headered file under `root` (recursively), skipping anything git ignores
/// and, on top of that, the dependency / build directories named below.
///
/// Used by the `all` pipeline to gather existing registry-mode e2e files
/// (`test_apps/`) so their `alef:hash:` lines can be re-stamped after the
/// sources hash changes — without regenerating their content. Also used by
/// [`super::write::finalize_hashes_sweeping`] as a self-healing safety net:
/// matching on the generated-file **marker** (not on an already-present hash)
/// is deliberate here — a file can be alef-owned and still be missing its
/// `alef:hash:` line (freshly written and not yet finalized, or stripped by
/// an older/interrupted run), and those are exactly the files that need to be
/// found so they can be re-stamped. Matching on `extract_hash(..).is_some()`
/// instead would make an already-unstamped file permanently invisible to this
/// scan, which is the bug this function exists to fix.
///
/// The directory-name list below is a fast path, not the gate. It cannot be completed —
/// `tmp`, `stage` and `build` are per-tool names, and each of `vendor/`, `deps/` and finally a
/// gem-packaging `tmp/` stage was added only after a staged copy had already been collected —
/// and a name that *is* on it can still be legitimate output (a `vendor/` tree committed for
/// offline builds). Git's ignore rules answer the question the names only approximate, so they
/// decide and the names merely save a walk. Ignored-ness rather than tracked-ness, because the
/// re-stamp route this feeds must still reach a file `alef generate` wrote moments ago and the
/// consumer has not committed yet; the stricter tracked-ness bar stays where it belongs, on
/// [`sweep_manifest_orphans`]'s deletion route. ~keep
pub fn collect_alef_headered_paths(root: &std::path::Path) -> std::collections::HashSet<std::path::PathBuf> {
    fn is_alef_owned(path: &std::path::Path) -> bool {
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        crate::core::hash::content_has_alef_marker(&content)
    }

    let mut paths = std::collections::HashSet::new();
    if !root.exists() {
        return paths;
    }
    let visible = crate::cli::git::IgnoreFilter::for_root(root);
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(
                    name,
                    ".git"
                        | "target"
                        | "node_modules"
                        | "vendor"
                        | "_build"
                        | "deps"
                        | ".venv"
                        | "venv"
                        | "build"
                        | "dist"
                        | "Pods"
                ) || !visible.allows(&path)
                {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() && visible.allows(&path) && is_alef_owned(&path) {
                paths.insert(path);
            }
        }
    }
    paths
}

fn path_is_alef_owned(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|content| content_is_alef_owned(&content))
}

/// Report disk-scanned orphan candidates. **Reports only — never deletes.**
///
/// The deletion this replaces was gated on: alef marker, git-tracked, under an owned root, absent
/// from this run's `keep`, and the root's manifest non-degenerate. Every one of those passed for a
/// consumer's Java *public API class*, and for a second class a live test depends on. Neither was
/// refused; the backend ran, emitted 56 files, and did not emit these two.
///
/// The rule's error is inferring **orphan** from **absent from this run's output**. Three
/// different situations produce that absence and nothing here distinguishes them: the emitter
/// stopped emitting the file (a true orphan), the emitter failed to emit it this run (a bug), or
/// it is a create-once seed alef emits only when absent. The non-degeneracy clause certifies that
/// the *backend* keeps books; it says nothing about whether *this file* was deliberately dropped.
///
/// The asymmetry decides it: a missed orphan leaves a stale file a human eventually notices, while
/// a false orphan silently removes a public API from a generated tree nobody reads by hand. Only a
/// positive assertion from the producer -- the generator recording that it no longer emits a path
/// -- can separate "dropped" from "failed to emit", and that record does not exist yet. Until it
/// does, this surface reports and a human decides. ~keep
fn report_disk_scan_candidates(root: &Path, candidates: &[PathBuf]) {
    // ~keep The claim is "absent from this run's recorded output", not "was not emitted". Those
    // differ: `keep` is the backends' own bookkeeping, and the check two screens up warns that
    // some backends record nothing but their Rust crate path. Asserting non-emission from a gap in
    // that record is the same false inference the C# visitor-file report used to make -- there it
    // named files the very same run had written. The body already lists non-emission as only one
    // of three explanations; the headline should not assert the one it cannot distinguish.
    tracing::warn!(
        "{} alef-marked, git-tracked file(s) under {} are not in this run's recorded output. NOT \
         deleted -- absence from that record does not prove an orphan (the emitter may have \
         stopped emitting it, failed to emit it, emit it only when absent, or not recorded it). \
         Review each and remove by hand if genuinely stale:",
        candidates.len(),
        root.display()
    );
    for path in candidates {
        tracing::warn!("  unrecorded alef-marked file: {}", path.display());
    }
}

/// Git-tracked files under `root`, or `None` when tracked-ness cannot be determined.
///
/// Required before [`sweep_manifest_orphans`]'s disk-scan route deletes anything: a build tool can
/// stage a copy of a real generated file at a mirrored path under the same owned root (a gem
/// packaging stage directory is the concrete case this was written against), and the copy
/// inherits the original's `alef:hash:` marker verbatim, so content alone cannot tell the two
/// apart. Alef's own output is committed by the consumer; a disposable staged copy is not (and is
/// usually gitignored), so tracked-ness is the signal that closes the gap the marker leaves open. ~keep
fn git_tracked_paths_under(root: &Path) -> Option<std::collections::HashSet<PathBuf>> {
    crate::cli::git::tracked_paths_under(root)
}

fn content_is_alef_owned(content: &str) -> bool {
    crate::core::hash::content_has_alef_marker(content) && crate::core::hash::extract_hash(content).is_some()
}

#[cfg(test)]
#[path = "orphans/tests.rs"]
mod sweep_roots_tests;
