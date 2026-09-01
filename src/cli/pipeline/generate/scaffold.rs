use super::normalization::normalize_content;
use super::write::apply_shebang_chmod;
use crate::core::backend::GeneratedFile;
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::ApiSurface;
use anyhow::Context as _;
use std::path::Path;
use tracing::{debug, warn};

#[path = "scaffold_migrations.rs"]
mod migrations;
#[path = "ruby_gemfile.rs"]
mod ruby_gemfile;

/// Generate scaffold files for given languages.
///
/// After the built-in scaffold generators run, each registered extension gets a
/// chance to rewrite the scaffold file set per language via
/// [`crate::core::extension::Extension::transform_scaffold_files`] — for example
/// to wire an ergonomic entry module into a package `main`/wrapper or to add
/// runtime dependencies to a manifest. Extensions receive their
/// `[extensions.<name>]` config from `config_path` (`alef.toml`).
pub fn scaffold(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
    config_path: &Path,
) -> anyhow::Result<Vec<GeneratedFile>> {
    let mut files = crate::scaffold::scaffold(api, config, languages)?;
    crate::with_extensions(|exts| {
        let env = crate::core::template_env::TemplateEnv::new();
        for ext in exts {
            let raw = crate::core::extension::read_extension_config(config_path, ext.name())
                .with_context(|| format!("extension `{}`: failed to read config from alef.toml", ext.name()))?;
            let cfg = ext
                .parse_config(raw.as_ref())
                .with_context(|| format!("extension `{}`: failed to parse config", ext.name()))?;
            for &language in languages {
                ext.transform_scaffold_files(api, &cfg, language, &mut files, &env)
                    .with_context(|| {
                        format!(
                            "extension `{}`: transform_scaffold_files({language}) failed",
                            ext.name()
                        )
                    })?;
            }
        }
        Ok::<(), anyhow::Error>(())
    })?;
    Ok(files)
}

/// Generate README files for given languages.
pub fn readme(
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    languages: &[Language],
) -> anyhow::Result<Vec<GeneratedFile>> {
    crate::readme::generate_readmes(api, config, languages)
}

/// Reconcile generated manifests needed by generated bindings.
///
/// Existing files are eligible only when their embedded marker proves Alef owns
/// them. Missing manifests may be created because the scaffold declaration itself
/// marks them as generated; handwritten manifests and non-manifest scaffold files
/// remain outside this generate-stage repair.
///
/// Eligibility is `generated_header` alone. It was additionally gated on
/// `extension == "toml"`, which silently excluded every non-TOML managed manifest —
/// `packages/java/pom.xml`, `packages/ruby/*.gemspec`, `packages/ruby/Rakefile`,
/// `crates/*-ffi/cmake/*-config.cmake`, `packages/elixir/mix.exs` — all of which are
/// emitted `generated_header: true` and are exactly as managed as the TOML ones. That
/// made this a *second*, independent reason those manifests never converge, on top of
/// the write-time ownership guard refusing them: repairing only the guard would have
/// left them stranded here anyway. Both mechanisms had to fail for the observed
/// stranding, so both had to be fixed. ~keep
pub fn reconcile_managed_scaffold_manifests(
    files: &[GeneratedFile],
    base_dir: &Path,
) -> anyhow::Result<super::write::WriteReport> {
    let mut manifests = Vec::new();
    for file in files.iter().filter(|file| file.generated_header) {
        let path = base_dir.join(&file.path);
        if !path.exists() {
            manifests.push(file.clone());
            continue;
        }
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("failed to read existing {}", path.display()))?;
        if crate::core::hash::content_has_alef_marker(&content) {
            manifests.push(file.clone());
        }
    }
    let report = write_scaffold_files_report(&manifests, base_dir, false)?;
    // `alef generate` reaches manifest writes only through this function, never through the
    // parent `generate` module's `write_scaffold_files*` wrappers -- so the relock hook has to
    // be called here too, or a manifest `alef generate` regenerates (as opposed to `alef
    // build`/`alef all`) would never get its sibling Cargo.lock refreshed. ~keep
    super::super::version_lockfiles::relock_lockfiles_beside_changed_manifests(&report.changed_paths);
    super::super::version_lockfiles::relock_dart_lockfiles_beside_generated_manifests(
        &manifests,
        base_dir,
        &report.changed_paths,
    );
    Ok(report)
}

