use super::*;
use crate::test_support::{git_add, git_init, write_file};
use tracing_test::traced_test;

fn config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    }
}

/// Regression for #178: a filtered `generate --lang python` must not return
/// any other language's directory, nor the unconditional wasm/typescript
/// roots. Sweeping them with a python-only keep set was deleting every other
/// binding's still-valid output.
#[test]
fn filtered_run_excludes_other_languages_and_ts_fallback_roots() {
    let base = Path::new("/base");
    let roots = generate_sweep_roots(&[Language::Python], true, &config(), base);

    assert!(
        roots.contains(&base.join("packages/python")),
        "must keep the requested language"
    );
    assert!(
        !roots.contains(&base.join("packages/ruby")),
        "must not touch an unrequested language"
    );
    assert!(
        !roots.contains(&base.join("packages/wasm")),
        "must not add the wasm fallback root on a filtered run"
    );
    assert!(
        !roots.contains(&base.join("packages/typescript")),
        "must not add the typescript fallback root on a filtered run"
    );
}

#[test]
fn unfiltered_run_includes_all_languages_and_ts_fallback_roots() {
    let base = Path::new("/base");
    let roots = generate_sweep_roots(&[Language::Python, Language::Ruby], false, &config(), base);

    assert!(roots.contains(&base.join("packages/python")));
    assert!(roots.contains(&base.join("packages/ruby")));
    assert!(
        roots.contains(&base.join("packages/wasm")),
        "unfiltered run keeps the wasm fallback root"
    );
    assert!(
        roots.contains(&base.join("packages/typescript")),
        "unfiltered run keeps the typescript fallback root"
    );
}

#[test]
fn manifest_sweep_removes_only_prior_managed_orphans_in_selected_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let selected = dir.path().join("python");
    let other = dir.path().join("ruby");
    std::fs::create_dir_all(&selected).expect("selected root");
    std::fs::create_dir_all(&other).expect("other root");
    let managed = selected.join("orphan.py");
    let handwritten = selected.join("notes.py");
    let unselected = other.join("orphan.rb");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&managed, &hashed).expect("managed");
    std::fs::write(&handwritten, "handwritten\n").expect("handwritten");
    std::fs::write(&unselected, &hashed).expect("unselected");

    let previous = vec![managed.clone(), handwritten.clone(), unselected.clone()];
    let removed =
        sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[selected], &[]).expect("manifest sweep");

    assert_eq!(removed, 1);
    assert!(!managed.exists());
    assert!(handwritten.exists());
    assert!(unselected.exists());
}

#[test]
fn manifest_sweep_preserves_non_header_generator_manifests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let selected = dir.path().join("swift");
    let manifest = selected.join("rust/Cargo.toml");
    std::fs::create_dir_all(manifest.parent().expect("manifest parent")).expect("selected root");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&manifest, hashed).expect("manifest");

    let previous = vec![manifest.clone()];
    let keep = std::collections::HashSet::from([manifest.clone()]);
    let removed = sweep_manifest_orphans(&previous, &keep, &[selected], &[]).expect("manifest sweep");

    assert_eq!(removed, 0);
    assert!(manifest.exists());
}

/// A manifest alef wrote last run and does not write this run IS reclaimed,
/// even though `composer.json` never carries a marker or hash — this is the
/// core "previous-manifest provenance" case from orphan reports filed by two
/// independent consumer repos: a co-located/split layout toggle stops emitting
/// a `composer.json` copy, and the old copy must not linger forever.
#[test]
fn manifest_sweep_reclaims_unmarkable_manifest_absent_from_current_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let composer_json = package_dir.join("composer.json");
    std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");

    let previous = vec![composer_json.clone()];
    let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir], &[])
        .expect("manifest sweep");

    assert_eq!(removed, 1, "unmarkable manifest absent from this run must be reclaimed");
    assert!(!composer_json.exists());
}

