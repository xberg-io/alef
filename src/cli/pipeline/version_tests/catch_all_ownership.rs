use super::*;
use crate::core::config::{ResolvedCrateConfig, SyncConfig, TextReplacement};
use tracing_test::traced_test;

/// A crate config whose ONLY version-sync work is the two catch-all paths, so a
/// rewrite observed here can only have come from them and not from a named-filename
/// branch. ~keep
fn catch_all_config(root: &std::path::Path, sync: SyncConfig) -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample_core".to_string(),
        version_from: root.join("Cargo.toml").to_string_lossy().into_owned(),
        sources: vec![root.join("src/lib.rs")],
        workspace_root: Some(root.to_path_buf()),
        sync: Some(sync),
        ..ResolvedCrateConfig::default()
    }
}

fn seed_crate(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).expect("source directory");
    std::fs::write(root.join("src/lib.rs"), "pub fn transform() {}\n").expect("Rust source");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample_core\"\nversion = \"1.2.3\"\nedition = \"2024\"\n",
    )
    .expect("Cargo.toml");
}

fn run_sync(root: &std::path::Path, config: &ResolvedCrateConfig) {
    let config_path = root.join("alef.toml");
    std::fs::write(&config_path, "").expect("alef.toml");
    let original_cwd = std::env::current_dir().expect("current directory");
    std::env::set_current_dir(root).expect("enter temporary directory");
    let result = sync_versions(config, &config_path, None, true, true, None);
    std::env::set_current_dir(&original_cwd).expect("restore current directory");
    result.expect("sync versions");
}

/// THE CANARY. `extra_paths` accepts a glob, so a slightly wide pattern reaches files
/// nobody meant to hand to it, and the catch-all arm then runs a blanket
/// `SEMVER_RE.replace_all` over whatever it matched. Before the guard this rewrote a
/// hand-written file silently; `notes.toml` here is stampable (`.toml` has a comment
/// style) and carries no marker, which is the only combination where absence is real
/// evidence of foreignness.
///
/// This fails against the pre-guard generator, which rewrote 9.9.9 to 1.2.3. ~keep
#[test]
fn catch_all_semver_rewrite_leaves_a_stampable_file_that_carries_no_marker() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    let hand_written = "# maintained by hand\npinned = \"9.9.9\"\n";
    std::fs::write(root.join("notes.toml"), hand_written).expect("hand-written file");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: vec!["notes.toml".to_string()],
                text_replacements: Vec::new(),
            },
        ),
    );

    assert_eq!(
        std::fs::read_to_string(root.join("notes.toml")).expect("read notes.toml"),
        hand_written,
        "a stampable file with no alef marker reads as hand-written and must survive the catch-all"
    );
}

/// The other half of the same predicate: once alef owns the file, the catch-all is
/// free to keep its version coordinates current. Without this, the test above would
/// also pass if the guard simply refused everything. ~keep
#[test]
fn catch_all_semver_rewrite_still_updates_a_stampable_file_alef_owns() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    let owned = format!(
        "{}\npinned = \"9.9.9\"\n",
        crate::core::hash::header(crate::core::hash::CommentStyle::Hash)
    );
    std::fs::write(root.join("owned.toml"), &owned).expect("alef-owned file");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: vec!["owned.toml".to_string()],
                text_replacements: Vec::new(),
            },
        ),
    );

    let after = std::fs::read_to_string(root.join("owned.toml")).expect("read owned.toml");
    assert!(
        after.contains("1.2.3") && !after.contains("9.9.9"),
        "an alef-owned file must still be rewritten by the catch-all, got:\n{after}"
    );
}

/// The exemption `marker_comment_style`'s own `~keep` demands. `.md` has no comment
/// style alef stamps in, so a generated README never carries a marker even when alef
/// wrote every byte — refusing there would freeze legitimate regeneration, which is
/// exactly the failure mode that made this a policy question rather than a bug fix.
/// Guarding on marker-absence alone would break this. ~keep
#[test]
fn catch_all_rewrite_still_touches_an_unstampable_file_because_absence_proves_nothing() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    std::fs::write(root.join("NOTES.md"), "docs for 9.9.9\n").expect("markdown file");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: vec!["NOTES.md".to_string()],
                text_replacements: Vec::new(),
            },
        ),
    );

    assert!(
        std::fs::read_to_string(root.join("NOTES.md"))
            .expect("read NOTES.md")
            .contains("1.2.3"),
        "an extension alef cannot stamp must stay eligible — marker absence is not evidence there"
    );
}

/// `text_replacements` is the second catch-all and runs a consumer-supplied regex, so
/// it can rewrite anything its glob reaches. Same predicate, separate call site — a fix
/// applied to only one of the two would leave this one open. ~keep
#[test]
fn text_replacement_leaves_a_stampable_file_that_carries_no_marker() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    let hand_written = "version: 9.9.9\n";
    std::fs::write(root.join("hand.yaml"), hand_written).expect("hand-written yaml");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: Vec::new(),
                text_replacements: vec![TextReplacement {
                    path: "hand.yaml".to_string(),
                    search: r"version: [0-9.]+".to_string(),
                    replace: "version: {version}".to_string(),
                }],
            },
        ),
    );

    assert_eq!(
        std::fs::read_to_string(root.join("hand.yaml")).expect("read hand.yaml"),
        hand_written,
        "text_replacements must honour the same ownership predicate as the semver catch-all"
    );
}

