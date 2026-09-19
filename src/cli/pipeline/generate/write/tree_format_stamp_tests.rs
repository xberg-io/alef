//! Regression coverage for the stamp-scope/format-scope agreement invariant.
//!
//! Both halves of every assertion here run the real production functions -- the write
//! path's stamping seam ([`super::finalize_hashes`] /
//! [`super::finalize_hashes_after_tree_format`]) and `alef verify`'s own staleness walk
//! ([`crate::bin_cli::helpers::verify_walk`]) -- never a hand-rolled restatement of either.
//! A reimplementation of the hash comparison would agree with itself while the two real
//! call paths disagreed, which is precisely the bug shape this file exists to catch. ~keep

use crate::bin_cli::helpers::verify_walk;
use crate::core::hash;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Stand-in for a crate's `sources_hash`; the value is opaque to both call paths, only its
/// being identical across the stamp and the verify side matters.
const SOURCES_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const ALEF_TOML: &[u8] = b"[workspace]\nlanguages = [\"python\"]\n";

#[test]
fn tree_stamping_does_not_adopt_hand_written_regeneration_instructions() {
    let tree = tree();
    let document = tree.root.join("GENERATED_E2E.md");
    let prose = "# Generated conformance suites\n\nRegenerate with `alef e2e generate`; edit the fixtures or generator, never the\ngenerated suites.\n";
    std::fs::write(&document, prose).unwrap();
    super::finalize_hashes_after_tree_format(&path_set(&[&tree.in_stamp_set]), &tree.root, SOURCES_HASH, ALEF_TOML)
        .unwrap();
    assert_eq!(std::fs::read_to_string(document).unwrap(), prose);
    assert!(stale_paths(&tree.root).is_empty());
}

/// Write an alef-marked Rust file, creating parents. Returns the absolute path.
///
/// `relative` is joined one component at a time: `Path::join("a/b")` keeps the literal `/`
/// inside a single Windows component, so the fixture path would stringify as
/// `...\crates/demo-py/src\lib.rs` while `verify_walk` reports the all-`\` form, and the
/// stale-path assertions compare `String`s. ~keep
fn write_marked(root: &Path, relative: &str, body: &str) -> PathBuf {
    let path = relative.split('/').fold(root.to_path_buf(), |acc, part| acc.join(part));
    std::fs::create_dir_all(path.parent().expect("relative path has a parent")).expect("create parent dirs");
    let content = format!("{}{body}", hash::header(hash::CommentStyle::DoubleSlash));
    std::fs::write(&path, content).expect("write marked file");
    path
}

/// The generated pyo3 binding crate source, in the unformatted shape `alef generate` writes
/// it (one long line rustfmt would wrap).
const UNFORMATTED_BODY: &str = "pub fn build(alpha: u32, beta: u32, gamma: u32, delta: u32, epsilon: u32) -> u32 { alpha + beta + gamma + delta + epsilon }\n";

/// The same file after a repo-wide `cargo fmt` / `poly fmt --fix` pass -- identical tokens,
/// different line width. This is the only thing the formatter changes, and it is enough to
/// change the content-inclusive `alef:hash:` value.
const FORMATTED_BODY: &str = "pub fn build(alpha: u32, beta: u32, gamma: u32, delta: u32, epsilon: u32) -> u32 {\n    alpha + beta + gamma + delta + epsilon\n}\n";

/// A tree shaped like a consumer repo: a package-directory file that a partial-regen stamp
/// set covers, and a generated binding-crate file outside it.
struct Tree {
    _dir: tempfile::TempDir,
    root: PathBuf,
    in_stamp_set: PathBuf,
    outside_stamp_set: PathBuf,
}

fn tree() -> Tree {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let in_stamp_set = write_marked(&root, "packages/python/glue.rs", FORMATTED_BODY);
    let outside_stamp_set = write_marked(&root, "crates/demo-py/src/lib.rs", UNFORMATTED_BODY);
    Tree {
        _dir: dir,
        root,
        in_stamp_set,
        outside_stamp_set,
    }
}

fn path_set(paths: &[&PathBuf]) -> HashSet<PathBuf> {
    paths.iter().map(|p| (*p).clone()).collect()
}

fn stale_paths(root: &Path) -> Vec<String> {
    verify_walk(root)
        .expect("verify walk")
        .into_iter()
        .map(|mismatch| mismatch.path)
        .collect()
}

/// The mechanism, stated as a test so the diagnosis cannot silently rot: a file stamped by
/// the real write path and then rewritten by a formatter that ran *outside* the stamp set is
/// reported stale by the real verify path, forever. `alef verify` is not re-deriving anything
/// here -- it hashes the bytes on disk, and those bytes genuinely no longer match the stamp.
#[test]
fn formatting_a_file_outside_the_stamp_set_makes_verify_report_it_stale() {
    let tree = tree();
    let all = path_set(&[&tree.in_stamp_set, &tree.outside_stamp_set]);

    super::finalize_hashes(&all, SOURCES_HASH, ALEF_TOML).expect("initial stamp");
    assert!(
        stale_paths(&tree.root).is_empty(),
        "the tree must start verify-clean, or this test proves nothing"
    );

    // A whole-tree formatter pass (`poly fmt --fix <repo>`, `cargo fmt --all`) rewraps the
    // binding crate source.
    std::fs::write(
        &tree.outside_stamp_set,
        hash::inject_hash_line(
            &format!("{}{FORMATTED_BODY}", hash::header(hash::CommentStyle::DoubleSlash)),
            &hash::extract_hash(&std::fs::read_to_string(&tree.outside_stamp_set).expect("read"))
                .expect("stamped file carries a hash line"),
        ),
    )
    .expect("simulate formatter rewrite");

    // The caller re-stamps only the paths it tracked, which do not include the file the
    // formatter touched.
    super::finalize_hashes(&path_set(&[&tree.in_stamp_set]), SOURCES_HASH, ALEF_TOML).expect("narrow stamp");

    assert_eq!(
        stale_paths(&tree.root),
        vec![tree.outside_stamp_set.display().to_string()],
        "a formatter pass wider than the stamp set leaves exactly the untracked file stale"
    );
}

/// The fix: a caller that formatted the whole tree stamps the whole tree, so no file the
/// formatter could have rewritten is left holding a stamp derived from its pre-format bytes.
#[test]
fn finalize_hashes_after_tree_format_keeps_verify_clean_for_files_outside_the_stamp_set() {
    let tree = tree();
    let all = path_set(&[&tree.in_stamp_set, &tree.outside_stamp_set]);

    super::finalize_hashes(&all, SOURCES_HASH, ALEF_TOML).expect("initial stamp");

    std::fs::write(
        &tree.outside_stamp_set,
        hash::inject_hash_line(
            &format!("{}{FORMATTED_BODY}", hash::header(hash::CommentStyle::DoubleSlash)),
            &hash::extract_hash(&std::fs::read_to_string(&tree.outside_stamp_set).expect("read"))
                .expect("stamped file carries a hash line"),
        ),
    )
    .expect("simulate formatter rewrite");

    super::finalize_hashes_after_tree_format(&path_set(&[&tree.in_stamp_set]), &tree.root, SOURCES_HASH, ALEF_TOML)
        .expect("tree-scoped stamp");

    assert!(
        stale_paths(&tree.root).is_empty(),
        "after a whole-tree format pass every alef-marked file under the formatted root must \
         carry a stamp derived from its post-format bytes; still stale: {:?}",
        stale_paths(&tree.root)
    );
}