/// A manifest alef writes on both runs is NOT touched: it is present in
/// `keep` (this run's own output), so the `keep.contains(path)` guard must
/// short-circuit before `path_is_reclaimable` (and therefore before the
/// unmarkable-manifest allowlist) is ever consulted.
#[test]
fn manifest_sweep_preserves_unmarkable_manifest_still_written_this_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let composer_json = package_dir.join("composer.json");
    std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");

    let previous = vec![composer_json.clone()];
    let keep = std::collections::HashSet::from([composer_json.clone()]);
    let removed = sweep_manifest_orphans(&previous, &keep, &[package_dir], &[]).expect("manifest sweep");

    assert_eq!(removed, 0, "manifest still written this run must survive");
    assert!(composer_json.exists());
}

/// Reclaiming an orphaned `composer.json` cascades to its `composer.lock`
/// sibling in the same directory — the concrete instance seen in a consumer
/// repo: a lockfile whose manifest was removed long ago must not linger forever
/// either, even though alef never authored the lockfile itself.
#[test]
fn manifest_sweep_reclaims_lockfile_sibling_of_reclaimed_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let composer_json = package_dir.join("composer.json");
    let composer_lock = package_dir.join("composer.lock");
    std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");
    std::fs::write(&composer_lock, "{\n  \"_readme\": []\n}\n").expect("composer.lock");

    let previous = vec![composer_json.clone()];
    let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir], &[])
        .expect("manifest sweep");

    assert_eq!(
        removed, 2,
        "both the manifest and its lockfile sibling must be reclaimed"
    );
    assert!(!composer_json.exists());
    assert!(
        !composer_lock.exists(),
        "orphaned lockfile must be reclaimed alongside its manifest"
    );
}

/// THE risk-bounding test: a hand-authored `composer.lock` at a path alef
/// has never emitted a `composer.json` for must survive `--clean`. This is
/// the case the design explicitly calls out as capable of real damage — a
/// predicate that was widened to "any lockfile in a package dir" (instead of
/// "a lockfile beside a manifest this call just reclaimed by provenance")
/// would delete it, because `previous_paths` is empty here: alef has no
/// record of ever writing anything in this directory.
#[test]
fn manifest_sweep_preserves_hand_authored_lockfile_alef_never_emitted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    // No composer.json ever existed here — this project was never scaffolded
    // by alef, only a developer ran `composer install` by hand.
    let hand_authored_lock = package_dir.join("composer.lock");
    std::fs::write(&hand_authored_lock, "{\n  \"_readme\": [\"hand rolled\"]\n}\n").expect("hand-authored lock");

    let previous: Vec<PathBuf> = Vec::new();
    let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir], &[])
        .expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "a lockfile with no reclaimed manifest sibling must never be touched"
    );
    assert!(
        hand_authored_lock.exists(),
        "hand-authored lockfile alef never emitted must survive"
    );
}

/// Second half of the risk-bounding guarantee: even when a real
/// `composer.json` exists beside the lockfile, the lockfile survives if the
/// manifest is still being written this run (`keep`) — the cascade must only
/// fire when the manifest itself was actually reclaimed, never merely
/// because a lockfile happens to sit beside *some* manifest.
#[test]
fn manifest_sweep_preserves_lockfile_when_its_manifest_is_still_kept() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let composer_json = package_dir.join("composer.json");
    let composer_lock = package_dir.join("composer.lock");
    std::fs::write(&composer_json, "{\n  \"name\": \"acme/demo\"\n}\n").expect("composer.json");
    std::fs::write(&composer_lock, "{\n  \"_readme\": []\n}\n").expect("composer.lock");

    let previous = vec![composer_json.clone()];
    let keep = std::collections::HashSet::from([composer_json.clone()]);
    let removed = sweep_manifest_orphans(&previous, &keep, &[package_dir], &[]).expect("manifest sweep");

    assert_eq!(removed, 0);
    assert!(composer_json.exists());
    assert!(
        composer_lock.exists(),
        "lockfile must survive when its manifest is still written this run"
    );
}

