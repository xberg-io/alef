//! In-place repair for a `poly.toml` table [`crate::scaffold::scaffold_poly_config`] stopped
//! emitting but that `merge_managed_toml`'s managed-merge never reaches on an already-scaffolded
//! consumer. See [`migrate_poly_toml_drop_snippet_hook`]'s doc for the full defect, and
//! [`migrate_poly_toml_drop_unrunnable_snapshot_hooks`]'s doc for a second, independent instance
//! of the same defect.

use anyhow::Context as _;
use std::path::Path;

/// Path of the repo-root poly config this migration repairs, relative to the repo root.
const POLY_CONFIG_RELATIVE: &str = "poly.toml";

/// The exact `run` invocation `workspace_hook`'s retracted snippet-check call site (`snippet_check_hook`,
/// dropped in `a139a680`, "drops the alef-snippets pre-commit hook from generated poly.toml") last
/// emitted -- matched as a *substring* of the table's `run` command, alongside `workspace = true`, so
/// a consumer's own unrelated `[hooks.pre-commit.commands.alef-snippets]` entry running a different
/// command is never a match, while a consumer who wrapped this exact invocation in their own shell
/// guard (observed on a real consumer: `sh -c 'command -v alef >/dev/null 2>&1 && exec alef snippets
/// check --strict --cache off || echo ...'`, hardening against `alef` being absent from a lint job's
/// `PATH`) is still recognised as carrying alef's own retracted hook, not a repurposed one. ~keep
const STALE_SNIPPET_HOOK_RUN: &str = "alef snippets check --strict --cache off";

/// Remove a pre-existing `[hooks.pre-commit.commands.alef-snippets]` table from `poly.toml` --
/// the exact hook retracted from generation in `a139a680` but never reachable on an
/// already-scaffolded consumer.
///
/// `merge_managed_toml_core`'s prune pass (see its doc) tracks and removes only ARRAY values,
/// via `.alef/toml-merge-provenance.json` -- never a whole TABLE alef stops emitting. The union
/// pass that follows only ever ADDS tables present in `generated` and not yet in `existing`; it
/// has no counterpart that removes one present in `existing` but absent from `generated`. A
/// consumer scaffolded while `workspace_hook` still emitted this table therefore keeps
/// re-merging it forever: every regenerate leaves it untouched (it is already present, so the
/// union pass changes nothing), and nothing in the merge ever proposes removing it. Every commit
/// this hook runs on shells out to an `alef` binary the consumer's lint job never installs,
/// failing `poly lint`/pre-commit with `alef-snippets: 1: alef: not found`.
///
/// Guarded on the table's own `run` command *containing* [`STALE_SNIPPET_HOOK_RUN`] -- the exact
/// invocation alef itself ever emitted here -- AND `workspace = true`, the only mode
/// `workspace_hook` ever set for it. Matching on the table's name alone would risk removing a
/// consumer's own, differently-configured `alef-snippets` command; this guard leaves that
/// untouched. Substring rather than exact-equality containment is deliberate: it still recognises
/// alef's own retracted call site inside a consumer's shell wrapper around it (e.g. a `command -v
/// alef` PATH guard), while a table whose `run` never invokes this call at all -- a genuine
/// repurposing -- still does not match. Silent
/// (returns `Ok(false)`) on a missing `poly.toml`, unparsable TOML, a `poly.toml` with no such
/// table, or a table that no longer matches (idempotent: nothing left to remove on a second
/// pass). ~keep
pub(crate) fn migrate_poly_toml_drop_snippet_hook(base_dir: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, POLY_CONFIG_RELATIVE.as_ref())?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut doc) = existing.parse::<toml_edit::DocumentMut>() else {
        return Ok(false);
    };

    let commands = doc
        .as_table_mut()
        .get_mut("hooks")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|hooks| hooks.get_mut("pre-commit"))
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|pre_commit| pre_commit.get_mut("commands"))
        .and_then(toml_edit::Item::as_table_mut);
    let Some(commands) = commands else {
        return Ok(false);
    };

    let is_stale_snippet_hook = commands
        .get("alef-snippets")
        .and_then(toml_edit::Item::as_table)
        .is_some_and(|hook| {
            hook.get("run")
                .and_then(toml_edit::Item::as_str)
                .is_some_and(|run| run.contains(STALE_SNIPPET_HOOK_RUN))
                && hook.get("workspace").and_then(toml_edit::Item::as_bool) == Some(true)
        });
    if !is_stale_snippet_hook {
        return Ok(false);
    }

    commands.remove("alef-snippets");

    let parent = path.parent().context("poly.toml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, doc.to_string().as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing poly.toml: removed the retracted alef-snippets pre-commit hook"
    );
    Ok(true)
}

