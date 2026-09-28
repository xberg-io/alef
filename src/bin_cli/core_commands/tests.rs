use super::ensure_required_records_tracked;
use crate::bin_cli::args::Commands;
use crate::bin_cli::dispatch::DispatchContext;
use crate::cli::cache;
use std::path::Path;

/// `cache::OWNERSHIP_MANIFEST` is private to that module, so the name is spelled out
/// here; it is also the literal an operator has to type into `git add`, which is what
/// the assertions below are really about. ~keep
const OWNERSHIP_MANIFEST: &str = ".alef-ownership.toml";

fn init_git_work_tree(base_dir: &Path) -> Option<()> {
    let status = crate::test_support::git_command(base_dir)
        .args(["init", "--quiet"])
        .status()
        .ok()?;
    status.success().then_some(())
}

fn git_add(base_dir: &Path, relative: &str) {
    let status = crate::test_support::git_command(base_dir)
        .args(["add", "--", relative])
        .status()
        .expect("git add");
    assert!(status.success(), "git add {relative} failed");
}

/// The load-bearing assertion is the *status* flipping from failure to success across a
/// single `git add`, driven end to end by real files and a real git index. Asserting
/// only on the message text would keep passing even if the run never failed at all --
/// which is exactly the "check that examines nothing" defect this whole fix exists to
/// correct, since the notice it replaces printed a true sentence and changed no
/// outcome. ~keep
#[test]
fn verify_fails_on_an_untracked_required_record_and_passes_once_it_is_staged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    if init_git_work_tree(base).is_none() {
        return;
    }
    cache::record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");

    let error = ensure_required_records_tracked(&cache::untracked_required_records(base), false)
        .expect_err("an untracked required record must fail verification, not merely print");
    let message = error.to_string();
    assert!(
        message.contains(OWNERSHIP_MANIFEST),
        "the failure must name the offending record, got: {message}"
    );
    assert!(
        message.contains(&format!("git add {OWNERSHIP_MANIFEST}")),
        "the failure must carry the exact remedy command, got: {message}"
    );

    git_add(base, OWNERSHIP_MANIFEST);

    ensure_required_records_tracked(&cache::untracked_required_records(base), false)
        .expect("staging the record must make verification pass");
}

/// Outside a git work tree tracked-ness is unanswerable, so verification must not
/// invent a failure there -- an export tarball or a git-less container would fail
/// forever with nothing the operator could do. ~keep
#[test]
fn verify_passes_outside_a_git_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    cache::record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");
    assert!(base.join(OWNERSHIP_MANIFEST).is_file(), "sanity: the record exists");

    ensure_required_records_tracked(&cache::untracked_required_records(base), false)
        .expect("no repository to ask means no fault to report");
}

#[test]
fn report_only_downgrades_an_untracked_record_to_a_report() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path();
    if init_git_work_tree(base).is_none() {
        return;
    }
    cache::record_scaffold_owned_path(base, &base.join("packages/node/package.json")).expect("record");
    let untracked = cache::untracked_required_records(base);
    assert_eq!(
        untracked,
        vec![OWNERSHIP_MANIFEST],
        "sanity: without this the report-only assertion below would examine nothing"
    );

    ensure_required_records_tracked(&untracked, true).expect("--report-only keeps a successful exit status");
}

const DIFF_FIXTURE_SOURCE: &str = "pub fn greet(name: String) -> String {\n    name\n}\n";
const DIFF_FIXTURE_CARGO_TOML: &str = "[package]\nname = \"test-lib\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

/// `[crates.python.stubs]` is required for the stubs phase to emit anything and also pins
/// the public-API phase's output directory (see the identical fixture this mirrors,
/// `LANG_MANIFEST_FIXTURE_ALEF_TOML` in `all_commands_tests.rs`), so this crate's Python
/// output spans three phases -- bindings, stubs, and public API -- exactly like the real
/// consumer tree that measured `python 1/6`. ~keep
const DIFF_FIXTURE_ALEF_TOML: &str = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"

[crates.python]
module_name = "test_lib"

[crates.python.stubs]
output = "packages/python/test_lib"
"#;