/// Write standalone generated files (not grouped by language) to disk, returning the full
/// [`super::write::WriteReport`] (expected vs. actually-changed paths) instead of a bare count.
///
/// Scaffold files are create-only by default: if the target file already exists on disk it is
/// left untouched so that user customisations are preserved. Pass `overwrite = true` (e.g. via
/// `--clean`) to force-write all files.
///
/// Two call paths reach this function: the parent `generate` module's
/// [`super::write_scaffold_files`]/[`super::write_scaffold_files_with_overwrite`] wrappers (used
/// by `alef build`/`alef all`/`alef scaffold`), and [`reconcile_managed_scaffold_manifests`]
/// above (`alef generate`'s manifest-only path). Both relock any `Cargo.lock` beside a
/// `Cargo.toml` this call actually changed, each calling
/// [`crate::cli::pipeline::version_lockfiles::relock_lockfiles_beside_changed_manifests`] on the
/// returned report rather than this function doing it once itself -- so a future third caller
/// of this exact function needs to remember the same call, not get it for free.
///
/// `[workspace.ownership] user_owned` outranks every branch below, including `overwrite` --
/// see [`super::user_owned::skip_declared_existing`] for what a declared path does instead.
///
/// NEVER-STAMP / NEVER-OVERWRITE guard: for every file this run is about to write, if a
/// file already exists on disk at that path with content alef cannot prove it authored,
/// the file is left completely untouched (no header stamp, no content change, `overwrite`
/// notwithstanding, `generated_header` notwithstanding), and a warning is logged instead.
/// Proof of authorship is checked in two layers, content first:
///
/// 1. **Content-driven, any extension**: [`crate::core::hash::content_has_alef_marker`] scans
///    the existing bytes for the marker text regardless of the path's extension. This is not
///    limited to extensions [`super::write::marker_comment_style`] recognises — a backend can
///    self-mark inside `content` on an extension that has no registered `CommentStyle` at all
///    (docs pages get an HTML-comment header from `docs::render::with_html_header`, which
///    embeds "auto-generated by alef" directly and is never routed through
///    [`super::write::ensure_generated_header`]). Gating the marker check on "is this a
///    markable extension" — an earlier revision of this guard did exactly that — misreads
///    every already-owned, unstampable-by-extension file (every generated `.md` reference
///    page, `README.md`) as foreign on a cache-less checkout, which is worse than the bug
///    being fixed. Content is therefore always checked first, on every extension.
/// 2. **Extension-driven fallback, unmarkable extensions only**: when content carries no
///    marker and the extension genuinely cannot carry one
///    ([`super::write::marker_comment_style`] is `None` — `.json`, `.xml`, `.m4`, `.gradle`,
///    extensionless, ...), authorship falls back to [`crate::cli::cache::is_scaffold_owned_path`],
///    the committed record at `<base_dir>/.alef-ownership.toml` that this write path populates
///    via [`crate::cli::cache::record_scaffold_owned_path`] the first time it *creates* a path.
///    Committed rather than kept under `.alef/`, which alef writes into every consumer's
///    `.gitignore` itself (`cli::pipeline::extract::gitignore::ensure_gitignore`): while the
///    record was gitignored, a fresh clone and a warm dev machine gave different answers for
///    the same commit, so CI refused writes a developer's machine permitted (alef #80). It is
///    populated on creation and on an authorised rewrite only — never from a converged file,
///    since byte-equality with generated output is not evidence of authorship and a committed
///    record would spread that mistake to every clone. This route is not dead weight for
///    extensions that self-mark (docs pages, README) but it is the *only* protection for ones
///    that don't and never will (`package.json`, `.jar` — see
///    [`crate::cli::cache::scaffold_owned_path_key`]'s doc for a real bug in this record that
///    made it read as inert across ordinary multi-command sequences, now fixed).
///
/// A **markable** extension (`.rs`, `.go`, `.dart`, `.toml`, ...) with no marker in its
/// content is never eligible for the fallback: absence of a marker there is real evidence
/// alef never wrote it, since alef could have marked it and did not. This applies uniformly
/// regardless of `generated_header` — a `generated_header: false` seed (`build.zig`,
/// `*_test.dart`, `*Tests.swift`, ...) never gets a marker by design, so once such a seed
/// exists on disk it is permanently create-once: neither a hand-edit nor a later regen intent
/// (e.g. a version bump wanting to update an embedded version string) can silently touch it
/// again. That is intentional, not a gap — a seed that needs ongoing regeneration belongs on
/// the marker rail instead, by becoming `generated_header: true`.
///
/// The guard only ever engages when the newly generated content would actually change what
/// is on disk (byte-identical regeneration is always a silent no-op, whichever route proved
/// ownership), so a healthy, converged tree never logs spurious warnings.
///
/// This closes two incidents from the same root cause. The original (a consumer's
/// hand-written `e2e/go/helpers_test.go` / `e2e/go/main_test.go`) was a `generated_header: true` file on
/// a markable extension stamped over hand-written content because nothing checked for a
/// prior marker at all — fixed by the markable route above. The second (zig/dart/swift
/// scaffold seeds silently replaced by a routine `alef version` bump; `packages/java/pom.xml`
/// silently reclaimed because `.xml` cannot carry a marker) came from this guard being scoped
/// to `generated_header: true` only and from unmarkable extensions being fully exempt rather
/// than routed onto the cache-backed proof above — both closed by this revision.
///
/// `poly.toml` is exempt from this guard because [`merge_managed_toml`] intentionally merges
/// project lint configuration; a marker or ownership record would gate an exact-regeneration
/// overwrite that this path never performs in the first place. Binary (`.jar`) targets are
/// unmarkable text-wise but still route through [`crate::cli::cache::is_scaffold_owned_path`]
/// for the same reason as any other unmarkable extension. ~keep
pub fn write_scaffold_files_report(
    files: &[GeneratedFile],
    base_dir: &Path,
    overwrite: bool,
) -> anyhow::Result<super::write::WriteReport> {
    let declared = super::user_owned::declared_user_owned(base_dir)?;
    let mut report = super::write::WriteReport::default();
    let mut prepared = std::collections::BTreeMap::new();
    for file in files {
        if let Some(existing) = prepared.insert(file.path.clone(), file) {
            anyhow::ensure!(
                existing.content == file.content && existing.generated_header == file.generated_header,
                "multiple generators emitted different content for {}",
                file.path.display()
            );
        }
    }
    for file in prepared.into_values() {
        let full_path = super::write::contained_output_path(base_dir, &file.path)?;
        // Asked once per file, not re-derived at each refusal site below: the identical
        // predicate `alef adopt` gates `--clobber-create-once-seeds` on, so a refusal this
        // guard reports and adopt's own refusal of the same path can never disagree. ~keep
        let create_once = crate::cli::commands::adopt::is_create_once_seed(file);
        if super::user_owned::skip_declared_existing(&declared, base_dir, &full_path, &mut report) {
            continue;
        }
        let ruby_gemfile = ruby_gemfile::MergeCandidate::new(file, &full_path);
        if ruby_gemfile.should_skip_scaffold(overwrite) {
            report.expected_paths.insert(full_path.clone());
            debug!("  skipped (already exists): {}", full_path.display());
            continue;
        }
        let is_jar_file = super::binary::is_base64_binary_output(&full_path);
        let is_poly_merge_target = file.path == Path::new(POLY_CONFIG) && full_path.exists();
        if is_jar_file {
            let binary_content = super::binary::decode_base64_binary(&full_path, &file.content)?;
            let existing_binary = std::fs::read(&full_path).ok();
            if existing_binary.as_deref() == Some(binary_content.as_slice()) {
                debug!("  unchanged: {}", full_path.display());
                // Records nothing: a pre-existing `.jar` that happens to match is not proof
                // alef put it there, and the record is now committed, so the mistaken claim
                // would travel to every clone. See the text branch below. ~keep
                continue;
            }
            if existing_binary.is_some() && !crate::cli::cache::is_scaffold_owned_path(base_dir, &full_path) {
                warn!(
                    "refusing to write {}: pre-existing file has no durable record of alef ownership",
                    full_path.display()
                );
                // Counted like any other refusal: a binary target cannot carry a marker, so
                // it is permanently part of the residue and must not be silently omitted
                // from the number that reports it. ~keep
                report.refuse_drifted(&full_path, create_once);
                continue;
            }
            report.expected_paths.insert(full_path.clone());
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory {}", parent.display()))?;
            }
            super::write::atomic_write(&full_path, &binary_content)?;
            if !declared.matches(base_dir, &full_path) {
                crate::cli::cache::record_scaffold_owned_path(base_dir, &full_path)?;
            }
            report.changed_paths.insert(full_path.clone());
            debug!("  wrote (binary): {}", full_path.display());
            continue;
        }

        let content = if is_poly_merge_target {
            let existing = std::fs::read_to_string(&full_path)
                .with_context(|| format!("failed to read existing {}", full_path.display()))?;
            merge_managed_toml(&existing, &file.content, base_dir, &file.path)
                .with_context(|| format!("failed to merge existing {}", full_path.display()))?
        } else if ruby_gemfile.is_target() {
            ruby_gemfile.merge_file()?
        } else {
            if file.path == Path::new(POLY_CONFIG) {
                // Brand-new merge target (nothing on disk to merge with yet): still
                // record this run's generated array values as the provenance
                // baseline, so pruning is available starting the very next run that
                // actually merges, instead of needing a second merge pass first to
                // establish one.
                record_poly_merge_baseline(base_dir, &file.path, &file.content)
                    .with_context(|| format!("failed to record merge baseline for {}", full_path.display()))?;
            }
            file.content.clone()
        };
        let normalized = normalize_content(&full_path, &content);
        // Seeded unstamped -- see `super::user_owned::skip_declared_existing`. ~keep
        let normalized = if file.generated_header && !declared.matches(base_dir, &full_path) {
            super::write::ensure_generated_header(&full_path, &normalized)
        } else {
            normalized
        };

        if full_path.exists() {
            let existing_text = std::fs::read_to_string(&full_path).ok();
            let is_unchanged = existing_text.as_deref().is_some_and(|existing| {
                crate::core::hash::strip_hash_line(existing) == crate::core::hash::strip_hash_line(&normalized)
            });
            if is_unchanged {
                apply_shebang_chmod(&full_path, &normalized)?;
                debug!("  unchanged: {}", full_path.display());
                // Deliberately records nothing. Byte-equality with this run's output is not
                // evidence of authorship — it is exactly the coincidence the founding
                // incident (`e2e/go/helpers_test.go`) turned on — so recording here is the
                // rejected `bootstrap_owned` predicate wearing the record as a disguise. It
                // mattered less while the record was machine-local and gitignored; now that
                // it is committed (`cache::OWNERSHIP_MANIFEST`) a coincidence on one
                // developer's disk would become a permanent claim in every clone. Files alef
                // really owns were recorded by the authorised-write path below on the run
                // that created them. ~keep
                continue;
            }
            if !is_poly_merge_target && !ruby_gemfile.is_target() {
                // Content-driven marker detection is checked first, on every extension,
                // not only markable ones: a backend can self-mark inside `content` on an
                // extension `marker_comment_style` has no comment syntax for at all (docs
                // pages get an HTML-comment header from `docs::render::with_html_header`
                // that is never routed through `ensure_generated_header`/`CommentStyle`).
                // Gating the check on `is_markable` missed that marker entirely and read
                // every already-owned `.md` reference page as foreign on a cache-less
                // checkout. The committed ownership record is the fallback for extensions that
                // truly cannot carry a marker in any form, not the primary signal. ~keep
                let has_marker = existing_text
                    .as_deref()
                    .is_some_and(crate::core::hash::content_has_alef_marker);
                let is_markable = super::write::marker_comment_style(&full_path).is_some();
                // No third, content-equivalence route is added here on purpose: it cannot
                // tell an older-release alef file from a hand-written one that coincides,
                // and the founding incident (`e2e/go/helpers_test.go`) is exactly that
                // coincidence. Adoption is `alef adopt`'s job, with a diff and a human.
                // See `super::write::stamp_for_adoption`. ~keep
                //
                // The snippet-coverage ledger is the one narrow exception: it is strict
                // JSON (never markable) and its dotfile name has no meaning to anything but
                // alef's own bookkeeping, so a pre-existing, unrecorded copy is trusted by
                // name alone instead of being permanently refused for want of a record that
                // could only ever have been created by a write this same guard forever
                // blocks. See `e2e::snippets::is_snippet_coverage_manifest_path`'s doc for
                // why this is not widened into a general filename allowlist. ~keep
                // A generated snippet recorded in the PREVIOUS run's coverage ledger is owned by
                // that record, not by an inference. `e2e::snippets::coverage::orphaned_paths`
                // already treats `generated_metadata` as the one place alef records "I wrote this
                // exact path", and alef already UNLINKS files on the strength of it -- refusing to
                // overwrite the same recorded paths while being willing to delete them is
                // incoherent. The snapshot is taken before generation, so this run's own
                // intentions can never widen it. ~keep
                //
                // Delegates to `write::is_owned_by_ownership_record` rather than inlining its own
                // copy of the OR-chain -- see that function's doc for the drift this closes. ~keep
                let owned =
                    has_marker || (!is_markable && super::write::is_owned_by_ownership_record(base_dir, &full_path));
                if !owned {
                    // Distinguishes "nothing here even tried" from "something here tried and got
                    // the spelling wrong" -- see `hash::near_miss_marker`'s doc. Kept deliberately
                    // identical to `write.rs`'s guard, same as `owned` above. ~keep
                    match existing_text.as_deref().and_then(crate::core::hash::near_miss_marker) {
                        Some(near_miss) => warn!(
                            "refusing to write {}: pre-existing file carries no alef marker and \
                             alef has no durable record of ever owning it -- its leading lines \
                             contain {near_miss:?}, close but not recognized (alef accepts \
                             \"generated by alef\" case-insensitively)",
                            full_path.display()
                        ),
                        None => warn!(
                            "refusing to write {}: pre-existing file carries no alef marker and \
                             alef has no durable record of ever owning it",
                            full_path.display()
                        ),
                    }
                    report.refuse_text(&full_path, existing_text.as_deref(), &normalized, create_once);
                    continue;
                }
            }
        }

        report.expected_paths.insert(full_path.clone());
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
        super::write::atomic_write(&full_path, normalized.as_bytes())?;
        apply_shebang_chmod(&full_path, &normalized)?;
        // A snippet already owned by the coverage ledger is deliberately NOT copied into
        // `.alef-ownership.toml`. Recording it would put one fact in two records that can then
        // disagree -- the defect this release is named for -- and would add one committed line per
        // generated snippet (thousands, on a consumer with a full snippet tree) to a file whose
        // whole value is being readable. ~keep
        if !is_poly_merge_target
            && super::write::marker_comment_style(&full_path).is_none()
            && !crate::e2e::snippets::ownership::is_ledger_owned_snippet_path(base_dir, &full_path)
            && !declared.matches(base_dir, &full_path)
        {
            crate::cli::cache::record_scaffold_owned_path(base_dir, &full_path)?;
        }
        report.changed_paths.insert(full_path.clone());
        debug!("  wrote: {}", full_path.display());
        if file.path == Path::new(POLY_CONFIG) {
            normalize_poly_config(&full_path, base_dir);
        }
    }

    // Every in-place repair to a pre-existing, create-once scaffold file this run's `files`
    // still names -- see `migrations::apply_pending_migrations`'s doc for why these run here,
    // after the write loop, rather than through the normal write path above. ~keep
    migrations::apply_pending_migrations(files, base_dir)?;

    // The refusal summary is deliberately NOT emitted here. Scaffolding is one of five writing
    // phases, and a summary printed by one writer can only ever describe that writer's refusals —
    // which is how `alef all` came to report the scaffold phase's refusals while silently omitting
    // every binding-phase one. Callers accumulate with `absorb_unwritten` and report once. ~keep
    Ok(report)
}

