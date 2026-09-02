//! Regression coverage for [`migrate_poly_toml_drop_snippet_hook`]: `poly.toml`'s managed merge
//! unions and prunes array values but never retracts a whole table alef stops emitting, so an
//! already-scaffolded consumer keeps re-merging the retracted `alef-snippets` pre-commit hook
//! forever. See the migration's own doc for the full defect.

use super::*;

const STALE_POLY_TOML: &str = "[discovery]\nexclude = []\n\n[hooks.pre-commit.commands.alef-snippets]\n\
     run = \"alef snippets check --strict --cache off\"\n\
     root = \".\"\n\
     workspace = true\n\
     files = \"{alef.toml,fixtures/**/*.json,docs/snippets/**}\"\n";

#[test]
fn should_remove_the_retracted_alef_snippets_pre_commit_hook() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("poly.toml"), STALE_POLY_TOML).expect("write stale poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(
        changed,
        "the known-stale alef-snippets hook must be reported as changed"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read migrated file");
    assert!(
        !on_disk.contains("alef-snippets"),
        "the retracted hook table must be gone: {on_disk}"
    );
    assert!(
        !on_disk.contains("alef snippets check"),
        "the retracted hook command must be gone: {on_disk}"
    );
    // The rest of the file -- untouched tables -- must survive.
    assert!(on_disk.contains("[discovery]"));
    toml::from_str::<toml::Value>(&on_disk).expect("migrated poly.toml must still parse");

    let changed_again = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

/// Regression coverage for the real consumer variant observed in `crawlberg/poly.toml`: the same
/// retracted `alef snippets check --strict --cache off` invocation, wrapped in a `sh -c` guard
/// that skips the check when `alef` is absent from `PATH` (a hardening added while the hook was
/// still alef's own, for a lint job that never installs alef). A byte-exact match on
/// [`STALE_SNIPPET_HOOK_RUN`] never recognises this shape -- the hardening drifted the table off
/// the one string the migration used to key off, so it was misclassified as consumer-repurposed
/// and silently preserved forever.
#[test]
fn should_remove_the_stale_hook_wrapped_in_a_consumers_path_guard() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[discovery]\nexclude = []\n\n\
         [hooks.pre-commit.commands.alef-snippets]\n\
         run = \"sh -c 'command -v alef >/dev/null 2>&1 && exec alef snippets check --strict --cache off || echo \\\"alef not on PATH - snippet check runs in the ci-lint Alef snippets job\\\"'\"\n\
         root = \".\"\n\
         workspace = true\n\
         files = \"{alef.toml,fixtures/**/*.json,docs-site/src/snippets/**}\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(
        changed,
        "the retracted hook wrapped in a consumer's PATH guard must still be recognised as stale"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read migrated file");
    assert!(
        !on_disk.contains("alef-snippets"),
        "the retracted hook table must be gone: {on_disk}"
    );
    assert!(
        !on_disk.contains("alef snippets check"),
        "the retracted hook command must be gone: {on_disk}"
    );
    assert!(on_disk.contains("[discovery]"));
    toml::from_str::<toml::Value>(&on_disk).expect("migrated poly.toml must still parse");

    let changed_again = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

#[test]
fn should_not_touch_a_consumer_added_pre_commit_command_of_a_different_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.rubocop]\n\
         run = \"bundle exec rubocop\"\n\
         root = \"packages/ruby\"\n\
         workspace = true\n\
         files = \"packages/ruby/**/*.rb\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(!changed, "no alef-snippets table present -- must be a no-op");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(
        on_disk, poly_toml,
        "an unrelated pre-commit command must survive byte-for-byte"
    );
}

#[test]
fn should_not_touch_a_same_named_hook_the_consumer_repurposed_with_a_different_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.alef-snippets]\n\
         run = \"echo custom hook the consumer wrote themselves\"\n\
         root = \".\"\n\
         workspace = true\n\
         files = \"docs/**\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("migration must not error");
    assert!(
        !changed,
        "a same-named table running a different command was never alef's own -- must be left alone"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(
        on_disk, poly_toml,
        "a consumer-repurposed alef-snippets table must survive byte-for-byte"
    );
}

#[test]
fn migrate_poly_toml_drop_snippet_hook_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let changed = migrate_poly_toml_drop_snippet_hook(dir.path()).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join("poly.toml").exists());
}