fn write_diff_fixture_workspace(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture src directory");
    std::fs::write(root.join("src/lib.rs"), DIFF_FIXTURE_SOURCE).expect("write fixture source");
    std::fs::write(root.join("Cargo.toml"), DIFF_FIXTURE_CARGO_TOML).expect("write fixture Cargo.toml");
    std::fs::write(root.join("alef.toml"), DIFF_FIXTURE_ALEF_TOML).expect("write fixture alef.toml");
}

/// Regression for the second half of alef#158: `alef generate` already reconciles every
/// phase's alef-marked output into `<lang>.manifest` via `cache::write_lang_manifest` (see
/// `write_lang_manifest_records_the_full_union_once_every_phase_is_reconciled` in
/// `cli/pipeline/generate/generation.rs`), so a fresh `alef generate` run on this fixture
/// records all six Python files below -- the "N files emitted, N paths recorded" property,
/// proven through the real dispatch path rather than by constructing a manifest by hand.
///
/// `alef diff` is documented as "without writing", so it must never be able to move that
/// number. Before this fix, `Commands::Diff` called `pipeline::generate` with
/// `write_cache: true`, so its internal `write_lang_hash` unconditionally overwrote
/// `<lang>.manifest` with just the bindings phase's own file (`crates/test-lib-py/src/lib.rs`),
/// regressing the manifest `alef generate` had just built from 6 entries back down to 1 --
/// the exact ratio measured on the real consumer tree. This is the mandatory control: the
/// backend already recorded correctly (via `alef generate`) before `alef diff` ran, and its
/// recorded set must be byte-identical after `alef diff` runs, proving the fix rather than
/// merely a manifest that happens to be non-empty. The wiring under test here is generic
/// over every language `Commands::Diff` iterates, so this one fixture stands in for all of
/// python/node/ruby/elixir/php/wasm rather than repeating the same assertion four times. ~keep
#[test]
fn diff_does_not_regress_a_language_manifest_generate_already_reconciled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    super::handle(
        Commands::Generate {
            lang: None,
            clean: false,
            skip_frb: false,
            // Lenient, deliberately: this fixture run must not depend on which formatters the
            // machine running the suite has installed. ~keep
            strict: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef generate must succeed against the fixture");

    let mut before = cache::read_lang_manifest("test-lib", "python");
    before.sort();
    let mut expected = vec![
        root.join("crates/test-lib-py/src/lib.rs"),
        root.join("packages/python/test_lib/test_lib.pyi"),
        root.join("packages/python/test_lib/options.py"),
        root.join("packages/python/test_lib/api.py"),
        root.join("packages/python/test_lib/exceptions.py"),
        root.join("packages/python/test_lib/__init__.py"),
    ];
    expected.sort();
    assert_eq!(
        before, expected,
        "sanity: alef generate must record all six alef-marked Python files before alef diff \
         ever runs, or the assertion below would pass even if diff wiped the manifest clean"
    );

    super::handle(Commands::Diff { exit_code: false }, &context).expect("alef diff must succeed");

    let mut after = cache::read_lang_manifest("test-lib", "python");
    after.sort();
    assert_eq!(
        after, before,
        "alef diff is documented as \"without writing\" and must not regress \
         <lang>.manifest -- got {after:?}, expected the unchanged pre-diff set {before:?}"
    );
}

fn verify_command() -> Commands {
    Commands::Verify {
        exit_code: false,
        report_only: false,
        compile: false,
        lint: false,
        lang: None,
    }
}

/// Drives `alef verify`'s orphan finding through the real `Commands::Verify` dispatch path
/// against a real `alef generate` output tree -- not a direct call into
/// `verify_orphans::find_orphaned_generated_files`, which the unit tests in
/// `verify_orphans::tests` already cover in isolation. A unit test proves the diff logic is
/// correct; it does not prove the CLI ever reaches it. This is the "implemented, tested, but
/// never wired into the command that is supposed to call it" shape the module doc for
/// `verify_orphans` exists to close, so the regression this guards against is the wiring,
/// not the diff. ~keep
#[test]
fn verify_command_reports_and_fails_on_a_real_orphaned_generated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    // `Commands::All`, not `Commands::Generate`: `alef verify`'s missing-file check spans
    // every stage in `collect_managed_surface` (bindings, scaffold, e2e, README, docs), so
    // `alef generate` alone always leaves README/docs reported missing regardless of this
    // fix -- a pre-existing, correct, and unrelated finding. Only `alef all`'s full pass
    // produces a tree the sanity check below can honestly call clean.
    //
    // `crate::bin_cli::all_commands::handle`, not `super::handle`: `core_commands::handle`'s
    // match has no `Commands::All` arm, so in the real binary `dispatch::run`'s
    // chain-of-responsibility loop (`src/bin_cli/dispatch.rs`) passes `All` straight through
    // core_commands untouched and on to `all_commands::handle`, which is the one that
    // actually owns it. `super::handle(Commands::All { .. }, ..)` would return `Ok(Some(_))`
    // having done nothing -- an `Ok` a careless `.expect` would not catch -- which is exactly
    // why this bootstrap step names the real owning handler instead. ~keep
    crate::bin_cli::all_commands::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: true,
            skip_snippet_validation: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed against the fixture");

    // Sanity: immediately after a real `alef all`, a real `alef verify` against the
    // same tree must pass. Without this, a failure below could not be pinned on the orphan
    // this test injects -- it could equally be a fixture that was never clean to begin with.
    // This is also the regression control for the `bindings_stage` cache fix directly above
    // this test in the diff: before it, `packages/python/lib.rs` -- already cached from the
    // `alef all` run that just wrote it -- was silently dropped from `collect_managed_surface`
    // and reported as an orphan right here, on the exact tree `alef verify` is supposed to
    // pass on. ~keep
    super::handle(verify_command(), &context)
        .expect("alef verify must pass on a tree alef all just produced, before any orphan is injected");

    // Simulate a backend that stopped emitting a file it used to (the Java visitor-file
    // case `verify_orphans`'s module doc describes): copy an existing alef-marked file's
    // real bytes -- header and hash intact -- to a path no current backend's output would
    // include. `api.py` is one of the six paths `diff_does_not_regress_a_language_manifest_
    // generate_already_reconciled` already proves `alef generate` writes for this fixture.
    let current = root.join("packages/python/test_lib/api.py");
    let stale = root.join("packages/python/test_lib/legacy_visitor.py");
    std::fs::copy(&current, &stale).expect("plant a stale alef-marked file");

    // `Commands` carries no `Debug` impl, so `Result<Option<Commands>, _>` cannot be
    // `expect_err`/`{:?}`-formatted directly; `.err()` discards the `Ok` payload and hands
    // back a plain `anyhow::Error`, which does implement `Debug`/`Display`.
    let error = super::handle(verify_command(), &context)
        .err()
        .expect("alef verify must fail once an alef-marked file is orphaned on disk");
    let message = error.to_string();
    assert!(
        message.contains("out of date"),
        "alef verify's real failure path must be the one under test, got: {message}"
    );

    // `output::line` writes straight to stdout (see `bin_cli::output`), not through
    // anything this in-process test can intercept, so causation is pinned by timing
    // instead: verify passed on this exact tree immediately before the copy above and will
    // pass again immediately after the removal below, so the one file present only in
    // between is what the failure in between is attributable to. The orphan module's own
    // unit tests (`verify_orphans::tests`) are what assert on the specific path text.
    let report_only_error = super::handle(
        Commands::Verify {
            exit_code: false,
            report_only: true,
            compile: false,
            lint: false,
            lang: None,
        },
        &context,
    )
    .err();
    assert!(
        report_only_error.is_none(),
        "--report-only must downgrade the same orphan finding to a non-fatal report, got: \
         {report_only_error:?}"
    );

    std::fs::remove_file(&stale).expect("remove the planted orphan");
    super::handle(verify_command(), &context)
        .expect("alef verify must pass again once the orphaned file is removed from disk");
}