/// Repo-root poly config, emitted by the scaffold pass.
const POLY_CONFIG: &str = "poly.toml";

/// Merge alef's freshly generated `poly.toml` into the consumer's existing one.
///
/// Three passes, in order:
///
/// 1. **Prune.** Remove array values from `existing` that alef itself proposed on
///    some prior run (per [`crate::cli::cache::read_toml_merge_provenance`],
///    recorded straight from a past run's `generated` -- never from `existing`,
///    so a consumer's own duplicate of the same string is never what's being
///    tracked) but no longer proposes this run. A value the consumer still
///    configures via `[workspace.poly] exclude`/`file_safety_exclude` is echoed
///    back into `generated` every run for as long as it stays configured, so it
///    is never a prune candidate; see the doc on
///    [`crate::cli::cache::read_toml_merge_provenance`] for the full argument
///    and its one disclosed gap (a value the consumer hand-typed directly into
///    the array, bypassing that config field, that happens to collide with a
///    string alef also once emitted, is indistinguishable from alef's own copy
///    and would be pruned alongside it).
/// 2. **Union.** [`merge_tables`] adds anything from `generated` not already
///    present, using [`values_equal`] (the decoded value, not its serialized
///    text) so differing quote style or `poly fmt` decor from a previous pass
///    can never cause a spurious duplicate to be appended.
/// 3. **Dedupe.** Every array touched by the union pass collapses any
///    already-present duplicates down to one occurrence
///    ([`dedupe_array`]) -- unconditionally safe regardless of who authored a
///    given entry, since removing a redundant *copy* of a value that is still
///    present at least once never changes the set of values in the array, and
///    fixes the monotonic 4x-per-run growth this merge exhibited before this
///    fix existed for `poly.toml`'s `exclude` arrays specifically.
///
/// The full set of array values this run generated is then recorded (replacing
/// last run's), so a future run can repeat step 1 against a fresh baseline.
///
/// This is the write-path entry point: it persists the new provenance
/// snapshot as a side effect, via [`crate::cli::cache::write_toml_merge_provenance`].
/// [`merge_managed_toml_preview`] shares the same prune/union/dedupe logic for
/// callers that only want to know what *would* change (e.g. `diff_files`,
/// which runs in parallel over many files) without racing concurrent writers
/// on the same provenance file or mutating state a read-only preview has no
/// business mutating.
pub(super) fn merge_managed_toml(
    existing: &str,
    generated: &str,
    base_dir: &Path,
    relative_path: &Path,
) -> anyhow::Result<String> {
    let previous_generated_arrays = crate::cli::cache::read_toml_merge_provenance(base_dir, relative_path);
    let (merged, current_generated_arrays) = merge_managed_toml_core(existing, generated, &previous_generated_arrays)?;
    crate::cli::cache::write_toml_merge_provenance(base_dir, relative_path, &current_generated_arrays)?;
    Ok(merged)
}