/// Documents the chosen behavior for the genuinely contentious case: a user
/// hand-edits an unmarkable manifest (`composer.json`) at a path alef
/// previously wrote and has now decided to stop writing. The unmarkable-
/// manifest route does not inspect content (see `path_is_reclaimable`'s
/// doc), so the hand-edited file is reclaimed exactly like an untouched one
/// — deletion wins over preserving the edit, because alef's current-run
/// intent ("this path is not mine") is unambiguous and the file is already
/// orphaned relative to that intent.
#[test]
fn manifest_sweep_reclaims_user_modified_unmarkable_manifest_at_disowned_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/php");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let composer_json = package_dir.join("composer.json");
    std::fs::write(
        &composer_json,
        "{\n  \"name\": \"acme/demo\",\n  \"require\": {\n    \"hand-added/package\": \"^9.9\"\n  }\n}\n",
    )
    .expect("hand-edited composer.json");

    let previous = vec![composer_json.clone()];
    let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir], &[])
        .expect("manifest sweep");

    assert_eq!(
        removed, 1,
        "hand-edited content at a disowned path is reclaimed, not preserved"
    );
    assert!(!composer_json.exists());
}

/// The unmarkable-manifest allowlist is narrow by construction: a filename
/// outside [`UNMARKABLE_ALEF_MANIFESTS`] (e.g. `pubspec.yaml`, which opts
/// out of a marker for unrelated reasons) still requires the original
/// content-based marker check. This guards against the allowlist silently
/// growing to swallow every `generated_header: false` scaffold file.
#[test]
fn manifest_sweep_does_not_widen_provenance_route_beyond_the_allowlist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/dart");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let pubspec = package_dir.join("pubspec.yaml");
    std::fs::write(&pubspec, "name: demo\nversion: 1.0.0\n").expect("pubspec.yaml");

    let previous = vec![pubspec.clone()];
    let removed = sweep_manifest_orphans(&previous, &std::collections::HashSet::new(), &[package_dir], &[])
        .expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "a filename outside the allowlist must still require the marker check"
    );
    assert!(pubspec.exists());
}

#[test]
fn recursive_sweep_and_collection_ignore_hash_only_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let generated_root = dir.path().join("java");
    std::fs::create_dir_all(&generated_root).expect("generated root");
    let legacy_scaffold = generated_root.join("settings.gradle.kts");
    std::fs::write(
        &legacy_scaffold,
        format!("// alef:hash:{}\nrootProject.name = \"sample\"\n", "0".repeat(64)),
    )
    .expect("legacy scaffold");

    let removed = sweep_orphans(std::slice::from_ref(&generated_root), &std::collections::HashSet::new())
        .expect("recursive sweep");
    let collected = collect_alef_headered_paths(&generated_root);

    assert_eq!(removed, 0);
    assert!(legacy_scaffold.exists());
    assert!(!collected.contains(&legacy_scaffold));
}

/// The core case this whole route exists for: a file dropped from every manifest still on
/// disk (simulated here by a `previous_paths`/`keep` that never mention it at all) is
/// unreachable by the `previous_paths` route no matter how correct today's bookkeeping is --
/// only the marker on disk, under a root the caller has vouched for, can find it.
/// The disk-scan route REPORTS and never deletes. This test previously asserted the opposite
/// and is inverted deliberately: the same five-clause gate it encoded would have deleted a
/// consumer's Java public API class and a class a live test depended on, both of which passed
/// every clause. Absence from a run's output does not prove a file is an orphan. If this test
/// is ever "fixed" back to asserting deletion, read `report_disk_scan_candidates`' doc first. ~keep
#[test]
fn manifest_sweep_disk_scan_reports_but_never_deletes_a_marked_file_absent_from_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/kotlin_android");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let kept = package_dir.join("Bridge.kt");
    let orphan = package_dir.join("Language.kt");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&kept, &hashed).expect("kept file");
    std::fs::write(&orphan, &hashed).expect("orphan file");
    git_add(&package_dir, &["Bridge.kt"]);
    git_add(&package_dir, &["Language.kt"]);

    let previous = vec![kept.clone()];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(removed, 0, "the disk-scan route must never delete");
    assert!(
        orphan.exists(),
        "a marked, tracked file absent from keep must be REPORTED, not removed -- it may be a \
         create-once seed or a file the emitter failed to emit this run"
    );
    assert!(kept.exists(), "the file still recorded in keep must survive");
}

