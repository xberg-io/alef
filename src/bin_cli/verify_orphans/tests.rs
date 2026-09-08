use super::find_orphaned_generated_files;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Join a `/`-separated relative path onto `root` one component at a time.
///
/// `Path::join("a/b")` keeps the literal `/` inside a single Windows component, so the
/// fixture path stringifies as `...\packages/java/dev/demo\File.java` while the walk under
/// test yields the all-`\` form. `PathBuf` comparison hides that (Windows `Path` treats both
/// separators as separators); the `String` comparison these assertions need does not. ~keep
fn join_components(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

fn marked_java_content() -> String {
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    crate::core::hash::inject_hash_line(&header, &"0".repeat(64))
}

/// Case (a): a file a backend stopped producing IS reported. Simulates the
/// `NodeContext.java`/`HtmlVisitor.java`/`VisitorBridge.java` case this module exists to catch --
/// the file is still on disk, still carries alef's marker, but the current run's managed-path set
/// (empty here, standing in for "no backend emits this anymore") does not include it.
#[test]
fn reports_a_marked_file_the_current_run_no_longer_produces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let dropped = package_dir.join("VisitorBridge.java");
    std::fs::write(&dropped, marked_java_content()).expect("write dropped file");

    let managed_paths: HashSet<PathBuf> = HashSet::new();
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths, &Default::default());

    assert_eq!(
        orphans,
        vec![dropped.display().to_string()],
        "a marked file absent from the managed-path set must be reported as an orphan"
    );
}

/// Case (b): a current, expected generated file is NOT reported -- it is marked on disk AND
/// present in the managed-path set, so it must never show up alongside a genuine orphan.
#[test]
fn does_not_report_a_marked_file_still_in_the_managed_path_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let current = package_dir.join("Bridge.java");
    std::fs::write(&current, marked_java_content()).expect("write current file");

    let managed_paths: HashSet<PathBuf> = HashSet::from([current.clone()]);
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths, &Default::default());

    assert!(
        orphans.is_empty(),
        "a file still in this run's managed-path set must never be reported: {orphans:?}"
    );
}

/// Case (c): a user-owned file with no alef marker, sitting in the same generated directory as
/// alef-managed output, must NEVER be reported -- this is the case that makes the check safe to
/// ship, since ownership is decided purely by the marker `collect_alef_hashes` already gates on.
#[test]
fn does_not_report_an_unmarked_user_owned_file_in_a_generated_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let current = package_dir.join("Bridge.java");
    std::fs::write(&current, marked_java_content()).expect("write current file");
    let hand_written = package_dir.join("UserExtensions.java");
    std::fs::write(&hand_written, "package dev.demo;\npublic class UserExtensions {}\n").expect("write hand file");

    let managed_paths: HashSet<PathBuf> = HashSet::from([current]);
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths, &Default::default());

    assert!(
        orphans.is_empty(),
        "an unmarked hand-written file must never be reported as an orphan, even sitting in a \
         generated directory: {orphans:?}"
    );
}

/// A known create-once seed (`rust-toolchain.toml`) must never be reported, even though it is
/// absent from `managed_paths` on every run after the one that created it -- `scaffold()` only
/// includes it in a run's surface when the path does not already exist on disk, so this is the
/// expected steady state for every consumer that has one, not a rare edge case.
#[test]
fn does_not_report_the_rust_toolchain_create_once_seed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    let seed = dir.path().join("rust-toolchain.toml");
    std::fs::write(&seed, hashed).expect("write seed");

    let managed_paths: HashSet<PathBuf> = HashSet::new();
    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths, &Default::default());

    assert!(
        orphans.is_empty(),
        "rust-toolchain.toml is a documented create-once seed and must never be reported: {orphans:?}"
    );
}

/// A file dropped by crate A but still legitimately owned by crate B in a multi-crate workspace
/// must not be reported, as long as the caller unions both crates' managed paths before calling
/// this function -- exactly as the doc comment requires.
#[test]
fn does_not_report_a_file_owned_by_a_different_crate_once_paths_are_unioned() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let owned_by_crate_b = package_dir.join("CrateBOnly.java");
    std::fs::write(&owned_by_crate_b, marked_java_content()).expect("write file");

    // Crate A's managed paths alone do not mention this file; the union with crate B's does.
    let crate_a_managed: HashSet<PathBuf> = HashSet::new();
    let crate_b_managed: HashSet<PathBuf> = HashSet::from([owned_by_crate_b.clone()]);
    let unioned: HashSet<PathBuf> = crate_a_managed.union(&crate_b_managed).cloned().collect();

    let orphans = find_orphaned_generated_files(dir.path(), &unioned, &Default::default());

    assert!(
        orphans.is_empty(),
        "a file owned by another crate in the union must not be reported: {orphans:?}"
    );
}

/// A path the repository declared under `[workspace.ownership] user_owned` must not be reported
/// as an orphan.
///
/// The declaration's whole purpose is to take a path out of alef's managed surface, so such a
/// path is *always* absent from `managed_paths` and would otherwise land here unconditionally.
/// Because `has_orphan_files` gates `alef verify`'s exit code, that turned the declaration into
/// a no-op for the case it exists to fix: a consumer moves a permanently-failing path out of
/// "stale"/"frozen", and it reappears under "orphaned", still failing, still with no reachable
/// remedy. `OwnershipConfig`'s module doc promises a declared path is "counted as a declared
/// skip rather than a failure"; this is the test that keeps that promise. ~keep
#[test]
fn a_declared_user_owned_path_is_not_reported_as_an_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let declared_path = package_dir.join("HandWritten.java");
    std::fs::write(&declared_path, marked_java_content()).expect("write declared file");

    let managed_paths: HashSet<PathBuf> = HashSet::new();
    let declared =
        crate::core::config::UserOwnedPaths::compile(&["packages/java/dev/demo/HandWritten.java".to_string()])
            .expect("compile user_owned patterns");

    let orphans = find_orphaned_generated_files(dir.path(), &managed_paths, &declared);

    assert!(
        orphans.is_empty(),
        "a user_owned path must never be reported as an orphan; got: {orphans:?}"
    );
}

/// The control: the declaration must not blanket-silence the orphan report for everything else.
#[test]
fn an_undeclared_sibling_is_still_reported_alongside_a_declared_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = join_components(dir.path(), "packages/java/dev/demo");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    std::fs::write(package_dir.join("HandWritten.java"), marked_java_content()).expect("declared");
    let dropped = package_dir.join("Dropped.java");
    std::fs::write(&dropped, marked_java_content()).expect("dropped");

    let declared =
        crate::core::config::UserOwnedPaths::compile(&["packages/java/dev/demo/HandWritten.java".to_string()])
            .expect("compile user_owned patterns");

    let orphans = find_orphaned_generated_files(dir.path(), &HashSet::new(), &declared);

    assert_eq!(
        orphans,
        vec![dropped.display().to_string()],
        "only the undeclared path may be reported"
    );
}