/// Read-only counterpart to [`merge_managed_toml`] for preview/diff callers:
/// runs the identical prune + union + dedupe merge against the same
/// committed provenance record, but never writes a new snapshot back. Safe
/// to call from multiple threads concurrently (only reads shared, committed
/// `.alef-toml-merge-provenance.toml` state).
pub(super) fn merge_managed_toml_preview(
    existing: &str,
    generated: &str,
    base_dir: &Path,
    relative_path: &Path,
) -> anyhow::Result<String> {
    let previous_generated_arrays = crate::cli::cache::read_toml_merge_provenance(base_dir, relative_path);
    Ok(merge_managed_toml_core(existing, generated, &previous_generated_arrays)?.0)
}

/// Record `generated`'s own array values as the merge-provenance baseline for
/// `relative_path` without merging anything -- used when a merge target like
/// `poly.toml` is being created for the first time (nothing on disk yet to
/// merge with), so the *next* run that does merge has a baseline to prune
/// against immediately rather than needing a merge pass of its own first.
fn record_poly_merge_baseline(base_dir: &Path, relative_path: &Path, generated: &str) -> anyhow::Result<()> {
    let generated_doc = generated.parse::<toml_edit::DocumentMut>()?;
    let mut arrays = std::collections::BTreeMap::new();
    collect_arrays_by_path(generated_doc.as_table(), "", &mut arrays);
    crate::cli::cache::write_toml_merge_provenance(base_dir, relative_path, &arrays)
}

