//! Coverage for [`super::write_materialized_files`].
//!
//! Regression for alef-task #557: `RustBridgeC.h` (and the `SwiftBridgeCore.swift`/
//! `{binding_crate}.swift` siblings) is written by `PostBuildStep::MaterializeSwiftBridge` via
//! this function, the one write path in `alef build` that reads content straight from
//! swift-bridge's own external build output rather than through the standard
//! `write_files`/`write_files_report` pipeline. That pipeline runs every other `GeneratedFile`
//! through `normalize_content` before it ever reaches disk; before this fix, this one write path
//! did not, so a trailing-whitespace line or a missing final newline in swift-bridge's own
//! codegen output (e.g. a consumer's real `#include <stdbool.h> ` with a trailing space) shipped
//! into the committed file uncorrected -- `git diff --check` then flags every such consumer's
//! generated tree as whitespace-dirty.

use super::write_materialized_files;
use crate::core::backend::GeneratedFile;

/// Split out of the parent test module: this is a self-contained concern (the write path's
/// normalization), distinct from `emit_swift_bridge_files`' own content-assembly coverage in
/// `bridge_artifacts.rs`'s inline `tests` module.
fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// THE REGRESSION: a line with trailing whitespace -- exactly the `#include <stdbool.h> ` shape
/// alef-task #557 reports -- must not survive onto disk. Generalized to the whole header rather
/// than pinned to that one line: every line the trio carries must come out clean.
#[test]
fn strips_trailing_whitespace_from_every_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Sources/RustBridgeC/RustBridgeC.h");
    let content =
        "#ifndef RUST_BRIDGE_C_H\n#define RUST_BRIDGE_C_H\n\n#include <stdbool.h> \n#include <stdint.h>\n\n#endif\n";

    write_materialized_files(
        vec![GeneratedFile {
            path: path.clone(),
            content: content.to_string(),
            generated_header: false,
        }],
        dir.path(),
    )
    .expect("write materialized files");

    let written = read(&path);
    assert!(
        written.lines().all(|line| line == line.trim_end()),
        "no emitted line may end in whitespace, got:\n{written:?}"
    );
    assert!(
        written.contains("#include <stdbool.h>\n"),
        "the include line's own text must survive, only the trailing space must go, got:\n{written:?}"
    );
}

/// Content missing a final trailing newline must gain exactly one, matching
/// `normalize_whitespace_with_policy`'s contract for every other generated file this command
/// writes.
#[test]
fn ensures_a_single_trailing_newline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Sources/RustBridge/Demo.swift");
    let content = "public struct Demo {}";

    write_materialized_files(
        vec![GeneratedFile {
            path: path.clone(),
            content: content.to_string(),
            generated_header: false,
        }],
        dir.path(),
    )
    .expect("write materialized files");

    let written = read(&path);
    assert_eq!(
        written, "public struct Demo {}\n",
        "content with no trailing newline must gain exactly one, got:\n{written:?}"
    );
}

/// A run of 3+ blank lines -- plausible in swift-bridge's own unformatted output -- must collapse
/// to AT MOST 2, matching `normalize_whitespace_with_policy`'s `max_blanks = 2` for every
/// non-Rust, non-markdown generated file (`is_markdown` is only true for `.md`; a `.h` is neither
/// Rust nor markdown, so it gets this cap, not the markdown-only single-blank-line one).
///
/// The real contract is "collapse to 2", not "collapse to 1" or "remove all blank lines" -- a run
/// of exactly 2 blank lines (three consecutive `\n`) in the output is the CORRECT canonical
/// result here, not a leftover bug. An earlier version of this test asserted
/// `!written.contains("\n\n\n")`, which rejects that correct 2-blank-line output and was wrong
/// about the contract, not about whether normalization ran (alef-task #557 follow-up). ~keep
#[test]
fn collapses_runs_of_blank_lines_to_the_non_markdown_cap_of_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Sources/RustBridgeC/RustBridgeC.h");
    let content = "#ifndef X\n#define X\n\n\n\n\n#endif\n";

    write_materialized_files(
        vec![GeneratedFile {
            path: path.clone(),
            content: content.to_string(),
            generated_header: false,
        }],
        dir.path(),
    )
    .expect("write materialized files");

    let written = read(&path);
    assert_eq!(
        written, "#ifndef X\n#define X\n\n\n#endif\n",
        "a run of 4 blank lines must collapse to exactly 2 (three consecutive newlines) -- fewer \
         would be the markdown-only single-blank-line policy misapplied to a `.h` file, and more \
         would mean the cap did not apply at all, got:\n{written:?}"
    );
    assert!(
        !written.contains("\n\n\n\n"),
        "no run of 3+ blank lines (four or more consecutive newlines) may survive, got:\n{written:?}"
    );
}

/// Parent directories that do not exist yet must be created -- matching the loop this function
/// replaced in `cli::pipeline::commands::build`'s `MaterializeSwiftBridge` handling.
#[test]
fn creates_missing_parent_directories() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("Sources").join("RustBridgeC").join("RustBridgeC.h");
    assert!(
        !path.parent().expect("parent").exists(),
        "precondition: parent must not exist yet"
    );

    write_materialized_files(
        vec![GeneratedFile {
            path: path.clone(),
            content: "#endif\n".to_string(),
            generated_header: false,
        }],
        dir.path(),
    )
    .expect("write materialized files");

    assert!(
        path.is_file(),
        "file must exist under the newly created parent directory"
    );
}
