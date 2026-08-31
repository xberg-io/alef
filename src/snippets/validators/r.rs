use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct RValidator;

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_CHECKER_NAME: &str = "alef_batch_check.R";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the R batch checker failed without a per-snippet diagnostic";
const BATCH_LINE_PREFIX: &str = "ALEF|";
const BATCH_LINE_FIELD_COUNT: usize = 3;
const BATCH_LINE_SEPARATOR: char = '|';
const BATCH_OK: &str = "ok";

/// The checker prints one line per file and R's parse errors span several lines, so the message
/// travels with its newlines escaped and is restored on the Rust side. ~keep
const ESCAPED_NEWLINE: &str = "\\n";

/// One R start for the whole batch instead of one per snippet — starting the interpreter, not
/// parsing, is what a per-snippet `Rscript` run spends its time on. The checker runs the same
/// `parse(file = ...)` the serial path uses over every file it is handed and prints a line for
/// each, so one snippet's parse error neither aborts the run nor leaks into another snippet's
/// result; it always exits 0, which is why a missing line, not the exit status, is what marks a
/// snippet unjudged. ~keep
const BATCH_CHECKER_SOURCE: &str = r#"for (path in commandArgs(trailingOnly = TRUE)) {
  message <- tryCatch({ parse(file = path); "" }, error = function(error) conditionMessage(error))
  status <- if (nzchar(message)) "err" else "ok"
  escaped <- paste(strsplit(message, "\n", fixed = TRUE)[[1]], collapse = "\\n")
  cat("ALEF|", path, "|", status, "|", escaped, "\n", sep = "")
}
"#;

impl RValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let dir = match session {
            Some(session) => ScratchDir::for_session(session)?,
            None => ScratchDir::isolated()?,
        };
        let mut file_names = Vec::with_capacity(snippets.len());
        let mut paths = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.R");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, &snippet.code)?;
            file_names.push(file_name);
            paths.push(path);
        }
        let checker_path = dir.path().join(BATCH_CHECKER_NAME);
        std::fs::write(&checker_path, BATCH_CHECKER_SOURCE)?;
        let mut command = std::process::Command::new("Rscript");
        command.arg(&checker_path).args(&paths);
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("R_LIBS_USER", &session.working_directory);
        }
        let (_, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::checker_results(&file_names, &output))
    }

    /// Maps the checker's lines back to the snippet that owns each file. A snippet the checker
    /// never reported on fails carrying the real output rather than passing by default. ~keep
    fn checker_results(file_names: &[String], output: &str) -> BatchValidation {
        let mut reported = vec![None; file_names.len()];
        let mut unmatched = Vec::new();
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match Self::parse_line(file_names, line) {
                Some((index, value)) => reported[index] = Some(value),
                None => unmatched.push(line.to_string()),
            }
        }
        let resolved = reported.iter().all(Option::is_some);
        let fallback = (!resolved).then(|| {
            if unmatched.is_empty() {
                BATCH_FAILED_WITHOUT_DIAGNOSTIC.to_string()
            } else {
                unmatched.join("\n")
            }
        });
        reported
            .into_iter()
            .map(|value| match (value, &fallback) {
                (Some(value), _) => value,
                (None, Some(message)) => (SnippetStatus::Fail, Some(message.clone())),
                (None, None) => (SnippetStatus::Pass, None),
            })
            .collect()
    }

    fn parse_line(file_names: &[String], line: &str) -> Option<(usize, (SnippetStatus, Option<String>))> {
        let fields: Vec<&str> = line
            .strip_prefix(BATCH_LINE_PREFIX)?
            .splitn(BATCH_LINE_FIELD_COUNT, BATCH_LINE_SEPARATOR)
            .collect();
        let [path, status, message] = fields[..] else {
            return None;
        };
        let index = Self::owner(file_names, path)?;
        let value = if status == BATCH_OK {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(message.replace(ESCAPED_NEWLINE, "\n")))
        };
        Some((index, value))
    }

    fn owner(file_names: &[String], path: &str) -> Option<usize> {
        let name = std::path::Path::new(path).file_name()?;
        file_names
            .iter()
            .position(|file_name| std::ffi::OsStr::new(file_name.as_str()) == name)
    }

    fn validate_with_context(
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let scratch_dir = session.map(ValidationSession::scratch_dir).transpose()?;
        let mut source = match &scratch_dir {
            Some(dir) => tempfile::Builder::new().suffix(".R").tempfile_in(dir.path())?,
            None => NamedTempFile::with_suffix(".R")?,
        };
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let mut command = std::process::Command::new("Rscript");
        if level == ValidationLevel::Run {
            command.arg(source.path());
        } else {
            command.args(["-e", &format!("parse(file = {:?})", source.path().to_string_lossy())]);
        }
        if let Some(value) = session {
            value.apply(&mut command);
            command.env("R_LIBS_USER", &value.working_directory);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }
}