#[test]
fn manifest_sweep_disk_scan_preserves_unmarked_tracked_file_absent_from_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/kotlin_android");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let kept = package_dir.join("Bridge.kt");
    let handwritten = package_dir.join("Extensions.kt");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&kept, &hashed).expect("kept file");
    std::fs::write(&handwritten, "fun extra() {}\n").expect("handwritten file");
    git_add(&package_dir, &["Bridge.kt"]);
    git_add(&package_dir, &["Extensions.kt"]);

    let previous = vec![kept.clone()];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "a file with no alef marker must never be deleted by the disk-scan route"
    );
    assert!(handwritten.exists());
}

#[test]
fn manifest_sweep_disk_scan_preserves_tracked_marked_file_present_in_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/kotlin_android");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let kept = package_dir.join("Bridge.kt");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&kept, &hashed).expect("kept file");
    git_add(&package_dir, &["Bridge.kt"]);

    let previous = vec![kept.clone()];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(removed, 0);
    assert!(kept.exists());
}

/// The concrete failure mode this gate exists for: a backend whose manifest bookkeeping
/// records only its Rust crate source path, and never a single path under its own
/// language-output root (measured on a real tree, for several backends), is indistinguishable
/// from a healthy root with nothing to reclaim if the only signal consulted is "does this
/// path individually appear in keep". The gate must refuse to scan a root it cannot vouch for
/// at all, even though the caller explicitly names it as a `disk_scan_roots` candidate. ~keep
#[test]
fn manifest_sweep_disk_scan_skips_root_with_degenerate_manifest_baseline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/elixir");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let orphan = package_dir.join("dropped.ex");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&orphan, &hashed).expect("orphan file");
    git_add(&package_dir, &["dropped.ex"]);

    // The manifest baseline records only an unrelated Rust source path, never anything under
    // `package_dir` -- the exact signature measured for several real backends. ~keep
    let rust_source = dir.path().join("native/sample/src/lib.rs");
    let previous = vec![rust_source.clone()];
    let keep = std::collections::HashSet::from([rust_source]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "a root with zero manifest/keep entries recorded under it must be skipped, not scanned"
    );
    assert!(orphan.exists());
}

/// A build tool can stage a copy of a real generated file at a mirrored path under the same
/// owned root (a gem packaging stage directory is the concrete case this guards against); the
/// copy inherits the marker verbatim, so content alone cannot distinguish it from the
/// original. Only the original is git-tracked -- the staged copy is disposable build output.
#[test]
fn manifest_sweep_disk_scan_preserves_untracked_marked_file_absent_from_keep() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/ruby");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let kept = package_dir.join("client.rb");
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let hashed = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    std::fs::write(&kept, &hashed).expect("kept file");
    git_add(&package_dir, &["client.rb"]);

    let staged_dir = package_dir.join("tmp/ruby/stage/lib");
    std::fs::create_dir_all(&staged_dir).expect("staged dir");
    let staged_copy = staged_dir.join("client.rb");
    std::fs::write(&staged_copy, &hashed).expect("staged copy");

    let previous = vec![kept.clone()];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "an untracked staged copy that merely inherits the marker must never be deleted"
    );
    assert!(staged_copy.exists());
    assert!(kept.exists());
}