/// Regression coverage for [`migrate_poly_toml_drop_unrunnable_snapshot_hooks`]: `8ed9ad8d4`
/// ("drop pre-commit hooks that cannot run in poly's snapshot") retracted `rubocop`, `steep`,
/// `dart-analyze` and `dart-e2e-analyze` from `scaffold_poly_config` but shipped no matching
/// migration, so an already-scaffolded consumer's `poly.toml` keeps re-merging all four forever
/// -- the same reachability gap [`migrate_poly_toml_drop_snippet_hook`] closes for
/// `alef-snippets`.
const STALE_UNRUNNABLE_SNAPSHOT_POLY_TOML: &str = "[discovery]\nexclude = []\n\n\
     [hooks.pre-commit.commands.rubocop]\n\
     run = \"bundle exec rubocop\"\n\
     root = \"packages/ruby\"\n\
     workspace = true\n\
     files = \"packages/ruby/**/*.rb\"\n\n\
     [hooks.pre-commit.commands.steep]\n\
     run = \"bundle exec steep check\"\n\
     root = \"packages/ruby\"\n\
     workspace = true\n\
     files = \"packages/ruby/**/*.rb\"\n\n\
     [hooks.pre-commit.commands.dart-analyze]\n\
     run = \"dart analyze\"\n\
     root = \"packages/dart\"\n\
     workspace = true\n\
     files = \"packages/dart/**/*.dart\"\n\n\
     [hooks.pre-commit.commands.dart-e2e-analyze]\n\
     run = \"dart analyze\"\n\
     root = \"e2e/dart\"\n\
     workspace = true\n\
     files = \"e2e/dart/**/*.dart\"\n";

#[test]
fn should_remove_all_four_retracted_unrunnable_snapshot_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("poly.toml"), STALE_UNRUNNABLE_SNAPSHOT_POLY_TOML).expect("write stale poly.toml");

    let changed = migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("migration must not error");
    assert!(changed, "all four known-stale hooks must be reported as changed");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read migrated file");
    for hook in ["rubocop", "steep", "dart-analyze", "dart-e2e-analyze"] {
        assert!(!on_disk.contains(hook), "`{hook}` must be gone: {on_disk}");
    }
    // The rest of the file -- untouched tables -- must survive.
    assert!(on_disk.contains("[discovery]"));
    toml::from_str::<toml::Value>(&on_disk).expect("migrated poly.toml must still parse");

    let changed_again =
        migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("second pass must not error");
    assert!(
        !changed_again,
        "second pass over an already-migrated file must be a no-op"
    );
}

#[test]
fn should_remove_an_older_pre_hardening_spelling_of_the_rubocop_and_steep_hooks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.rubocop]\n\
         run = \"BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop\"\n\
         root = \"packages/ruby\"\n\
         workspace = true\n\
         files = \"packages/ruby/**/*.rb\"\n\n\
         [hooks.pre-commit.commands.steep]\n\
         run = \"ruby -S bundle exec steep check\"\n\
         root = \"packages/ruby\"\n\
         workspace = true\n\
         files = \"packages/ruby/**/*.rb\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("migration must not error");
    assert!(changed, "every known historical spelling must be recognised as stale");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read migrated file");
    assert!(!on_disk.contains("rubocop") && !on_disk.contains("steep"), "{on_disk}");
}

#[test]
fn should_not_touch_an_unrunnable_snapshot_hook_the_consumer_repurposed_with_a_different_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let poly_toml = "[hooks.pre-commit.commands.rubocop]\n\
         run = \"echo custom rubocop wrapper the consumer wrote themselves\"\n\
         root = \"packages/ruby\"\n\
         workspace = true\n\
         files = \"packages/ruby/**/*.rb\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("migration must not error");
    assert!(
        !changed,
        "a same-named table running a different command was never alef's own -- must be left alone"
    );

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(
        on_disk, poly_toml,
        "a consumer-repurposed rubocop table must survive byte-for-byte"
    );
}

#[test]
fn should_not_touch_a_known_run_command_missing_the_workspace_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Same `run` string alef itself once emitted, but `workspace` is absent -- `workspace_hook`
    // never emitted this shape, so it cannot be alef's own retracted table.
    let poly_toml = "[hooks.pre-commit.commands.dart-analyze]\n\
         run = \"dart analyze\"\n\
         root = \"packages/dart\"\n\
         files = \"packages/dart/**/*.dart\"\n";
    std::fs::write(dir.path().join("poly.toml"), poly_toml).expect("write poly.toml");

    let changed = migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("migration must not error");
    assert!(!changed, "missing `workspace = true` must never match");

    let on_disk = std::fs::read_to_string(dir.path().join("poly.toml")).expect("read file");
    assert_eq!(on_disk, poly_toml);
}

#[test]
fn migrate_poly_toml_drop_unrunnable_snapshot_hooks_is_a_no_op_when_file_does_not_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let changed = migrate_poly_toml_drop_unrunnable_snapshot_hooks(dir.path()).expect("must not error");
    assert!(!changed);
    assert!(!dir.path().join("poly.toml").exists());
}
