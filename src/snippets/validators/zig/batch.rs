//! The batched syntax path for Zig snippets.
//!
//! It lives beside `mod.rs`-equivalent `zig.rs` rather than inside it because that file is at the
//! module size limit, and batching is a concern of its own: one toolchain invocation covering many
//! snippets, and the attribution of its output back to the snippet that caused each line. ~keep

use super::apply_cache_dirs;
use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Snippet, SnippetStatus};
use crate::snippets::validators::BatchValidation;
use crate::snippets::validators::run_command;

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the zig toolchain failed without a snippet-specific diagnostic";

/// The substring that marks a diagnostic zig rejects a file over. `zig fmt` also names every file
/// it reformatted, one per line on stdout, and emits `note:` lines alongside errors — attributing
/// such a line to a snippet must not fail it, only an error may. ~keep
const ERROR_DIAGNOSTIC_MARKER: &str = ": error: ";

/// One toolchain start for the whole batch instead of one per snippet. `zig ast-check` takes
/// exactly one file — a second path is rejected outright as an extra positional parameter — so
/// the batch goes through `zig fmt --ast-check`, which runs the same check over every file it
/// is handed and reports each file's errors against that file's own path. Each snippet stays
/// its own root file, so the `main` every snippet declares never collides. ~keep
pub(super) fn validate_batch_with_context(
    snippets: &[&Snippet],
    timeout_secs: u64,
    session: Option<&ValidationSession>,
) -> Result<BatchValidation> {
    let dir = match session {
        Some(session) => session.scratch_dir()?,
        None => ScratchDir::isolated()?,
    };
    let mut file_names = Vec::with_capacity(snippets.len());
    let mut paths = Vec::with_capacity(snippets.len());
    for (index, snippet) in snippets.iter().enumerate() {
        let file_name = format!("{BATCH_FILE_PREFIX}{index}.zig");
        let path = dir.path().join(&file_name);
        std::fs::write(&path, snippet.code.trim())?;
        file_names.push(file_name);
        paths.push(path);
    }
    let mut command = std::process::Command::new("zig");
    command.args(["fmt", "--ast-check"]).args(&paths);
    apply_cache_dirs(&mut command, dir.path(), session);
    if let Some(session) = session {
        session.apply(&mut command);
    }
    let (success, output) = run_command(&mut command, timeout_secs)?;
    Ok(batch_results(&file_names, success, &output))
}

/// Attributes toolchain output back to the snippet that owns it. Every diagnostic opens with
/// its own source path (`snippet_batch_2.zig:4:9: error: …`), and the source and caret lines
/// that follow carry no path at all, so a pathless line stays with the file last named. ~keep
fn batch_results(file_names: &[String], success: bool, output: &str) -> BatchValidation {
    let mut diagnostics = vec![Vec::new(); file_names.len()];
    let mut rejected = vec![false; file_names.len()];
    let mut unmatched = Vec::new();
    let mut current = None;
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match file_owner(file_names, line).or(current) {
            Some(index) => {
                current = Some(index);
                diagnostics[index].push(line.to_string());
                rejected[index] |= line.contains(ERROR_DIAGNOSTIC_MARKER);
            }
            None => unmatched.push(line.to_string()),
        }
    }
    let attributed = rejected.iter().any(|value| *value);
    let fallback = (!success && !attributed).then(|| {
        if unmatched.is_empty() {
            BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
        } else {
            unmatched.join("\n")
        }
    });
    rejected
        .into_iter()
        .zip(diagnostics)
        .map(|(rejected, messages)| match (rejected, &fallback) {
            (true, _) => (SnippetStatus::Fail, Some(messages.join("\n"))),
            (false, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
            (false, None) => (SnippetStatus::Pass, None),
        })
        .collect()
}

fn file_owner(file_names: &[String], line: &str) -> Option<usize> {
    file_names
        .iter()
        .position(|file_name| line.contains(file_name.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{Language, SnippetMetadata, SourceOrigin, ValidationLevel};
    use crate::snippets::validators::SnippetValidator;
    use crate::snippets::validators::zig::ZigValidator;
    use std::path::PathBuf;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    fn zig_snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.zig"),
            language: Language::Zig,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.zig"),
                line: 1,
                block_index: 0,
            },
        }
    }

    #[test]
    fn batch_declines_the_levels_that_build_a_root_file() {
        let only = zig_snippet("pub fn main() void {}\n");

        for level in [
            ValidationLevel::Compile,
            ValidationLevel::TypeCheck,
            ValidationLevel::Run,
        ] {
            let declined = ZigValidator.validate_batch_in_session(&[&only], level, 10, None);
            assert!(
                declined.is_none(),
                "{level:?} must fall back to one process per snippet"
            );
        }
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !super::super::zig_is_runnable() {
            return;
        }
        let first = zig_snippet("pub fn first() u8 {\n    return 1;\n}\n");
        let second = zig_snippet("pub fn second() u8 {\n    return 2;\n}\n");
        let third = zig_snippet("pub fn third() u8 {\n    return 3;\n}\n");

        let results = validate_batch_with_context(&[&first, &second, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
            .expect("batch validation runs");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None),
                (SnippetStatus::Pass, None)
            ]
        );
    }

    #[test]
    fn batch_fails_only_the_broken_snippet_and_passes_its_neighbours() {
        if !super::super::zig_is_runnable() {
            return;
        }
        let first = zig_snippet("pub fn first() u8 {\n    return 1;\n}\n");
        let broken = zig_snippet("pub fn second() void { this is not zig ,, }\n");
        let third = zig_snippet("pub fn third() u8 {\n    return 3;\n}\n");

        let results = validate_batch_with_context(&[&first, &broken, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
            .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("error:")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// Each snippet stays its own root file, which is what lets one invocation cover them all:
    /// both declare `main`, a collision the moment anything compiled them as one root. ~keep
    #[test]
    fn batch_passes_two_snippets_that_each_declare_main() {
        if !super::super::zig_is_runnable() {
            return;
        }
        let first = zig_snippet("pub fn main() void {}\n");
        let second = zig_snippet("pub fn main() void {}\n");

        let results = validate_batch_with_context(&[&first, &second], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
            .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// `zig fmt` names every file it reformatted on stdout. Attributing such a line to its snippet
    /// must not fail that snippet — only an error diagnostic may. ~keep
    #[test]
    fn batch_results_do_not_fail_a_snippet_zig_merely_reformatted() {
        let file_names = vec!["snippet_batch_0.zig".to_string(), "snippet_batch_1.zig".to_string()];
        let output = "/tmp/scratch/snippet_batch_0.zig\n/tmp/scratch/snippet_batch_1.zig\n";

        let results = batch_results(&file_names, true, output);

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A toolchain that fails without naming any snippet must not let the batch pass: every
    /// snippet carries the real output instead. ~keep
    #[test]
    fn batch_results_fail_every_snippet_when_no_diagnostic_names_one() {
        let file_names = vec!["snippet_batch_0.zig".to_string(), "snippet_batch_1.zig".to_string()];

        let results = batch_results(&file_names, false, "error: unable to resolve zig cache directory\n");

        assert_eq!(
            results,
            vec![
                (
                    SnippetStatus::Fail,
                    Some("error: unable to resolve zig cache directory".to_string())
                ),
                (
                    SnippetStatus::Fail,
                    Some("error: unable to resolve zig cache directory".to_string())
                ),
            ]
        );
    }
}