impl SnippetValidator for RValidator {
    fn language(&self) -> Language {
        Language::R
    }

    fn is_available(&self) -> bool {
        which::which("Rscript").is_ok() || which::which("R").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, None)
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        Self::validate_with_context(snippet, level, timeout_secs, session)
    }

    /// `Run` is declined: each snippet must execute in its own process so its output, exit status
    /// and side effects belong to it alone. Every other level runs the same `parse(file = ...)`
    /// call, which is what the batch checker applies per file. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        (level != ValidationLevel::Run).then(|| Self::validate_batch_with_context(snippets, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("could not find function")
            || output.contains("there is no package called")
            || output.contains("cannot open file")
    }

    // `parse(file = ...)` (the only check `validate_with_context` ever runs below `Run`, see
    // above) is R's pure syntax parser: it never resolves a function or package, and it is sent
    // identically for both `Syntax` and `Compile` — there is no separate compile step at all, so
    // a `Compile` request silently got the same result as `Syntax` while being reported as if it
    // had validated further. Real static checkers (`codetools::checkUsage`, lintr) do exist, but
    // aren't wired up here, so neither `compile` nor `typecheck` may be claimed until they are.
    // ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `parse(file = ...)` resolve a function. Structural, so it's
    // exempted from `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::runner::{RunnerConfig, run_validation};
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use crate::snippets::validators::ValidatorRegistry;

    /// Whether `Rscript` runs, not merely resolves: a version-manager shim (e.g. rig, asdf)
    /// spawns fine then exits non-zero, so `RValidator::is_available`'s PATH-only check would
    /// leave the skip below unreachable and fire the assert everywhere R is absent. ~keep
    fn rscript_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("Rscript")
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn undefined_function_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::R,
            title: None,
            code: "thisFunctionDoesNotExist12345()\n".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    fn r_snippet(code: &str) -> Snippet {
        let mut value = undefined_function_snippet();
        value.code = code.into();
        value
    }

    #[test]
    fn batch_declines_run_so_each_snippet_executes_on_its_own() {
        let only = r_snippet("value <- 1\n");

        let declined = RValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !rscript_is_runnable() {
            return;
        }
        let first = r_snippet("value <- 1\n");
        let second = r_snippet("value <- 2\n");
        let third = r_snippet("value <- 3\n");

        let results =
            RValidator::validate_batch_with_context(&[&first, &second, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
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
        if !rscript_is_runnable() {
            return;
        }
        let first = r_snippet("value <- 1\n");
        let broken = r_snippet("value <- (\n");
        let third = r_snippet("value <- 3\n");

        let results =
            RValidator::validate_batch_with_context(&[&first, &broken, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("unexpected end of input")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// The checker parses each file and evaluates none of it, so two snippets binding the same
    /// name in the interpreter's global environment — which one process sharing a scope would let
    /// the second silently overwrite — are judged independently. ~keep
    #[test]
    fn batch_passes_two_snippets_binding_the_same_global_name() {
        if !rscript_is_runnable() {
            return;
        }
        let first = r_snippet("shared <- function(x) x + 1\n");
        let second = r_snippet("shared <- function(x) x + 2\n");

        let results = RValidator::validate_batch_with_context(&[&first, &second], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
            .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A checker that never reported on a snippet must not let it pass by default: the snippet
    /// fails carrying whatever the toolchain actually said. ~keep
    #[test]
    fn checker_results_fail_every_unreported_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.R".to_string(), "snippet_batch_1.R".to_string()];

        let results = RValidator::checker_results(&file_names, "Fatal error: cannot open file\n");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Fail, Some("Fatal error: cannot open file".to_string())),
                (SnippetStatus::Fail, Some("Fatal error: cannot open file".to_string())),
            ]
        );
    }

    #[test]
    fn checker_results_restore_the_newlines_a_reported_message_carried() {
        let file_names = vec!["snippet_batch_0.R".to_string()];

        let results = RValidator::checker_results(&file_names, "ALEF|/tmp/s/snippet_batch_0.R|err|first\\nsecond\n");

        assert_eq!(results, vec![(SnippetStatus::Fail, Some("first\nsecond".to_string()))]);
    }

    #[test]
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(RValidator.achievable_level(ValidationLevel::Run), ValidationLevel::Run);
    }

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(RValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(RValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!RValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// A snippet that is syntactically valid but references a function that cannot resolve must
    /// not come back as a `typecheck` pass. `parse(file = ...)` accepts this file (it never
    /// resolves functions), so `achievable_level` caps it to `syntax`; because that gap is
    /// structural, it is exempted from `Downgraded` the same way a `max_level` ceiling is — a
    /// capability-capped `Pass`, not a claim of `typecheck`. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_function_does_not_pass_as_typecheck() {
        if !rscript_is_runnable() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary =
            run_validation(&[undefined_function_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::TypeCheck),
            "undefined-function snippet must not pass claiming typecheck: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(result.capability_capped);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// The regression this fix closes: before `achievable_level` also capped `Compile`, this
    /// undefined-function snippet passed a `Compile` request as an ordinary, unqualified `Pass` —
    /// `parse(file = ...)` accepts it, and nothing distinguished `Compile` from `Syntax`, so the
    /// result carried no `capability_capped` flag and no `downgrade_reason` at all. That is
    /// precisely the silent downgrade this validator must never produce again. ~keep
    #[test]
    fn compile_request_for_an_undefined_function_does_not_pass_as_compile() {
        if !rscript_is_runnable() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary =
            run_validation(&[undefined_function_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::Compile),
            "undefined-function snippet must not pass claiming compile: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(
            result.capability_capped,
            "a Compile request that only ran a syntax check must be flagged, not folded into an \
             ordinary Pass: {result:?}"
        );
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// Regression: `validate_with_context` used to write its session-scoped scratch file
    /// directly into `session.working_directory` via a bare `tempfile_in`, leaving an untracked
    /// `.tmp<random>.R` file loose in a tracked package source directory — with no `.gitignore`
    /// coverage at all. It must nest under the session's own `.alef/snippets/tmp` cache root
    /// instead, and stay gone whether the snippet passes or fails. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_and_is_removed_on_pass_and_fail() {
        if !rscript_is_runnable() {
            return;
        }
        let directory = tempfile::tempdir().expect("temp directory");
        let session = ValidationSession {
            language: Language::R,
            working_directory: directory.path().to_path_buf(),
            manifest: None,
            fingerprint: "scratch-shape-fixture".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        };
        let passing = Snippet {
            id: None,
            path: "passing.R".into(),
            language: Language::R,
            title: None,
            code: "value <- 1\n".into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "passing.R".into(),
                line: 1,
                block_index: 0,
            },
        };
        let mut failing = passing.clone();
        failing.code = "value <- (\n".into();

        let (pass_status, pass_message) =
            RValidator::validate_with_context(&passing, ValidationLevel::Syntax, 10, Some(&session))
                .expect("passing snippet validates");
        assert_eq!(pass_status, SnippetStatus::Pass, "{pass_message:?}");
        let (fail_status, _) = RValidator::validate_with_context(&failing, ValidationLevel::Syntax, 10, Some(&session))
            .expect("failing snippet validates");
        assert_eq!(fail_status, SnippetStatus::Fail);

        let top_level_entries: Vec<_> = std::fs::read_dir(directory.path())
            .expect("read working directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect();
        assert!(
            top_level_entries.is_empty(),
            "no scratch entry may be left directly in working_directory: {top_level_entries:?}"
        );
        let scratch_root = directory.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a passing and a failing snippet validation"
        );
    }
}
