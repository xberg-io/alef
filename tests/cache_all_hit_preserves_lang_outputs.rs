//! `alef all` must not delete a cache-hit language's own generated files.
//!
//! Reproduced by hand against this repo's own binary before this file existed: a warm `alef
//! all` on a completely unchanged fixture reported the language a cache HIT ("unchanged since
//! the last run by this alef build, skipping") and then, in the very same run, the binding-orphan
//! sweep deleted every file that language's own manifest still listed. The manifest text itself
//! survived the run untouched -- `cache::write_lang_manifest` faithfully wrote back the same five
//! paths it read -- but since the files those paths named no longer existed, the next run's
//! `outputs_exist` check failed and the language came back as a MISS. Delete on every hit,
//! regenerate on every miss: the cache alternated hit/miss forever, one full regeneration per run,
//! for a language nothing about the source ever touched.
//!
//! Root cause: `all_commands.rs` builds `current_gen_paths` -- the `keep` set handed to
//! `sweep_manifest_orphans` -- only from files `pipeline::generate` actually produced *this run*.
//! A cache-hit language contributes nothing to that call (it was filtered out of `to_generate`
//! before generation ran at all), so it contributed nothing to `current_gen_paths` either, and
//! every path recorded under that language's own root in last run's `all-bindings-<lang>-ownership`
//! stage manifest looked exactly like a path the generator had permanently stopped emitting.
//! `core_commands.rs`'s `alef generate` handler already re-seeds its own `keep` set
//! (`cleanup_keep_paths`) from `cache::read_lang_manifest` for every language it skips as cached
//! -- this is the same fix, applied to the sibling command that skipped it.
//!
//! Not a literal "the manifest comes back empty" as first suspected: the manifest file is never
//! empty here, and `outputs_exist` (`src/cli/cache_outputs.rs`) already refuses an empty one as a
//! miss regardless. The mechanism is a step further downstream -- the files a non-empty manifest
//! still names get removed out from under it -- but the externally observable result is the same
//! one first reported: a run that hits the cache should be a no-op, and instead it costs a full
//! regeneration next time, forever.
//!
//! Two full `alef all` runs are not enough to assert a hit here, and asserting one would be
//! testing an unrelated, pre-existing effect rather than this bug: on a cold fixture,
//! `all_commands.rs` stamps a language's `alef:hash:` marker *before* the whole-tree formatting
//! pass reformats that same content, so the very first warm run legitimately (and, as far as this
//! file is concerned, correctly) re-detects that mismatch as one real miss and self-heals it --
//! confirmed by hand across three independently created fixtures, always on run 2 and never
//! again after. That is a distinct, pre-existing ordering question this file does not adjudicate.
//! What this file asserts is the shape the ticket cares about: once warmed up, repeated runs must
//! both report a hit AND leave the language's files on disk -- which is exactly what the sweep
//! bug above breaks, run after run, with no warm-up exception. ~keep

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn alef_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_alef"))
}

const GO_OUTPUT_FILES: &[&str] = &[
    "packages/go/binding.go",
    "packages/go/embed_ffi.go",
    "packages/go/generate.go",
    "packages/go/native_setup.go",
];

fn write_fixture(root: &Path) {
    fs::create_dir_all(root.join("src")).expect("create fixture source directory");
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"cache-all-hit-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write fixture Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub struct Record { pub value: String }\n\npub fn record_value(record: Record) -> String { record.value }\n",
    )
    .expect("write fixture source");
    fs::write(
        root.join("alef.toml"),
        format!(
            "[workspace]\nalef_version = \"{}\"\nlanguages = [\"go\"]\n\n\
             [[crates]]\nname = \"cache-all-hit-fixture\"\nsources = [\"src/lib.rs\"]\n\
             version_from = \"Cargo.toml\"\n\n[crates.generate]\npublic_api = false\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("write alef config");
}

fn run_all(root: &Path) -> Output {
    Command::new(alef_binary())
        .env("RUST_LOG", "info")
        .current_dir(root)
        .args(["all"])
        .output()
        .expect("run alef all")
}

fn assert_go_files_present(root: &Path, context: &str) {
    for relative in GO_OUTPUT_FILES {
        assert!(
            root.join(relative).is_file(),
            "{relative} must still be on disk {context}; a cache-hit language's own files must \
             never be swept as orphans"
        );
    }
}

#[test]
fn a_cache_hit_alef_all_run_keeps_the_go_language_files_it_vouches_for() {
    let fixture = tempfile::tempdir().expect("create fixture directory");
    let root = fixture.path();
    write_fixture(root);

    let cold = run_all(root);
    assert!(
        cold.status.success(),
        "cold `alef all` must succeed: {}",
        String::from_utf8_lossy(&cold.stderr)
    );
    assert_go_files_present(root, "after the cold run");

    // Run 2 is a deliberate no-op on the assertions below: see the module doc for why it may
    // legitimately report either a hit or the one-time formatting-order miss. The only thing
    // asserted here is the one invariant that must hold regardless of which it is -- the files
    // the language owns are still on disk once the run has finished.
    let warm_up = run_all(root);
    assert!(
        warm_up.status.success(),
        "second `alef all` must succeed: {}",
        String::from_utf8_lossy(&warm_up.stderr)
    );
    assert_go_files_present(root, "after the second run");

    // Run 3 and run 4 are the real assertion: once warmed up, a cache hit must stay a cache hit,
    // and it must never cost the language's own generated files. Before the fix, run 3 already
    // reported a hit ("unchanged since the last run by this alef build, skipping") while its own
    // orphan sweep deleted every one of `GO_OUTPUT_FILES` in the same breath -- the assertion
    // below on run 3's stderr would have passed while `assert_go_files_present` right after it
    // failed, which is exactly the gap this test exists to close.
    let hit = run_all(root);
    assert!(
        hit.status.success(),
        "third `alef all` must succeed: {}",
        String::from_utf8_lossy(&hit.stderr)
    );
    let hit_stderr = String::from_utf8_lossy(&hit.stderr);
    assert!(
        hit_stderr.contains("unchanged since the last run by this alef build, skipping"),
        "the third run must be a genuine cache hit for `go` once warmed up, got:\n{hit_stderr}"
    );
    assert_go_files_present(root, "after the third run (a cache hit)");

    let hit_again = run_all(root);
    assert!(
        hit_again.status.success(),
        "fourth `alef all` must succeed: {}",
        String::from_utf8_lossy(&hit_again.stderr)
    );
    let hit_again_stderr = String::from_utf8_lossy(&hit_again.stderr);
    assert!(
        hit_again_stderr.contains("unchanged since the last run by this alef build, skipping"),
        "the fourth run must still be a cache hit -- a hit that deletes its own files makes the \
         next run a miss, so a real fix must not alternate, got:\n{hit_again_stderr}"
    );
    assert_go_files_present(root, "after the fourth run (a second consecutive cache hit)");
}
