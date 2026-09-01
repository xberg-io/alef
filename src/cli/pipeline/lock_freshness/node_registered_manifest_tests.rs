//! Coverage for the registered-path union that closes the structural blind spot in
//! [`check_generated_node_lock_freshness`]: `crates/*-wasm/package.json` (and, identically,
//! `crates/*-node/package.json`) is `generated_header: false` JSON, which cannot carry an
//! `alef:hash:` marker at all, so it is never a member of `current_gen_paths` -- the gate's
//! original, and only, source of paths to examine -- in any run, ever. Every fixture here
//! reproduces that shape structurally: a `package.json` recorded in the committed ownership
//! manifest (via the real production write path, [`crate::cli::cache::record_scaffold_owned_path`])
//! but deliberately ABSENT from the `generated_paths` argument, exactly as it would be on every
//! real `alef generate`/`alef all` run before and after this fix.

use super::*;

const REGISTERED_DIR_RELATIVE: &str = "crates/sample-wasm";
/// Independent of `NODE_DEPENDENCY`/`NODE_STALE_SPEC`/`NODE_FRESH_SPEC` on purpose: even though
/// this module is nested under the node gate's own test module and could see those consts, a
/// separate trio keeps this fixture file legible on its own without a reader having to track
/// which outer scope a bare identifier came from.
const WASM_DEPENDENCY: &str = "sample-pkg";
const WASM_STALE_SPEC: &str = "1.3.0";
const WASM_FRESH_SPEC: &str = "1.2.3";

fn registered_dir(root: &Path) -> PathBuf {
    root.join(REGISTERED_DIR_RELATIVE)
}

/// Writes a `package.json` at [`registered_dir`] and records it in the committed ownership
/// manifest the same way `write_scaffold_files_report`'s write guard does the first time it
/// creates an unmarkable, `generated_header: false` manifest -- see
/// `crate::cli::pipeline::generate::scaffold::write_scaffold_files_report`'s doc for that
/// guard. Using the real recording function, not a hand-rolled `.alef-ownership.toml`,
/// exercises the write side and the read side of the registration through the same
/// production code path the gate itself relies on.
fn write_and_register_package_json(root: &Path, specifier: &str) -> PathBuf {
    let dir = registered_dir(root);
    std::fs::create_dir_all(&dir).expect("create registered dir");
    let manifest = dir.join("package.json");
    std::fs::write(
        &manifest,
        format!(
            "{{\n  \"name\": \"sample-wasm\",\n  \"version\": \"0.1.0\",\n  \"private\": \
             false,\n  \"devDependencies\": {{\n    \"{WASM_DEPENDENCY}\": \"{specifier}\"\n  }}\n}}\n"
        ),
    )
    .expect("write registered package.json");
    crate::cli::cache::record_scaffold_owned_path(root, &manifest)
        .expect("record the manifest in the committed ownership manifest");
    manifest
}

fn write_pnpm_lock_v9_in(dir: &Path, locked_specifier: &str) {
    std::fs::write(
        dir.join("pnpm-lock.yaml"),
        format!(
            "lockfileVersion: '9.0'\n\nimporters:\n  .:\n    devDependencies:\n      \
             {WASM_DEPENDENCY}:\n        specifier: {locked_specifier}\n        version: \
             {locked_specifier}\n"
        ),
    )
    .expect("write pnpm-lock.yaml");
}

/// (a) RED: proves the gate's *original* path source examines ZERO paths for a
/// `generated_header: false` JSON manifest -- not "the gate returns a clean result", which is
/// also true of a gate that correctly found nothing wrong. `stampable_output_paths` is exactly
/// what production wires into `check_generated_node_lock_freshness`'s `generated_paths`
/// argument (see `bin_cli::all_commands`/`bin_cli::core_commands::generate`), and it filters on
/// [`crate::core::backend::GeneratedFile::carries_alef_marker`] -- always `false` for JSON,
/// which has no comment syntax to carry the marker at all. Asserting the COUNT is zero, not
/// merely that the gate later returns `None`, is the point: a `0` here is a set the gate never
/// even walks, which is a different failure than "walked it and found nothing".
#[test]
fn stampable_output_paths_examines_zero_paths_for_an_unmarkable_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let files = vec![crate::core::backend::GeneratedFile {
        path: PathBuf::from(format!("{REGISTERED_DIR_RELATIVE}/package.json")),
        content: "{\n  \"name\": \"sample-wasm\"\n}\n".to_string(),
        generated_header: false,
    }];

    let examined = crate::cli::pipeline::stampable_output_paths(&files, root);

    assert_eq!(
        examined.len(),
        0,
        "a generated_header: false JSON manifest carries no alef:hash: marker and JSON has no \
         comment syntax to hold one, so it must be structurally absent from the path set the \
         gate's original source examined -- examined: {examined:?}"
    );
}