/// alef#478: an ownership-guard refusal on a *declared* `sync.text_replacements` target must
/// not be silent. Before the fix, `hand.yaml` above was refused with no signal connecting the
/// refusal to the fact that a named version-sync contract had just gone unfulfilled -- this test
/// pins the warning that now fires, naming the file, the expected version, and the reason. ~keep
#[traced_test]
#[test]
fn text_replacement_refusal_warns_that_the_declared_sync_target_went_unwritten() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    std::fs::write(root.join("hand.yaml"), "version: 9.9.9\n").expect("hand-written yaml");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: Vec::new(),
                text_replacements: vec![TextReplacement {
                    path: "hand.yaml".to_string(),
                    search: r"version: [0-9.]+".to_string(),
                    replace: "version: {version}".to_string(),
                }],
            },
        ),
    );

    assert!(
        logs_contain("declared sync.text_replacements target was not updated to 1.2.3"),
        "expected a sync-target-not-updated warning naming the expected version"
    );
    assert!(
        logs_contain("ownership guard refused the write"),
        "expected the warning to name the refusal reason"
    );
    assert!(
        logs_contain("hand.yaml"),
        "expected the warning to name the refused file"
    );
}

/// The negative control for the warning above: a `sync.text_replacements` target alef is free
/// to write (it carries alef's own marker) produces no such warning -- the guard's refusal, not
/// merely being a declared sync target, is what triggers it. ~keep
#[traced_test]
#[test]
fn text_replacement_success_does_not_warn_that_the_sync_target_went_unwritten() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    let owned = format!(
        "{}\nversion: 9.9.9\n",
        crate::core::hash::header(crate::core::hash::CommentStyle::Hash)
    );
    std::fs::write(root.join("owned.yaml"), &owned).expect("alef-owned yaml");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: Vec::new(),
                text_replacements: vec![TextReplacement {
                    path: "owned.yaml".to_string(),
                    search: r"version: [0-9.]+".to_string(),
                    replace: "version: {version}".to_string(),
                }],
            },
        ),
    );

    let after = std::fs::read_to_string(root.join("owned.yaml")).expect("read owned.yaml");
    assert!(
        after.contains("1.2.3") && !after.contains("9.9.9"),
        "an alef-owned text_replacements target must still be rewritten, got:\n{after}"
    );

    assert!(
        !logs_contain("was not updated"),
        "a successfully-written sync target must not warn that it went unwritten"
    );
    assert!(
        !logs_contain("matched nothing"),
        "a pattern that did match (and got rewritten) must not also warn that it matched nothing"
    );
}

/// alef #A1 regression: the ownership guard must never fire when the substitution would not have
/// changed anything. Before the fix, `catch_all_rewrite_is_permitted` ran unconditionally ahead
/// of the substitution, so a `sync.text_replacements` target that was ALREADY at the version this
/// run would produce -- but carried no alef marker, since it is genuinely hand-written -- was
/// reported "not updated to a stale version" on every single sync, forever. tslp hit this 19
/// times across 5 paths in one chain run, and every one of the 19 was false: every file already
/// held `1.16.1`.
#[traced_test]
#[test]
fn text_replacement_already_current_does_not_warn_even_without_an_alef_marker() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    // Hand-written (no alef marker) but ALREADY at the version this sync run would produce --
    // the substitution has nothing left to do here.
    std::fs::write(root.join("hand.yaml"), "version: 1.2.3\n").expect("hand-written yaml");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: Vec::new(),
                text_replacements: vec![TextReplacement {
                    path: "hand.yaml".to_string(),
                    search: r"version: [0-9.]+".to_string(),
                    replace: "version: {version}".to_string(),
                }],
            },
        ),
    );

    assert!(
        !logs_contain("was not updated"),
        "a file already at the target version must never be reported as refused/stale, even when \
         it carries no alef marker -- the ownership guard must only run once a write is actually \
         about to happen"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("hand.yaml")).expect("read hand.yaml"),
        "version: 1.2.3\n",
        "an already-current hand-written file must be left untouched"
    );
}

/// alef #A2 regression: a declared `sync.text_replacements` search pattern that matches nothing
/// in a file alef successfully opened must warn -- silence here is indistinguishable from
/// "already current" and hid a real drift for three consecutive releases in a real downstream
/// incident this closes: a `search` shaped for a downstream repo's `.git` URL with `from:` against a file
/// that had moved to a non-`.git` URL with `branch:`.
#[traced_test]
#[test]
fn text_replacement_pattern_matching_nothing_warns() {
    let _guard = CWD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let temporary = tempfile::tempdir().expect("temporary directory");
    let root = temporary.path();
    seed_crate(root);

    let owned = format!(
        "{}\nbranch: \"release/1.0.0\"\n",
        crate::core::hash::header(crate::core::hash::CommentStyle::Hash)
    );
    std::fs::write(root.join("owned.yaml"), &owned).expect("alef-owned yaml");

    run_sync(
        root,
        &catch_all_config(
            root,
            SyncConfig {
                extra_paths: Vec::new(),
                text_replacements: vec![TextReplacement {
                    path: "owned.yaml".to_string(),
                    search: r"version: [0-9.]+".to_string(),
                    replace: "version: {version}".to_string(),
                }],
            },
        ),
    );

    assert!(
        logs_contain("search pattern") && logs_contain("matched nothing"),
        "a pattern matching nothing in a file alef successfully opened must warn, not silently \
         look identical to an already-current file"
    );
    assert!(
        !logs_contain("was not updated"),
        "a pattern-matched-nothing outcome is a distinct signal from the ownership-guard refusal \
         and must not be reported as one"
    );
}