/// The exact `run` commands `workspace_hook`'s four retracted "cannot pass in poly's isolated
/// staged snapshot" call sites -- `rubocop`, `steep`, `dart-analyze`, `dart-e2e-analyze`, all
/// dropped in `8ed9ad8d4` ("drop pre-commit hooks that cannot run in poly's snapshot") -- ever
/// emitted, across every alef release that rendered them. `dart-analyze`/`dart-e2e-analyze` never
/// changed shape, but rubocop/steep passed through two independent hardening passes
/// (`0c76f0d3c`, "isolate gem toolchain by ABI", itself following `457bfe3fd`, "resolve Bundler
/// through active Ruby") before retraction, so a consumer scaffolded under an earlier release
/// carries an earlier exact string -- observed on real consumer repos scaffolded at different
/// alef versions: `"bundle exec rubocop"` (oldest), `"BUNDLE_PATH=vendor/bundle ruby -S bundle
/// exec ruby -S rubocop"` (post-hardening). Listed oldest-first per hook; order carries no
/// meaning to the match below, which checks the whole set. ~keep
const STALE_UNRUNNABLE_SNAPSHOT_HOOK_RUNS: &[(&str, &[&str])] = &[
    (
        "rubocop",
        &[
            "bundle exec rubocop",
            "ruby -S bundle exec rubocop",
            "BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop",
        ],
    ),
    (
        "steep",
        &[
            "bundle exec steep check",
            "ruby -S bundle exec steep check",
            "BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S steep check",
        ],
    ),
    ("dart-analyze", &["dart analyze"]),
    ("dart-e2e-analyze", &["dart analyze"]),
];

/// Remove any pre-existing `[hooks.pre-commit.commands.<name>]` table, for
/// `name` in `{rubocop, steep, dart-analyze, dart-e2e-analyze}`, left behind by `8ed9ad8d4`
/// ("drop pre-commit hooks that cannot run in poly's snapshot") retracting all four from
/// [`crate::scaffold::scaffold_poly_config`] -- the same reachability gap
/// [`migrate_poly_toml_drop_snippet_hook`] closes for `alef-snippets`, documented there in full:
/// `merge_managed_toml_core`'s prune pass tracks and removes only ARRAY values, never a whole
/// TABLE alef stops emitting, so a consumer scaffolded while any of these four still rendered
/// keeps re-merging it forever. Every commit these hooks run on shells out to a dependency graph
/// (`bundle`, `dart pub get`) poly's isolated staged snapshot never materializes, failing `poly
/// lint`/pre-commit with a resolution error rather than a lint finding.
///
/// Guarded per-hook on the table's own `run` command matching one of
/// [`STALE_UNRUNNABLE_SNAPSHOT_HOOK_RUNS`]'s known strings for that name -- the only ones alef
/// itself ever emitted there -- AND `workspace = true`, the only mode `workspace_hook` ever set.
/// Matching on a table's name alone would risk removing a consumer's own, differently-configured
/// same-named command; this guard leaves that untouched, exactly as
/// [`migrate_poly_toml_drop_snippet_hook`] does for its one table. Silent (returns `Ok(false)`)
/// on a missing `poly.toml`, unparsable TOML, a `poly.toml` with no such tables, or a file where
/// none of the four still matches (idempotent: nothing left to remove on a second pass). ~keep
pub(crate) fn migrate_poly_toml_drop_unrunnable_snapshot_hooks(base_dir: &Path) -> anyhow::Result<bool> {
    let path = crate::cli::pipeline::generate::write::contained_output_path(base_dir, POLY_CONFIG_RELATIVE.as_ref())?;
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut doc) = existing.parse::<toml_edit::DocumentMut>() else {
        return Ok(false);
    };

    let commands = doc
        .as_table_mut()
        .get_mut("hooks")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|hooks| hooks.get_mut("pre-commit"))
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|pre_commit| pre_commit.get_mut("commands"))
        .and_then(toml_edit::Item::as_table_mut);
    let Some(commands) = commands else {
        return Ok(false);
    };

    let mut changed = false;
    for (name, known_runs) in STALE_UNRUNNABLE_SNAPSHOT_HOOK_RUNS {
        let is_stale_hook = commands
            .get(name)
            .and_then(toml_edit::Item::as_table)
            .is_some_and(|hook| {
                hook.get("run")
                    .and_then(toml_edit::Item::as_str)
                    .is_some_and(|run| known_runs.contains(&run))
                    && hook.get("workspace").and_then(toml_edit::Item::as_bool) == Some(true)
            });
        if is_stale_hook {
            commands.remove(name);
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }

    let parent = path.parent().context("poly.toml path has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;
    std::io::Write::write_all(&mut temporary, doc.to_string().as_bytes())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    // Fires only after the replace above already succeeded: a completed self-heal, not an
    // outstanding problem. ~keep
    tracing::info!(
        path = %path.display(),
        "repaired pre-existing poly.toml: removed retracted pre-commit hooks that cannot pass \
         poly's isolated staged snapshot"
    );
    Ok(true)
}