/// The collection route must refuse a git-ignored staging copy, and must keep returning the
/// tracked original and an untracked-but-not-ignored file.
///
/// All three assertions are load-bearing together. Refusing the staged copy alone is
/// satisfiable by a walker that returns nothing, and the untracked case is what forces the
/// gate to be *ignored-ness* rather than tracked-ness: this set feeds
/// `finalize_hashes_sweeping`, which must still re-stamp a file `alef generate` wrote moments
/// ago and the consumer has not committed yet. ~keep
#[test]
fn collect_alef_headered_paths_refuses_git_ignored_copies_and_keeps_uncommitted_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/ruby");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);

    let header = crate::core::hash::header(crate::core::hash::CommentStyle::Hash);
    let marked = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));

    write_file(&package_dir, ".gitignore", "tmp/\n");
    let tracked = write_file(&package_dir, "lib/client.rb", &marked);
    git_add(&package_dir, &[".gitignore", "lib/client.rb"]);
    let uncommitted = write_file(&package_dir, "lib/fresh.rb", &marked);
    let staged_copy = write_file(&package_dir, "tmp/ruby/stage/lib/client.rb", &marked);

    let collected = collect_alef_headered_paths(&package_dir);

    assert!(
        !collected.contains(&staged_copy),
        "a gitignored gem-staging copy carries the same marker verbatim and must not be \
         collected: {collected:?}"
    );
    assert!(
        collected.contains(&tracked),
        "the tracked original must still be collected, or this test passes by collecting \
         nothing: {collected:?}"
    );
    assert!(
        collected.contains(&uncommitted),
        "generated output the consumer has not committed yet must still be collected, so the \
         re-stamp pass can reach it: {collected:?}"
    );
}

/// alef-task #557 (follow-up): a crate's genuinely FIRST run -- no manifest this call consulted
/// (`previous_paths` overall, not merely under this one root) recorded anything anywhere -- must
/// NOT be reported as the "orphan-reclaim bookkeeping gap" warning. That warning's own text
/// asserts the condition is "never legitimate", but an absent previous-run manifest (a fresh
/// checkout, a wiped `.alef/` cache, or truly the first run ever) is exactly legitimate: there is
/// no previous baseline for anything to have gone wrong in. This reproduces the false positive a
/// consumer reported across 14 generated roots on their first run under this bookkeeping --
/// before the fix this asserted `logs_contain("orphan-reclaim bookkeeping gap")`, which is the
/// bug, not the spec. See `root_with_entries_under_other_root_and_none_under_this_one_still_warns`
/// below for the real-gap case this must keep catching. ~keep
#[traced_test]
#[test]
fn root_with_keep_entries_and_wholly_absent_previous_manifest_is_not_a_bookkeeping_gap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let kept = package_dir.join("Bridge.java");
    std::fs::write(&kept, "class Bridge {}\n").expect("kept file");

    let previous: Vec<PathBuf> = Vec::new();
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed =
        sweep_manifest_orphans(&previous, &keep, std::slice::from_ref(&package_dir), &[]).expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "disk-scan reclaim is opted out of for this root, so nothing is ever removed here"
    );
    assert!(
        !logs_contain("orphan-reclaim bookkeeping gap"),
        "a wholly absent previous-run manifest (first run, nothing recorded anywhere) must not be \
         reported as the bookkeeping-gap defect -- there is no previous baseline to have a gap in"
    );
    assert!(
        logs_contain("no previous-run manifest"),
        "the first-run case must still be visible at a lower severity, not silently dropped"
    );
    assert!(kept.exists());
}

/// The real defect (alef#158, alef-task #557's original regression): a previous-run manifest DID
/// exist and recorded entries under a DIFFERENT root, proving the bookkeeping mechanism itself
/// works, yet it recorded nothing under THIS root even though this run has kept output here. This
/// is the case the warning exists to catch, and it must keep firing at full severity even after
/// the first-run false positive above is fixed -- otherwise the fix would have made the check
/// vacuous instead of accurate. ~keep
#[traced_test]
#[test]
fn root_with_entries_under_other_root_and_none_under_this_one_still_warns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let kept = package_dir.join("Bridge.java");
    std::fs::write(&kept, "class Bridge {}\n").expect("kept file");

    let other_root_entry = dir.path().join("packages/python/client.py");
    let previous: Vec<PathBuf> = vec![other_root_entry];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed =
        sweep_manifest_orphans(&previous, &keep, std::slice::from_ref(&package_dir), &[]).expect("manifest sweep");

    assert_eq!(
        removed, 0,
        "disk-scan reclaim is opted out of for this root, so nothing is ever removed here"
    );
    assert!(
        logs_contain("orphan-reclaim bookkeeping gap"),
        "a previous manifest that recorded entries elsewhere but nothing under this root is the \
         real bookkeeping gap and must still warn at full severity"
    );
    assert!(kept.exists());
}