fn merge_managed_toml_core(
    existing: &str,
    generated: &str,
    previous_generated_arrays: &std::collections::BTreeMap<String, Vec<String>>,
) -> anyhow::Result<(String, std::collections::BTreeMap<String, Vec<String>>)> {
    let mut existing_doc = existing.parse::<toml_edit::DocumentMut>()?;
    let generated_doc = generated.parse::<toml_edit::DocumentMut>()?;

    let mut current_generated_arrays = std::collections::BTreeMap::new();
    collect_arrays_by_path(generated_doc.as_table(), "", &mut current_generated_arrays);

    for (path, previous_values) in previous_generated_arrays {
        // A path this run did not emit AT ALL is unknown, not empty. `[lint.python.ruff]` and
        // `[lint.php.mago]` are emitted only when their language is in scope, so a scoped run
        // (`alef generate --lang java`) omits every other language's tables -- and reading that
        // absence as "alef no longer proposes these values" prunes the consumer's entire rule
        // selection out of a file alef only co-owns, leaving `select = []`, which every linter
        // reads as "check nothing" while still exiting green. Only a path this run DID emit can
        // testify that one of its values is gone. ~keep
        let Some(current_values) = current_generated_arrays.get(path) else {
            continue;
        };
        let dropped: Vec<String> = previous_values
            .iter()
            .filter(|value| !current_values.contains(value))
            .cloned()
            .collect();
        if !dropped.is_empty() {
            remove_values_at_path(existing_doc.as_table_mut(), path, &dropped);
        }
    }

    merge_tables(existing_doc.as_table_mut(), generated_doc.as_table());
    Ok((existing_doc.to_string(), current_generated_arrays))
}