/// alef#268: an `alef all --clean` killed mid-run leaves the tree missing files it just
/// removed and never got to rewrite, with nothing recording that the run did not finish.
/// `alef verify`'s next signal then names those absences, indistinguishable from ordinary
/// staleness. This drives the real `Commands::Verify` dispatch path -- not a direct call into
/// `cache::generation_record` -- against a real `alef all` output tree, the same shape as the
/// orphan test above this one, so the assertion is about the wiring, not just the marker logic.
///
/// The interruption itself is simulated at the level that matters: `mark_generation_in_progress`
/// is a plain file write, so a process actually being killed and a test calling it directly are
/// indistinguishable to every reader of the marker (`alef verify` included) -- that is the whole
/// point of choosing a file write over a `Drop` guard. Deleting the crate's bindings file
/// afterward stands in for the file `--clean` removed and never got to rewrite -- deliberately
/// the bindings-phase output, not a stubs/public-API one: only the bindings-phase cache check in
/// `all_commands.rs` (`generated_files_match_disk`) re-derives from actual disk presence, so this
/// is also what makes the recovery `alef all` call below actually rewrite the file rather than
/// trust a still-valid content-hash cache that never looked at the disk. ~keep
#[test]
fn verify_reports_an_incomplete_generation_run_instead_of_ordinary_staleness() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    crate::bin_cli::all_commands::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: true,
            skip_snippet_validation: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed against the fixture");

    // Positive control, run BEFORE any marker is written: a completed run must report
    // normally, with no incomplete-run signal at all. Without this, the failure asserted
    // below could not be pinned on the marker this test writes -- it could equally be a
    // fixture that was never clean to begin with, or a fix that always claims "incomplete"
    // regardless of the marker's presence. ~keep
    super::handle(verify_command(), &context)
        .expect("alef verify must pass on a tree alef all just produced, before any interruption");

    // Simulate the interruption: a marker for this crate with no matching completion, plus a
    // file `--clean` would have removed and not yet rewritten.
    cache::generation_record::mark_generation_in_progress(&root, "test-lib").expect("mark test-lib as in progress");
    let removed = root.join("crates/test-lib-py/src/lib.rs");
    std::fs::remove_file(&removed).expect("simulate a file --clean removed but never rewrote");

    let error = super::handle(verify_command(), &context)
        .err()
        .expect("alef verify must fail once a crate's last generation run did not complete");
    let message = error.to_string();
    assert!(
        message.contains("did not complete") && message.contains("test-lib"),
        "the failure must name the interrupted crate and diagnose it as an unfinished run, got: \
         {message}"
    );
    assert!(
        !message.contains("out of date"),
        "an incomplete run must not be reported through the generic staleness gate -- the whole \
         point is a distinct diagnosis; got: {message}"
    );

    // --report-only must downgrade this the same way it downgrades every other verify failure.
    let report_only_error = super::handle(
        Commands::Verify {
            exit_code: false,
            report_only: true,
            compile: false,
            lint: false,
            lang: None,
        },
        &context,
    )
    .err();
    assert!(
        report_only_error.is_none(),
        "--report-only must downgrade the incomplete-run finding to a non-fatal report, got: \
         {report_only_error:?}"
    );

    // Recovery: rerunning `alef all` regenerates the missing file and clears the marker, so
    // the crate becomes indistinguishable from one whose run was never interrupted.
    crate::bin_cli::all_commands::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: true,
            skip_snippet_validation: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed again and clear the in-progress marker");
    assert!(
        !cache::generation_record::incomplete_crate_names(&root, ["test-lib"]).contains(&"test-lib".to_string()),
        "a successful rerun must clear the marker, not merely regenerate the missing file"
    );
    super::handle(verify_command(), &context)
        .expect("alef verify must pass again once the interrupted run has been completed");
}

