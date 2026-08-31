use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct DartValidator;

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "dart analyze failed without a snippet-specific diagnostic";

/// `--format=machine` emits one `SEVERITY|TYPE|CODE|FILE|LINE|COL|LENGTH|MESSAGE` line per
/// diagnostic. The message is the last field and may itself contain escaped separators, so the
/// split is bounded to the field count rather than greedy. ~keep
const MACHINE_FIELD_COUNT: usize = 8;
const MACHINE_FIELD_SEPARATOR: char = '|';

/// Severities that fail a snippet at `Syntax`, which runs with `--no-fatal-warnings` exactly as
/// the per-snippet path does: a warning leaves that path passing, so it must leave the batch
/// passing too. ~keep
const SYNTAX_FATAL_SEVERITIES: &[&str] = &["ERROR"];

/// `--fatal-infos` makes every diagnostic the analyzer reports fail its snippet. ~keep
const TYPECHECK_FATAL_SEVERITIES: &[&str] = &["ERROR", "WARNING", "INFO"];

struct MachineDiagnostic {
    file: String,
    severity: String,
    text: String,
}

impl DartValidator {
    /// One analyzer start for the whole batch instead of one per snippet. Each snippet is written
    /// as its own file: a Dart file is its own library, so two snippets declaring the same
    /// top-level name — or each declaring `main` — never collide the way they would in a shared
    /// scope. ~keep
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let dir = match session {
            Some(value) => ScratchDir::for_session(value)?,
            None => ScratchDir::isolated()?,
        };
        let mut file_names = Vec::with_capacity(snippets.len());
        let mut paths = Vec::with_capacity(snippets.len());
        for (index, snippet) in snippets.iter().enumerate() {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.dart");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, snippet.code.trim())?;
            file_names.push(file_name);
            paths.push(path);
        }
        let mut command = std::process::Command::new("dart");
        command.args(["analyze", "--format=machine"]);
        command.arg(Self::fatality_flag(level));
        command.args(&paths);
        if let Some(value) = session {
            command.current_dir(Self::project_directory(value));
            value.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::batch_results(&file_names, level, success, &output))
    }

    /// Keeps the exit status and the per-snippet verdict answering the same question: whatever the
    /// level treats as fatal is what the analyzer is told to exit non-zero over, so a batch cannot
    /// report every snippet passing while the process reports failure. ~keep
    fn fatality_flag(level: ValidationLevel) -> &'static str {
        if level == ValidationLevel::TypeCheck {
            "--fatal-infos"
        } else {
            "--no-fatal-warnings"
        }
    }

    fn fatal_severities(level: ValidationLevel) -> &'static [&'static str] {
        if level == ValidationLevel::TypeCheck {
            TYPECHECK_FATAL_SEVERITIES
        } else {
            SYNTAX_FATAL_SEVERITIES
        }
    }

    fn batch_results(file_names: &[String], level: ValidationLevel, success: bool, output: &str) -> BatchValidation {
        let fatal = Self::fatal_severities(level);
        let mut diagnostics = vec![Vec::new(); file_names.len()];
        let mut rejected = vec![false; file_names.len()];
        let mut unmatched = Vec::new();
        for line in output.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let owner = Self::parse_diagnostic(line)
                .and_then(|diagnostic| Self::owner(file_names, &diagnostic.file).map(|index| (index, diagnostic)));
            match owner {
                Some((index, diagnostic)) => {
                    diagnostics[index].push(diagnostic.text);
                    rejected[index] |= fatal.contains(&diagnostic.severity.as_str());
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

    fn parse_diagnostic(line: &str) -> Option<MachineDiagnostic> {
        let fields: Vec<&str> = line.splitn(MACHINE_FIELD_COUNT, MACHINE_FIELD_SEPARATOR).collect();
        if fields.len() < MACHINE_FIELD_COUNT {
            return None;
        }
        let [severity, _, code, file, line_number, column, _, message] = fields[..] else {
            return None;
        };
        let name = std::path::Path::new(file).file_name()?.to_string_lossy().into_owned();
        let name_for_text = name.clone();
        Some(MachineDiagnostic {
            severity: severity.to_string(),
            text: format!("{severity} - {name_for_text}:{line_number}:{column} - {message} - {code}"),
            file: name,
        })
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
        let dir = match session {
            Some(value) => ScratchDir::for_session(value)?,
            None => ScratchDir::isolated()?,
        };
        let file = dir.path().join("snippet.dart");
        std::fs::write(&file, snippet.code.trim())?;
        let mut command = std::process::Command::new("dart");
        match level {
            ValidationLevel::Syntax => {
                command.args(["analyze", "--no-fatal-warnings"]).arg(&file);
            }
            ValidationLevel::Compile => {
                command
                    .args(["compile", "exe", "-o"])
                    .arg(dir.path().join("snippet.aot"))
                    .arg(&file);
            }
            ValidationLevel::TypeCheck => {
                command.args(["analyze", "--fatal-infos"]).arg(&file);
            }
            ValidationLevel::Run => {
                command.arg("run").arg(&file);
            }
        }
        if let Some(value) = session {
            command.current_dir(Self::project_directory(value));
            value.apply_environment(&mut command);
        }
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }

    fn project_directory(session: &ValidationSession) -> &std::path::Path {
        session
            .manifest
            .as_deref()
            .and_then(std::path::Path::parent)
            .unwrap_or(&session.working_directory)
    }
}

impl SnippetValidator for DartValidator {
    fn language(&self) -> Language {
        Language::Dart
    }

    fn is_available(&self) -> bool {
        which::which("dart").is_ok()
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

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    /// Batching covers the analyzer levels only. `Compile` writes one executable per snippet and
    /// `Run` must execute each snippet in its own process, so both fall back to one process per
    /// snippet. ~keep
    fn validate_batch_in_session(
        &self,
        snippets: &[&Snippet],
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        matches!(level, ValidationLevel::Syntax | ValidationLevel::TypeCheck)
            .then(|| Self::validate_batch_with_context(snippets, level, timeout_secs, session))
    }

    fn supports_batching(&self) -> bool {
        true
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        let lower = output.to_ascii_lowercase();
        lower.contains("uri_does_not_exist") || lower.contains("undefined_identifier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const TOOLCHAIN_TEST_TIMEOUT_SECS: u64 = 120;

    /// Whether `dart` runs, not merely resolves: a version-manager shim spawns fine then exits
    /// non-zero, so a spawn/PATH-only check leaves the skip below unreachable and fires the
    /// assert everywhere Dart is absent. ~keep
    fn dart_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("dart")
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    #[test]
    fn batch_declines_the_levels_that_build_or_execute_a_snippet() {
        let only = snippet("void main() {}\n");

        for level in [ValidationLevel::Compile, ValidationLevel::Run] {
            let declined = DartValidator.validate_batch_in_session(&[&only], level, 10, None);
            assert!(
                declined.is_none(),
                "{level:?} must fall back to one process per snippet"
            );
        }
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !dart_is_runnable() {
            return;
        }
        let first = snippet("void main() { print('one'); }\n");
        let second = snippet("void main() { print('two'); }\n");
        let third = snippet("void main() { print('three'); }\n");

        let results = DartValidator::validate_batch_with_context(
            &[&first, &second, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
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
        if !dart_is_runnable() {
            return;
        }
        let first = snippet("void main() { print('one'); }\n");
        let broken = snippet("void main() { this is not dart {{{ }\n");
        let third = snippet("void main() { print('three'); }\n");

        let results = DartValidator::validate_batch_with_context(
            &[&first, &broken, &third],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1].1.as_deref().is_some_and(|message| message.contains("ERROR")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// A Dart file is its own library, which is what lets the batch share one analyzer run: both
    /// snippets declare `main` and a top-level `value`, and neither sees the other's. ~keep
    #[test]
    fn batch_passes_two_snippets_declaring_the_same_top_level_names() {
        if !dart_is_runnable() {
            return;
        }
        let first = snippet("const value = 1;\nvoid main() { print(value); }\n");
        let second = snippet("const value = 2;\nvoid main() { print(value); }\n");

        let results = DartValidator::validate_batch_with_context(
            &[&first, &second],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// The per-snippet path runs `analyze --no-fatal-warnings` at `Syntax` and `--fatal-infos` at
    /// `TypeCheck`; the batch has to keep the same warning fatal at one level and harmless at the
    /// other, or it fails snippets the serial path passes. ~keep
    #[test]
    fn batch_treats_a_warning_as_fatal_only_at_the_typecheck_level() {
        if !dart_is_runnable() {
            return;
        }
        let clean = snippet("void main() { print('one'); }\n");
        let warned = snippet("void main() { int unused = 1; }\n");

        let lenient = DartValidator::validate_batch_with_context(
            &[&clean, &warned],
            ValidationLevel::Syntax,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("syntax batch runs");
        let strict = DartValidator::validate_batch_with_context(
            &[&clean, &warned],
            ValidationLevel::TypeCheck,
            TOOLCHAIN_TEST_TIMEOUT_SECS,
            None,
        )
        .expect("typecheck batch runs");

        assert_eq!(lenient, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
        assert_eq!(strict[0], (SnippetStatus::Pass, None), "{:?}", strict[0]);
        assert_eq!(strict[1].0, SnippetStatus::Fail);
    }

    /// An analyzer that fails without naming any snippet must not let the batch pass: every
    /// snippet carries the real output instead. ~keep
    #[test]
    fn batch_results_fail_every_snippet_when_no_diagnostic_names_one() {
        let file_names = vec!["snippet_batch_0.dart".to_string(), "snippet_batch_1.dart".to_string()];

        let results = DartValidator::batch_results(
            &file_names,
            ValidationLevel::Syntax,
            false,
            "Could not find a file named \"pubspec.yaml\"\n",
        );

        assert_eq!(
            results,
            vec![
                (
                    SnippetStatus::Fail,
                    Some("Could not find a file named \"pubspec.yaml\"".to_string())
                ),
                (
                    SnippetStatus::Fail,
                    Some("Could not find a file named \"pubspec.yaml\"".to_string())
                ),
            ]
        );
    }

    /// Attribution is by the file the diagnostic names, so a batch where only the second snippet
    /// is diagnosed leaves the first and third untouched. ~keep
    #[test]
    fn batch_results_attribute_a_machine_diagnostic_to_the_file_it_names() {
        let file_names = vec![
            "snippet_batch_0.dart".to_string(),
            "snippet_batch_1.dart".to_string(),
            "snippet_batch_2.dart".to_string(),
        ];
        let output = "ERROR|SYNTACTIC_ERROR|EXPECTED_TOKEN|/tmp/scratch/snippet_batch_1.dart|3|7|1|Expected ';'.\n";

        let results = DartValidator::batch_results(&file_names, ValidationLevel::Syntax, false, output);

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Pass, None),
                (
                    SnippetStatus::Fail,
                    Some("ERROR - snippet_batch_1.dart:3:7 - Expected ';'. - EXPECTED_TOKEN".to_string())
                ),
                (SnippetStatus::Pass, None),
            ]
        );
    }

    /// `--format=machine` upper-cases the CODE field (proven by the fixture in
    /// `batch_results_attribute_a_machine_diagnostic_to_the_file_it_names` above), so the
    /// dependency-error classifier must match case-insensitively or a missing-dependency batch
    /// diagnostic is misreported as a genuine snippet defect. ~keep
    #[test]
    fn is_dependency_error_matches_the_upper_cased_machine_format_code() {
        let output = "ERROR|COMPILE_TIME_ERROR|URI_DOES_NOT_EXIST|/tmp/scratch/snippet_batch_0.dart|1|8|20|\
                       Target of URI doesn't exist: 'package:missing_dep/missing_dep.dart'.";

        assert!(
            DartValidator.is_dependency_error(output),
            "machine-format URI_DOES_NOT_EXIST must be recognized as a dependency error: {output:?}"
        );
    }

    #[test]
    fn session_manifest_resolves_a_local_package_outside_the_working_directory() {
        if !dart_is_runnable() {
            return;
        }
        let root = tempfile::tempdir().expect("temporary root");
        let working = root.path().join("working");
        let project = root.path().join("project");
        std::fs::create_dir_all(&working).expect("working directory");
        std::fs::create_dir_all(project.join("lib")).expect("library directory");
        std::fs::write(
            project.join("pubspec.yaml"),
            "name: local_fixture\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n",
        )
        .expect("Dart manifest");
        std::fs::write(project.join("lib/local_fixture.dart"), "const value = 1;\n").expect("Dart library");
        let prepared = std::process::Command::new("dart")
            .args(["pub", "get"])
            .current_dir(&project)
            .status()
            .expect("dart pub get runs");
        assert!(prepared.success());
        let snippet = snippet("import 'package:local_fixture/local_fixture.dart';\nvoid main() { print(value); }");
        let session = ValidationSession {
            language: Language::Dart,
            working_directory: working,
            manifest: Some(project.join("pubspec.yaml")),
            fingerprint: "fixture".into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        };

        let (status, output) =
            DartValidator::validate_with_context(&snippet, ValidationLevel::TypeCheck, 30, Some(&session))
                .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");
    }

    fn scratch_shape_session(project: &std::path::Path, fingerprint: &str) -> ValidationSession {
        ValidationSession {
            language: Language::Dart,
            working_directory: project.to_path_buf(),
            manifest: None,
            fingerprint: fingerprint.into(),
            env: BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: BTreeMap::new(),
        }
    }

    fn scratch_top_level_entries(project: &std::path::Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(project)
            .expect("read project directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect()
    }

    /// Regression: `validate_with_context` used to create its session-scoped scratch directory
    /// directly inside `project_directory(session)` (a tracked package source directory) via a
    /// bare `tempdir_in`, leaving a `.alef-snippet-*/` directory loose in `packages/dart/` after
    /// every run. It must nest under that project's own `.alef/snippets/tmp` cache root instead. ~keep
    #[test]
    fn session_scratch_resolves_under_the_cache_root_not_the_project_directory() {
        if !dart_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-shape-fixture");
        let snippet = snippet("void main() { print('ok'); }\n");

        let (status, output) =
            DartValidator::validate_with_context(&snippet, ValidationLevel::Syntax, 30, Some(&session))
                .expect("validation runs");
        assert_eq!(status, SnippetStatus::Pass, "{output:?}");

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory: {leftovers:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails `dart analyze` must
    /// not leave its scratch directory behind under the project directory any more than a
    /// passing one does.
    #[test]
    fn session_scratch_is_removed_after_a_run_that_fails() {
        if !dart_is_runnable() {
            return;
        }
        let project = tempfile::tempdir().expect("project directory");
        let session = scratch_shape_session(project.path(), "scratch-cleanup-fixture");
        let snippet = snippet("this does not parse as dart {{{\n");

        let (status, _) = DartValidator::validate_with_context(&snippet, ValidationLevel::Syntax, 30, Some(&session))
            .expect("validation runs");
        assert_eq!(status, SnippetStatus::Fail);

        let leftovers = scratch_top_level_entries(project.path());
        assert!(
            leftovers.is_empty(),
            "no scratch entry may be left directly in the project directory after a failing run: {leftovers:?}"
        );
        let scratch_root = project.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("snippet.dart"),
            language: Language::Dart,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: PathBuf::from("snippet.dart"),
                line: 1,
                block_index: 0,
            },
        }
    }
}