/// Negative control: a root with BOTH manifest and keep entries recorded stays quiet -- proves
/// the warning fires on the manifest/keep mismatch, not merely on a root passing through
/// `sweep_manifest_orphans` at all. ~keep
#[traced_test]
#[test]
fn root_with_both_manifest_and_keep_entries_stays_quiet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let package_dir = dir.path().join("packages/java");
    std::fs::create_dir_all(&package_dir).expect("package dir");
    let kept = package_dir.join("Bridge.java");
    std::fs::write(&kept, "class Bridge {}\n").expect("kept file");

    let previous = vec![kept.clone()];
    let keep = std::collections::HashSet::from([kept.clone()]);

    let removed =
        sweep_manifest_orphans(&previous, &keep, std::slice::from_ref(&package_dir), &[]).expect("manifest sweep");

    assert_eq!(removed, 0);
    assert!(
        !logs_contain("orphan-reclaim bookkeeping gap"),
        "a root with both manifest and keep entries recorded must not warn"
    );
    assert!(kept.exists());
}

/// The disk-scan loop's own skip message. Present in BOTH of its branches, so asserting it is
/// what proves the root reached classification at all -- a fixture whose root does not exist is
/// skipped silently by `sweep_manifest_orphans` and would otherwise make every "must not warn"
/// assertion below pass by examining nothing. ~keep
const DISK_SCAN_SKIPPED: &str = "disk-scan orphan reclaim skipped for";
/// Only the no-previous-baseline (first run / cleared cache / newly introduced ownership
/// manifest) branch.
const DISK_SCAN_NO_BASELINE: &str = "no previous-run manifest exists at all";
/// Only the real-defect branch: a baseline exists and still vouches for nothing under this root.
const DISK_SCAN_BACKEND_DEFECT: &str = "backend whose own bookkeeping has never vouched";

/// Fixture shared by the three disk-scan classification tests below: one real, existing,
/// git-tracked output root holding one alef-marked file. Only the manifest/keep state differs
/// between the three tests, so any difference in what they log is attributable to that state
/// alone rather than to one of them handing `sweep_manifest_orphans` a root it skips as
/// non-existent before classifying anything. ~keep
fn disk_scan_fixture(dir: &tempfile::TempDir, package_subdir: &str) -> (PathBuf, PathBuf) {
    let package_dir = dir.path().join(package_subdir);
    std::fs::create_dir_all(&package_dir).expect("package dir");
    git_init(&package_dir);
    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let marked = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    let generated = package_dir.join("Bridge.java");
    std::fs::write(&generated, &marked).expect("generated file");
    git_add(&package_dir, &["Bridge.java"]);
    assert!(
        package_dir.is_dir(),
        "fixture produced no existing root at {} -- sweep_manifest_orphans skips a non-existent \
         root before it classifies anything, so every assertion below would pass vacuously",
        package_dir.display()
    );
    (package_dir, generated)
}