fn merge_tables(existing: &mut toml_edit::Table, generated: &toml_edit::Table) {
    for (key, generated_item) in generated {
        match existing.get_mut(key) {
            Some(existing_item) => merge_items(existing_item, generated_item),
            None => {
                existing.insert(key, detached_item(generated_item.clone()));
            }
        }
    }
}

fn merge_items(existing: &mut toml_edit::Item, generated: &toml_edit::Item) {
    match (existing, generated) {
        (toml_edit::Item::Table(existing), toml_edit::Item::Table(generated)) => {
            merge_tables(existing, generated);
        }
        (toml_edit::Item::Value(existing), toml_edit::Item::Value(generated)) => {
            merge_values(existing, generated);
        }
        (existing, generated) => *existing = detached_item(generated.clone()),
    }
}

fn merge_values(existing: &mut toml_edit::Value, generated: &toml_edit::Value) {
    match (existing, generated) {
        (toml_edit::Value::Array(existing), toml_edit::Value::Array(generated)) => {
            for value in generated.iter() {
                if !existing.iter().any(|candidate| values_equal(candidate, value)) {
                    existing.push(value.clone());
                }
            }
            dedupe_array(existing);
        }
        (toml_edit::Value::InlineTable(existing), toml_edit::Value::InlineTable(generated)) => {
            for (key, generated_value) in generated.iter() {
                match existing.get_mut(key) {
                    Some(existing_value) => merge_values(existing_value, generated_value),
                    None => {
                        existing.insert(key, generated_value.clone());
                    }
                }
            }
        }
        (existing, generated) => {
            let decor = existing.decor().clone();
            *existing = generated.clone();
            *existing.decor_mut() = decor;
        }
    }
}

/// Strip a leading `/` from a `poly.toml` array value when doing so cannot
/// change what the string means to poly, so array-identity comparisons treat
/// `/packages/**` and `packages/**` as the same entry instead of accumulating
/// both spellings forever.
///
/// poly's own anchoring rule (`build_excludes` in
/// `poly-core/src/discover.rs`, gitignore-style): a glob is anchored to the
/// walk root whenever it carries a `/` *anywhere except as a single trailing
/// character* — a leading `/`, or one in the middle. `packages/**` already
/// carries a middle `/` (the one before `**`, which is not the pattern's last
/// character), so it is anchored exactly like `/packages/**` — the leading
/// `/` is redundant and safe to drop before comparing. `Package.swift` carries
/// no `/` at all, so it is unanchored (matches at any depth); `/Package.swift`
/// anchors it to root instead — there the leading `/` changes which files
/// match, so it is left alone. ~keep
fn strip_redundant_leading_slash(value: &str) -> &str {
    let Some(rest) = value.strip_prefix('/') else {
        return value;
    };
    let without_trailing_slash = rest.strip_suffix('/').unwrap_or(rest);
    if without_trailing_slash.contains('/') {
        rest
    } else {
        value
    }
}

/// Semantic equality for a TOML value, ignoring decor (whitespace, comments,
/// quote style) entirely rather than comparing serialized text.
///
/// The array-merge duplicate check used to compare `value.to_string().trim()`,
/// which includes each value's decor -- so an existing entry reformatted by
/// `poly fmt` between runs (different quoting, different surrounding
/// whitespace from multi-line array formatting) no longer textually matched
/// the freshly generated value even though they decode to the same string,
/// and got re-appended as a "new" entry on every single pass. Comparing the
/// decoded value sidesteps decor entirely. ~keep
fn values_equal(existing: &toml_edit::Value, generated: &toml_edit::Value) -> bool {
    match (existing, generated) {
        (toml_edit::Value::String(existing), toml_edit::Value::String(generated)) => {
            strip_redundant_leading_slash(existing.value()) == strip_redundant_leading_slash(generated.value())
        }
        (toml_edit::Value::Integer(existing), toml_edit::Value::Integer(generated)) => {
            existing.value() == generated.value()
        }
        (toml_edit::Value::Float(existing), toml_edit::Value::Float(generated)) => {
            existing.value() == generated.value()
        }
        (toml_edit::Value::Boolean(existing), toml_edit::Value::Boolean(generated)) => {
            existing.value() == generated.value()
        }
        (toml_edit::Value::Datetime(existing), toml_edit::Value::Datetime(generated)) => {
            existing.value() == generated.value()
        }
        (toml_edit::Value::Array(existing), toml_edit::Value::Array(generated)) => {
            existing.len() == generated.len() && existing.iter().zip(generated.iter()).all(|(a, b)| values_equal(a, b))
        }
        (toml_edit::Value::InlineTable(existing), toml_edit::Value::InlineTable(generated)) => {
            existing.len() == generated.len()
                && existing
                    .iter()
                    .all(|(key, value)| generated.get(key).is_some_and(|other| values_equal(value, other)))
        }
        (existing, generated) => existing.to_string().trim() == generated.to_string().trim(),
    }
}