/// Regression test for the html-to-markdown freshness-gate incident: `alef verify`'s disk walk
/// must never open a directory git considers ignored. Before `verify_gitignore::gitignored_dirs`
/// existed, a dependency-fetch cache or build-output directory sitting anywhere in the tree --
/// gitignored, untracked, populated by a tool other than this run's own `alef generate` -- was
/// opened like any other directory. A source file inside it that happened to carry a real (but
/// stale, foreign) `alef:hash:` marker was then reported stale/orphaned on a tree that had
/// otherwise just been cleanly regenerated, making the CI freshness gate permanently unable to
/// pass no matter how many times `alef all` ran. ~keep
#[test]
fn verify_passes_with_zero_findings_despite_a_gitignored_dependency_cache_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    if init_git_work_tree(&root).is_none() {
        return;
    }
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    // A generic dependency-fetch cache, gitignored the same shape as a real consumer's package
    // manager cache: a pattern with no interior slash, so it matches `vendor-cache/` at any
    // depth, not only at the repo root.
    std::fs::write(root.join(".gitignore"), "vendor-cache/\n").expect("write .gitignore");
    // Nested under a name that is not already in `VERIFY_SKIP_DIRS`'s hand-maintained list
    // (`target`, `build`, `vendor`, ...) -- this test exists to prove the *gitignore-aware*
    // pruning, not to accidentally pass because the hand-maintained list already covered it.
    let cache_dir = root.join("test_apps/native/vendor-cache/fetched-dep-9.9.9/src");
    std::fs::create_dir_all(&cache_dir).expect("create the vendored dependency cache directory");
    std::fs::write(
        cache_dir.join("legacy.py"),
        "# generated by alef\n# alef:hash:0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .expect("plant a foreign alef-marked file inside the gitignored cache");

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    crate::bin_cli::all_commands::handle(
        Commands::All {
            clean: false,
            clobber_create_once_seeds: false,
            strict: false,
            skip_frb: true,
            skip_snippet_validation: false,
            skip_compile: false,
        },
        &context,
    )
    .expect("alef all must succeed against the fixture");

    // `alef all` writes `.alef-ownership.toml`/`.alef-toml-merge-provenance.toml`, which
    // `untracked_required_records` (see `verify_fails_on_an_untracked_required_record_and_
    // passes_once_it_is_staged` above) refuses to certify as fresh while uncommitted -- a
    // real consumer commits these in the same change as the regenerated bindings, so a
    // fixture proving "freshly regenerated" must do the same before calling `alef verify`.
    git_add(&root, ".");

    super::handle(verify_command(), &context).expect(
        "alef verify must pass with zero findings on a freshly regenerated tree, even with a \
         gitignored dependency-cache directory containing a stale, alef-marked file sitting \
         alongside it",
    );
}