/// (b) A manifest the gate knows about ONLY through the registered-path union -- never a
/// member of `generated_paths`, exactly the blind spot (a) proves -- whose dependency
/// disagrees with the lock's importer entry must now be reported as drift.
#[test]
fn registered_only_manifest_mismatch_is_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_and_register_package_json(root, WASM_STALE_SPEC);
    write_pnpm_lock_v9_in(&registered_dir(root), WASM_FRESH_SPEC);
    let generated: HashSet<PathBuf> = HashSet::new();

    let error = check_generated_node_lock_freshness(&generated, root)
        .expect("a manifest known only through the ownership registry must still be checked");
    let message = format!("{error:#}");

    assert!(
        message.contains(WASM_DEPENDENCY) && message.contains(WASM_STALE_SPEC) && message.contains(WASM_FRESH_SPEC),
        "message must name the drifted dependency and both specifiers: {message}"
    );
}

/// (c) MISSING importer: the sibling lock exists and parses, but records no `.` (root)
/// importer at all -- e.g. a monorepo lock that has not yet been regenerated to include this
/// project. `locked_node_specifiers` finds nothing for either bucket, so this must not be
/// reported: there is no recorded copy to disagree with, which is different from (and must not
/// be conflated with) a recorded copy that disagrees.
#[test]
fn registered_only_manifest_missing_lock_importer_is_not_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_and_register_package_json(root, WASM_STALE_SPEC);
    std::fs::write(
        registered_dir(root).join("pnpm-lock.yaml"),
        "lockfileVersion: '9.0'\n\nimporters:\n  packages/unrelated:\n    devDependencies:\n      \
         other-pkg:\n        specifier: 2.0.0\n        version: 2.0.0\n",
    )
    .expect("write pnpm-lock.yaml");
    let generated: HashSet<PathBuf> = HashSet::new();

    assert!(
        check_generated_node_lock_freshness(&generated, root).is_none(),
        "a lock with no importer entry for this project at all must not be reported as drift"
    );
}

/// (d) NEW importer: the manifest is registered but the lock has never seen this package at
/// all -- no `pnpm-lock.yaml` beside it yet, e.g. a crate scaffolded moments ago. Alef never
/// authors a lockfile, so an absent one is a deliberate, not-yet-installed state, not a defect.
#[test]
fn registered_only_manifest_with_no_lock_at_all_is_not_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_and_register_package_json(root, WASM_STALE_SPEC);
    let generated: HashSet<PathBuf> = HashSet::new();

    assert!(
        check_generated_node_lock_freshness(&generated, root).is_none(),
        "a package the lock has never seen (no lockfile at all yet) must not be reported as drift"
    );
}

/// (e) Idempotent refresh: running the freshness check twice over the same fixture, with
/// nothing on disk changing in between, must produce the identical result both times -- no
/// spurious drift appearing on a second pass, and no duplicate findings from the union of
/// `generated_paths` and the registered set double-counting the same directory.
#[test]
fn repeated_checks_over_an_unchanged_registered_fixture_agree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_and_register_package_json(root, WASM_STALE_SPEC);
    write_pnpm_lock_v9_in(&registered_dir(root), WASM_FRESH_SPEC);
    let generated: HashSet<PathBuf> = HashSet::new();

    let first = check_generated_node_lock_freshness(&generated, root).expect("first pass must report drift");
    let second = check_generated_node_lock_freshness(&generated, root).expect("second pass must report drift");

    assert_eq!(
        format!("{first:#}"),
        format!("{second:#}"),
        "two runs over an unchanged fixture must report the identical finding, not accumulate \
         duplicates or drift in wording"
    );

    let first_set = registered_unmarkable_manifest_dirs(root, "package.json");
    let second_set = registered_unmarkable_manifest_dirs(root, "package.json");
    assert_eq!(
        first_set, second_set,
        "the registered-directory set itself must be stable across repeated reads"
    );
    assert_eq!(
        first_set.len(),
        1,
        "exactly one registered directory is expected, not a duplicate entry: {first_set:?}"
    );
}

/// Clean-fixture counterpart to (e): idempotent refresh must also hold when there is nothing
/// to report, not only when there is.
#[test]
fn repeated_checks_over_a_clean_registered_fixture_stay_clean() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_and_register_package_json(root, WASM_FRESH_SPEC);
    write_pnpm_lock_v9_in(&registered_dir(root), WASM_FRESH_SPEC);
    let generated: HashSet<PathBuf> = HashSet::new();

    assert!(check_generated_node_lock_freshness(&generated, root).is_none());
    assert!(
        check_generated_node_lock_freshness(&generated, root).is_none(),
        "a second pass over the same clean fixture must stay clean"
    );
}