/// Collapse an array's already-present duplicate values down to one
/// occurrence each, keeping the first and dropping the rest. Unconditionally
/// safe: removing a redundant copy of a value that remains present at least
/// once never changes the *set* of values the array represents, so this
/// needs no ownership information at all -- unlike pruning a value that
/// alef no longer emits, which does (see [`merge_managed_toml`]'s doc). ~keep
fn dedupe_array(array: &mut toml_edit::Array) {
    let mut kept: Vec<toml_edit::Value> = Vec::new();
    let mut index = 0;
    while index < array.len() {
        let is_duplicate = array
            .get(index)
            .is_some_and(|value| kept.iter().any(|seen| values_equal(seen, value)));
        if is_duplicate {
            array.remove(index);
            continue;
        }
        if let Some(value) = array.get(index) {
            kept.push(value.clone());
        }
        index += 1;
    }
}

/// The decoded, decor-free representation of a value used for the merge
/// provenance record and the prune comparison -- a plain string decodes to
/// itself modulo [`strip_redundant_leading_slash`] (so `/packages/**` and
/// `packages/**` are recorded and pruned as the same value, matching the
/// identity [`values_equal`] uses for the union pass); anything else falls
/// back to trimmed serialized text (arrays and inline tables are not a shape
/// alef's own `exclude`-style generators ever emit as array *elements*, so
/// this fallback is not expected to matter in practice, only to avoid
/// silently dropping an unusual value from the record).
fn canonical_value_repr(value: &toml_edit::Value) -> String {
    match value {
        toml_edit::Value::String(value) => strip_redundant_leading_slash(value.value()).to_string(),
        other => other.to_string().trim().to_string(),
    }
}