/// THE STRUCTURAL PROOF that `alef docs --skip-snippet-validation` actually reaches
/// `docs::generate_docs_stage_without_snippet_compile_validation` through the real CLI
/// dispatch path (`core_commands::handle` -> `core_commands::docs::handle`), not merely
/// that the flag parses.
///
/// Mirrors `generate_docs_stage_without_snippet_compile_validation_never_runs_the_validator`
/// in `docs/tests/generated_stage.rs`, which proves the same thing one layer down for the
/// function directly; this test is the CLI-surface half of that proof. Deliberately uses a
/// syntactically invalid JSON snippet under `validation_level = "syntax"`: `JsonValidator`
/// never spawns a process, so the assertion holds identically on a machine with every
/// referenced toolchain missing -- the same false-green class `alef adopt`'s 90-minute
/// regression fell into (see `generate_docs_stage_without_snippet_compile_validation`'s doc
/// comment).
///
/// Without `--skip-snippet-validation`, `alef docs` must fail on the invalid snippet --
/// that failure is what proves this fixture genuinely reaches the validator, so a pass with
/// the flag on can only mean the compile-validation step was skipped, never that it ran and
/// happened to pass. ~keep
#[test]
fn docs_skip_snippet_validation_flag_bypasses_the_real_validator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    std::fs::create_dir_all(root.join("docs/snippets/json")).expect("create snippet directory");
    std::fs::write(
        root.join("docs/snippets/json/example.md"),
        "```json\n{ this is not valid json\n```\n",
    )
    .expect("write invalid JSON snippet");
    std::fs::write(
        root.join("alef.toml"),
        format!(
            "{DIFF_FIXTURE_ALEF_TOML}\n[workspace.docs.snippets]\ndirs = [\"docs/snippets\"]\nvalidation_level = \"syntax\"\n"
        ),
    )
    .expect("overwrite fixture alef.toml with a docs.snippets section");
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };

    // `Commands` derives only `clap::Subcommand`, not `Debug`, so `Result::expect_err` (which
    // needs `Debug` on the `Ok` side to build its panic message) cannot be used directly on a
    // `Result<Option<Commands>, _>` -- match by hand instead. ~keep
    let validated_err = match super::handle(
        Commands::Docs {
            lang: None,
            output: None,
            skip_snippet_validation: false,
        },
        &context,
    ) {
        Err(error) => error,
        Ok(_) => panic!(
            "the invalid JSON snippet must fail validation when `alef docs` runs it -- if this \
             passes, the fixture never reaches the validator and the assertion below proves nothing"
        ),
    };
    assert!(
        validated_err.to_string().contains("snippet validation failed"),
        "expected a snippet-validation failure naming the invalid JSON, got: {validated_err:#}"
    );
    assert_reference_pages_are_stamped(&root, "the failing run above already wrote these pages (alef#475)");

    super::handle(
        Commands::Docs {
            lang: None,
            output: None,
            skip_snippet_validation: true,
        },
        &context,
    )
    .expect(
        "`alef docs --skip-snippet-validation` must never invoke the same invalid-JSON \
         snippet's validator -- its failure above proves this fixture reaches the validator \
         when it runs, so success here can only mean the compile-validation step was skipped",
    );
}