/// Case (c), the one a consumer hit on a clean regen: `alef cache clear` (or a fresh checkout, or
/// the first run after `{stage}-ownership` manifests were introduced) leaves
/// `cache::read_stage_paths` returning an empty `Vec` for EVERY language, so `previous_paths` is
/// globally empty and every existing output root reports `0 manifest entry(s)` at once. That is
/// the absence of a baseline, not a backend that keeps no books -- and it self-resolves as soon as
/// this run's callers write their ownership manifests back, which they do immediately after
/// `sweep_manifest_orphans` returns.
///
/// `sweep_manifest_orphans`' sibling `allowed_roots` loop already made this exact distinction
/// (commit ba58517b1); the disk-scan loop did not, and reported the cleared cache as
/// "this backend's bookkeeping has never vouched for anything" across 20+ roots. ~keep
#[traced_test]
#[test]
fn disk_scan_root_with_no_baseline_anywhere_is_not_reported_as_a_backend_bookkeeping_defect() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (package_dir, generated) = disk_scan_fixture(&dir, "packages/java");

    let previous: Vec<PathBuf> = Vec::new();
    let keep = std::collections::HashSet::from([generated.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(removed, 0, "the disk-scan route never deletes");
    assert!(
        logs_contain(DISK_SCAN_SKIPPED),
        "the root must reach classification and report a skip -- without this the assertions \
         below would pass on a root that was never examined"
    );
    assert!(
        !logs_contain(DISK_SCAN_BACKEND_DEFECT),
        "a wholly absent previous-run baseline (cleared cache, fresh checkout, first run under \
         these ownership manifests) must not be reported as a backend that never records its own \
         output -- there is no baseline for that claim to be measured against"
    );
    assert!(
        logs_contain(DISK_SCAN_NO_BASELINE),
        "the no-baseline case must stay visible at a lower severity, and must say so, rather than \
         going silent -- the consumer asked whether a second run would settle it"
    );
    assert!(generated.exists());
}

/// Case (a), the real defect this warning exists for, and the control that proves the fix above
/// did not simply make the check quiet: a previous-run manifest DOES exist and recorded paths --
/// just none under this root. That is a backend whose bookkeeping has never vouched for its own
/// output tree, and it must keep warning at unchanged severity. ~keep
#[traced_test]
#[test]
fn disk_scan_root_unrecorded_while_baseline_covers_another_root_still_warns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (package_dir, generated) = disk_scan_fixture(&dir, "packages/java");

    let previous: Vec<PathBuf> = vec![dir.path().join("packages/python/client.py")];
    let keep = std::collections::HashSet::from([generated.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(removed, 0, "the disk-scan route never deletes");
    assert!(
        logs_contain(DISK_SCAN_SKIPPED),
        "the root must reach classification and report a skip"
    );
    assert!(
        logs_contain(DISK_SCAN_BACKEND_DEFECT),
        "a baseline that recorded paths elsewhere but nothing under this root is the genuine \
         bookkeeping gap and must still warn"
    );
    assert!(
        !logs_contain(DISK_SCAN_NO_BASELINE),
        "this root has a baseline; it must not be excused as a first run"
    );
    assert!(generated.exists());
}

/// Case (b)'s negative control, and the anti-vacuity anchor for the two tests above: a root with
/// BOTH manifest and keep entries is not skipped at all, it is scanned. Asserted through a
/// positive artefact of the scan -- the unrecorded-file report -- so "no skip message" here means
/// "the scan ran and reached a verdict", not "nothing happened". ~keep
#[traced_test]
#[test]
fn disk_scan_root_with_both_manifest_and_keep_entries_is_scanned_not_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (package_dir, generated) = disk_scan_fixture(&dir, "packages/java");

    let header = crate::core::hash::header(crate::core::hash::CommentStyle::DoubleSlash);
    let marked = crate::core::hash::inject_hash_line(&header, &"0".repeat(64));
    let unrecorded = package_dir.join("Legacy.java");
    std::fs::write(&unrecorded, &marked).expect("unrecorded file");
    git_add(&package_dir, &["Legacy.java"]);

    let previous: Vec<PathBuf> = vec![generated.clone()];
    let keep = std::collections::HashSet::from([generated.clone()]);

    let removed = sweep_manifest_orphans(
        &previous,
        &keep,
        std::slice::from_ref(&package_dir),
        std::slice::from_ref(&package_dir),
    )
    .expect("manifest sweep");

    assert_eq!(removed, 0, "the disk-scan route reports, it never deletes");
    assert!(
        logs_contain("are not in this run's recorded output"),
        "the scan must actually have run and reported the unrecorded file -- this is what makes \
         the two `!logs_contain` assertions below meaningful instead of vacuous"
    );
    assert!(
        !logs_contain(DISK_SCAN_SKIPPED),
        "a root vouched for by both the baseline and this run's keep set must be scanned, not skipped"
    );
    assert!(unrecorded.exists(), "reported candidates are never deleted");
    assert!(generated.exists());
}