/// Walk `table` recording every array's decoded values under its dotted key
/// path (`"discovery.exclude"`, `"hooks.builtin.file_safety.exclude"`, ...).
/// Descends into plain tables and one level into inline tables (matching
/// `poly.toml`'s actual shape: `[hooks.builtin]` is a table whose `lint`,
/// `fmt`, `file_safety` members are inline tables each holding an `exclude`
/// array) so every one of poly.toml's exclude blocks gets its own entry,
/// tracked independently.
fn collect_arrays_by_path(
    table: &toml_edit::Table,
    prefix: &str,
    out: &mut std::collections::BTreeMap<String, Vec<String>>,
) {
    for (key, item) in table {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        match item {
            toml_edit::Item::Table(nested) => collect_arrays_by_path(nested, &path, out),
            toml_edit::Item::Value(toml_edit::Value::Array(array)) => {
                out.insert(path, array.iter().map(canonical_value_repr).collect());
            }
            toml_edit::Item::Value(toml_edit::Value::InlineTable(inline)) => {
                for (inner_key, inner_value) in inline.iter() {
                    if let toml_edit::Value::Array(array) = inner_value {
                        out.insert(
                            format!("{path}.{inner_key}"),
                            array.iter().map(canonical_value_repr).collect(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Remove every value in `values_to_remove` from the array found by
/// descending `table` along `path`'s dot-separated segments (mirrors
/// [`collect_arrays_by_path`]'s traversal: plain tables, then one level of
/// inline table). A path segment that no longer resolves to an array --
/// because the section was removed, renamed, or was never an array to begin
/// with -- is silently skipped: there is nothing to prune there, which is a
/// normal, expected outcome, not an error.
fn remove_values_at_path(table: &mut toml_edit::Table, path: &str, values_to_remove: &[String]) {
    let mut parts = path.splitn(2, '.');
    let Some(head) = parts.next() else { return };
    match (parts.next(), table.get_mut(head)) {
        (None, Some(toml_edit::Item::Value(toml_edit::Value::Array(array)))) => {
            remove_matching(array, values_to_remove);
        }
        (Some(rest), Some(toml_edit::Item::Table(nested))) => {
            remove_values_at_path(nested, rest, values_to_remove);
        }
        (Some(rest), Some(toml_edit::Item::Value(toml_edit::Value::InlineTable(inline)))) => {
            remove_values_at_inline_path(inline, rest, values_to_remove);
        }
        _ => {}
    }
}

fn remove_values_at_inline_path(inline: &mut toml_edit::InlineTable, path: &str, values_to_remove: &[String]) {
    let mut parts = path.splitn(2, '.');
    let Some(head) = parts.next() else { return };
    match (parts.next(), inline.get_mut(head)) {
        (None, Some(toml_edit::Value::Array(array))) => remove_matching(array, values_to_remove),
        (Some(rest), Some(toml_edit::Value::InlineTable(nested))) => {
            remove_values_at_inline_path(nested, rest, values_to_remove);
        }
        _ => {}
    }
}

fn remove_matching(array: &mut toml_edit::Array, values_to_remove: &[String]) {
    let mut index = 0;
    while index < array.len() {
        let should_remove = array.get(index).is_some_and(|value| {
            values_to_remove
                .iter()
                .any(|stale| *stale == canonical_value_repr(value))
        });
        if should_remove {
            array.remove(index);
        } else {
            index += 1;
        }
    }
}

fn detached_item(mut item: toml_edit::Item) -> toml_edit::Item {
    match &mut item {
        toml_edit::Item::Value(value) => value.decor_mut().clear(),
        toml_edit::Item::Table(table) => {
            table.set_position(None);
            let keys = table.iter().map(|(key, _)| key.to_string()).collect::<Vec<_>>();
            for key in keys {
                if let Some(child) = table.remove(&key) {
                    table.insert(&key, detached_item(child));
                }
            }
        }
        toml_edit::Item::ArrayOfTables(tables) => {
            for table in tables.iter_mut() {
                table.set_position(None);
            }
        }
        toml_edit::Item::None => {}
    }
    item
}

/// Hand `poly.toml` to poly immediately after writing it.
///
/// poly defines the canonical TOML form and the scaffold emits the file from a
/// hand-rolled string template that does not match it, so the file is rewritten on
/// every run and, left raw, fails the consumer's own `poly fmt --check`. The
/// full-regen convergence pass normally repairs it, but that runs many fallible
/// stages later (post-build, stubs, readme, e2e, docs) — an abort in any of them
/// leaves the raw file behind — and the partial-regen paths never pass the repo
/// root to poly at all. Formatting it here closes both gaps for the cost of one
/// single-file invocation. Best-effort: `poly_format` warns and returns when poly
/// is not on PATH.
fn normalize_poly_config(full_path: &Path, base_dir: &Path) {
    crate::cli::pipeline::poly_format(std::slice::from_ref(&full_path.to_path_buf()), base_dir);
}

#[cfg(test)]
mod merge_managed_toml_tests {
    use super::*;

    #[test]
    fn scaffold_write_boundary_rejects_an_escaping_generated_file() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let base = temporary.path().join("base");
        std::fs::create_dir(&base).expect("base directory");
        let files = vec![GeneratedFile {
            path: "../escaped.rs".into(),
            content: "pub fn generated() {}\n".into(),
            generated_header: true,
        }];

        let error = write_scaffold_files_report(&files, &base, true)
            .expect_err("escaping scaffold GeneratedFile path must be rejected");
        assert!(error.to_string().contains("not contained"), "{error}");
        assert!(!temporary.path().join("escaped.rs").exists());
    }

    fn exclude_values(merged: &str) -> Vec<String> {
        let doc = merged
            .parse::<toml_edit::DocumentMut>()
            .expect("parse merged poly.toml");
        doc["discovery"]["exclude"]
            .as_array()
            .expect("discovery.exclude is an array")
            .iter()
            .map(|value| value.as_str().expect("exclude entries are strings").to_string())
            .collect()
    }

    #[test]
    fn merge_does_not_duplicate_a_leading_slash_spelling_variant_of_an_existing_glob() {
        let existing = "[discovery]\nexclude = [\"/packages/**\"]\n";
        let generated = "[discovery]\nexclude = [\"packages/**\"]\n";
        let previous_generated_arrays = std::collections::BTreeMap::new();

        let (merged, _) =
            merge_managed_toml_core(existing, generated, &previous_generated_arrays).expect("merge succeeds");

        assert_eq!(
            exclude_values(&merged),
            vec!["/packages/**".to_string()],
            "packages/** and /packages/** anchor identically to poly (the / before ** already \
             anchors it) -- the union pass must not append a second, differently-spelled copy"
        );
    }

    #[test]
    fn merge_prunes_a_stale_glob_even_when_the_consumer_file_spells_it_with_a_leading_slash() {
        let existing = "[discovery]\nexclude = [\"/packages/**\", \"kept/**\"]\n";
        let generated = "[discovery]\nexclude = [\"kept/**\"]\n";
        let mut previous_generated_arrays = std::collections::BTreeMap::new();
        previous_generated_arrays.insert("discovery.exclude".to_string(), vec!["packages/**".to_string()]);

        let (merged, current_generated_arrays) =
            merge_managed_toml_core(existing, generated, &previous_generated_arrays).expect("merge succeeds");

        assert_eq!(
            exclude_values(&merged),
            vec!["kept/**".to_string()],
            "alef recorded packages/** as its own prior proposal and no longer generates it; the \
             consumer's on-disk copy spelled it /packages/** and must still be pruned rather than \
             surviving forever as unrecognised foreign content"
        );
        assert_eq!(
            current_generated_arrays.get("discovery.exclude"),
            Some(&vec!["kept/**".to_string()])
        );
    }

    #[test]
    fn strip_redundant_leading_slash_treats_a_dir_glob_leading_slash_as_a_no_op() {
        assert_eq!(strip_redundant_leading_slash("/packages/**"), "packages/**");
        assert_eq!(strip_redundant_leading_slash("packages/**"), "packages/**");
    }

    #[test]
    fn strip_redundant_leading_slash_keeps_a_leading_slash_that_changes_anchoring() {
        // `Package.swift` carries no other `/`, so it is unanchored (matches at any
        // depth); `/Package.swift` anchors it to the walk root instead -- these are
        // genuinely different globs to poly and must not be collapsed.
        assert_eq!(strip_redundant_leading_slash("/Package.swift"), "/Package.swift");
        assert_eq!(strip_redundant_leading_slash("Package.swift"), "Package.swift");
    }
}