/// Every `docs/reference/*.md` page on disk under `root` must carry an `alef:hash:` line.
///
/// `alef verify` claims a file by the marker in its leading lines and then holds it to its
/// embedded stamp (`helpers::stale_among`), so a written-but-unstamped page reads as
/// permanently drifted. The page count is asserted first on purpose: an empty
/// `docs/reference` would otherwise satisfy a bare "all pages are stamped" loop without
/// examining a single file. ~keep
fn assert_reference_pages_are_stamped(root: &Path, context: &str) {
    let reference_dir = root.join("docs/reference");
    let mut pages: Vec<std::path::PathBuf> = std::fs::read_dir(&reference_dir)
        .unwrap_or_else(|error| panic!("{} must exist: {error}", reference_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    pages.sort();
    assert!(
        !pages.is_empty(),
        "no reference pages were written to {} -- the stamping assertion below would examine \
         nothing ({context})",
        reference_dir.display()
    );

    let unstamped: Vec<String> = pages
        .iter()
        .filter(|path| {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            crate::core::hash::content_has_alef_marker(&content) && !content.contains("alef:hash:")
        })
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        unstamped.is_empty(),
        "{} of {} alef-marked reference page(s) carry no `alef:hash:` line, so `alef verify` \
         reports them drifted forever ({context}): {}",
        unstamped.len(),
        pages.len(),
        unstamped.join(", ")
    );
}

/// The healthy-run control for [`assert_reference_pages_are_stamped`]: with no snippet
/// configuration to fail on, `alef docs` succeeds and every page it writes is already stamped,
/// so the sibling assertion in
/// [`docs_skip_snippet_validation_flag_bypasses_the_real_validator`] is measuring the failure
/// path specifically and not a stamp that was never written on any path.
///
/// Scoped to "is the `alef:hash:` line present" and nothing further. Whether those bytes also
/// survive a `poly fmt` pass unchanged is a separate question about the markdown format
/// prediction (`helpers::format_drift`), which this says nothing about. ~keep
#[test]
fn a_successful_docs_run_stamps_every_reference_page() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().unwrap_or_else(|_| dir.path().to_path_buf());
    write_diff_fixture_workspace(&root);
    let _cwd = crate::test_support::CwdGuard::enter(&root);

    let context = DispatchContext {
        config_path: root.join("alef.toml"),
        crate_filter: Vec::new(),
    };
    super::handle(
        Commands::Docs {
            lang: None,
            output: None,
            skip_snippet_validation: false,
        },
        &context,
    )
    .expect("a docs run with no snippet configuration has nothing to fail on");

    assert_reference_pages_are_stamped(&root, "a successful `alef docs` run");
}
