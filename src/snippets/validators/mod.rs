pub mod bash;
pub mod c;
pub mod csharp;
pub mod dart;
pub mod documentation;
pub mod elixir;
pub mod go;
pub mod java;
pub mod json_validator;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod r;
pub mod ruby;
pub mod rust;
pub mod swift;
pub mod toml_validator;
pub mod typescript;
pub mod yaml_validator;
pub mod zig;

use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use std::collections::HashMap;
use std::io::Write;

mod native_library;
mod process;
mod session_artifacts;

pub use process::{CapturedStreams, run_command, run_command_streams};

#[cfg(test)]
#[path = "dependency_error_classification_tests.rs"]
mod dependency_error_classification_tests;

#[cfg(test)]
pub(crate) fn jvm_toolchain_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub type SnippetValidation = (SnippetStatus, Option<String>);
pub type BatchValidation = Vec<SnippetValidation>;

pub trait SnippetValidator: Send + Sync {
    fn language(&self) -> Language;
    fn is_available(&self) -> bool;
    fn is_available_at(&self, _level: ValidationLevel) -> bool {
        self.is_available()
    }
    /// Validate a snippet at the requested level.
    ///
    /// # Errors
    ///
    /// Returns an error when the validator cannot execute its underlying toolchain.
    fn validate(&self, snippet: &Snippet, level: ValidationLevel, timeout_secs: u64) -> Result<SnippetValidation>;
    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<SnippetValidation> {
        if session.is_some() {
            return Err(crate::snippets::error::Error::Other(format!(
                "{} validator does not support binding-aware sessions",
                self.language()
            )));
        }
        self.validate(snippet, level, timeout_secs)
    }
    fn validate_batch_in_session(
        &self,
        _snippets: &[&Snippet],
        _level: ValidationLevel,
        _timeout_secs: u64,
        _session: Option<&ValidationSession>,
    ) -> Option<Result<BatchValidation>> {
        None
    }
    fn max_level(&self) -> ValidationLevel;

    /// The highest level this run's environment can actually reach for `requested`, as opposed
    /// to `max_level`'s fixed per-language ceiling. A validator whose deeper levels depend on a
    /// tool that may not be installed (a real type-checker, for instance) overrides this to
    /// report the level it can genuinely back up right now; the runner treats anything below
    /// `requested` as a real downgrade rather than a capability ceiling, because — unlike
    /// `max_level` — the limit could lift on a different machine. Default: no environmental
    /// limit beyond `max_level`. ~keep
    fn achievable_level(&self, _requested: ValidationLevel) -> ValidationLevel {
        ValidationLevel::Run
    }

    /// Whether the gap `achievable_level` reports for `requested` is structural — no check for
    /// that level is wired up in this validator at all, on any machine — as opposed to
    /// `achievable_level`'s default meaning of a real tool that merely happens to be missing from
    /// this run's environment. The runner exempts a structural gap from `Downgraded` the same way
    /// it exempts `max_level`, because it is unsatisfiable however healthy the environment is; an
    /// environmental gap keeps its `Downgraded` status so a genuinely broken environment is never
    /// silently waved through. Default: environmental, not structural. ~keep
    fn achievable_level_is_structural(&self, _requested: ValidationLevel) -> bool {
        false
    }

    fn is_dependency_error(&self, _error_output: &str) -> bool {
        false
    }

    /// Whether `validate_batch_in_session` is genuinely wired up to run one process for many
    /// snippets, as opposed to the default trait implementation, which always returns `None` and
    /// lets the runner fall back to one process per snippet. The runner uses this to decide
    /// upfront whether a group is a real batch — and should be logged and dispatched as one — or
    /// should skip the batch path entirely and go straight to the per-snippet fallback. Without
    /// this, every language was logged as `Starting batched snippet validation` regardless of
    /// batching support, and a validator that doesn't support it silently fell through to a
    /// codepath that log line never covered, leaving no matching `Finished` event. That is purely
    /// an observability gap in the batch/fallback dispatch path, not a signal about whether the
    /// validator itself ran or hung — a healthy, fully-passing language is just as silent there as
    /// a broken one. ~keep
    /// Whether two snippets of this language may run concurrently within one validation session.
    ///
    /// ~keep Most validators write every file they touch into a `ScratchDir`, which
    /// `tempfile::Builder::tempdir_in` allocates fresh per call, so they are already safe to run
    /// side by side. TypeScript, C# and Java instead write fixed-name files (`snippet.ts` +
    /// `tsconfig.json`, `Program.cs` + `Snippet.csproj`, `<Class>.java` + its `.class` output)
    /// directly into the session's shared fingerprint-keyed workspace, two of them compiling with
    /// `current_dir` set to it -- so concurrent snippets would overwrite each other's sources
    /// mid-compile and silently validate the wrong code. Kotlin keeps its sources in scratch but
    /// truncate-writes a fixed-path Gradle init script into the same shared workspace. Those four
    /// were made shared by `6ee684237`, which introduced the session mutex in the same commit;
    /// the mutex was then applied to every language, serializing validators that never needed it.
    /// A language returning `false` here still gets its own session and caches -- only the
    /// mutual exclusion is dropped.
    fn requires_session_exclusivity(&self) -> bool {
        false
    }

    fn supports_batching(&self) -> bool {
        false
    }

    /// Build artifacts that must already be on disk before ANY snippet of this language can be
    /// validated in `session` at `level` -- the files a `alef build` produces and a session's
    /// manifest then points its toolchain at. A non-empty answer means every snippet claiming
    /// this session is unsatisfiable *by construction*, so the runner reports them once and
    /// spawns nothing (see `runner::artifact_preflight`).
    ///
    /// Three rules keep this from becoming the vacuous gate its predecessor was -- alef removed a
    /// static `enforce_build_dependency` pre-flight that read only session *config* shape and
    /// bailed for languages whose validator needed no build at all:
    ///
    /// - Every path returned must be one this session's own manifest names, resolved and probed on
    ///   the filesystem. Never a guessed conventional location, never an inference from config
    ///   shape, never "this language usually needs a build".
    /// - The default is empty, i.e. satisfiable. A validator that builds its snippets purely from
    ///   source, or one whose requirement cannot be read off the manifest, keeps validating
    ///   exactly as before -- an unimplemented probe costs the old behaviour, never a false skip.
    /// - `Syntax` never reaches here: it resolves nothing, so no artifact can be required for it.
    ///
    /// A false positive is far more expensive than a false negative here (it silently stops
    /// checking a corpus that would have checked fine), so an implementation that cannot answer
    /// with evidence must answer "nothing missing". ~keep
    fn missing_session_artifacts(
        &self,
        _session: &ValidationSession,
        _level: ValidationLevel,
    ) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}

pub struct ValidatorRegistry {
    validators: HashMap<Language, Box<dyn SnippetValidator>>,
}

impl ValidatorRegistry {
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            validators: HashMap::new(),
        };

        registry.register(Box::new(rust::RustValidator));
        registry.register(Box::new(python::PythonValidator));
        registry.register(Box::new(typescript::TypeScriptValidator));
        registry.register(Box::new(php::PhpValidator));
        registry.register(Box::new(ruby::RubyValidator));
        registry.register(Box::new(elixir::ElixirValidator));
        registry.register(Box::new(bash::BashValidator));
        registry.register(Box::new(toml_validator::TomlValidator));
        registry.register(Box::new(c::CValidator));
        registry.register(Box::new(csharp::CsharpValidator));
        registry.register(Box::new(dart::DartValidator));
        registry.register(Box::new(go::GoValidator));
        registry.register(Box::new(java::JavaValidator));
        registry.register(Box::new(kotlin::KotlinValidator));
        registry.register(Box::new(swift::SwiftValidator::default()));
        registry.register(Box::new(zig::ZigValidator));
        registry.register(Box::new(json_validator::JsonValidator));
        registry.register(Box::new(yaml_validator::YamlValidator));
        registry.register(Box::new(r::RValidator));
        registry.register(Box::new(documentation::TextValidator));
        registry.register(Box::new(documentation::MermaidValidator));
        registry.register(Box::new(documentation::PowerShellValidator));
        registry.register(Box::new(documentation::XmlValidator));
        registry.register(Box::new(documentation::DockerValidator));

        registry
    }

    pub(crate) fn register(&mut self, validator: Box<dyn SnippetValidator>) {
        self.validators.insert(validator.language(), validator);
    }

    #[must_use]
    pub fn get(&self, language: Language) -> Option<&dyn SnippetValidator> {
        self.validators.get(&language).map(Box::as_ref)
    }

    /// Every language this registry can validate, sorted.
    ///
    /// Exists so contracts that must hold for *all* languages — the scratch destination above all,
    /// since the defect it guards against was runners disagreeing with each other — can be
    /// asserted over the registered set rather than over a hand-written list that silently stops
    /// covering the next language someone registers. ~keep
    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        let mut languages: Vec<Language> = self.validators.keys().copied().collect();
        languages.sort_unstable();
        languages
    }
}

impl Default for ValidatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared narrowing every "all diagnostics must match" `is_dependency_error` implementation
/// needs: reclassifying a `Fail` into `Unavailable` is safe only when EVERY error-severity line
/// in `output` is unambiguously a missing-dependency shape, never when the output mixes one with
/// a genuine code defect (task #130/#215 -- a mixed batch relabeled a real bug as an environment
/// gap). Before this existed, `typescript.rs`, `csharp.rs` and `rust.rs` each hand-rolled the
/// identical "collect the lines that are a diagnostic, bail on empty, then require `all()` to
/// match one of a pattern list" loop -- the exact "two components decide one fact separately"
/// shape this repo's tasks keep surfacing, just duplicated three times instead of two.
/// `is_diagnostic_line` picks out the lines that carry an error-severity diagnostic at all
/// (`": error: "` for javac/rustc-style compilers, `"error TS"` for `tsc`, ...); `matches` then
/// decides whether one such line is a dependency shape. Both take a predicate rather than a fixed
/// substring so a language whose diagnostic marker also needs excluding some lines (rustc's
/// `aborting due to`/`could not compile` summary lines, which start with `error` too but carry no
/// classification signal) can express that in the predicate instead of forcing every caller
/// through one fixed `contains`. ~keep
pub(crate) fn all_error_lines_match(
    output: &str,
    is_diagnostic_line: impl Fn(&str) -> bool,
    matches: impl Fn(&str) -> bool,
) -> bool {
    let diagnostic_lines: Vec<&str> = output.lines().filter(|line| is_diagnostic_line(line)).collect();
    if diagnostic_lines.is_empty() {
        return false;
    }
    diagnostic_lines.iter().all(|line| matches(line))
}

fn append_r_library_path(command: &mut std::process::Command, directory: &std::path::Path) -> Result<()> {
    let existing = command
        .get_envs()
        .find(|(key, _)| *key == "R_LIBS_USER")
        .map(|(_, value)| value.map(std::ffi::OsStr::to_os_string))
        .unwrap_or_else(|| std::env::var_os("R_LIBS_USER"));
    let mut directories = existing
        .as_ref()
        .map(|value| std::env::split_paths(value).collect::<Vec<_>>())
        .unwrap_or_default();
    if !directories.iter().any(|path| path == directory) {
        directories.push(directory.to_path_buf());
    }
    let joined = std::env::join_paths(directories)
        .map_err(|error| crate::snippets::error::Error::Other(format!("preserving R library search paths: {error}")))?;
    command.env("R_LIBS_USER", joined);
    Ok(())
}

pub fn run_script(
    snippet: &Snippet,
    level: ValidationLevel,
    timeout_secs: u64,
    session: Option<&ValidationSession>,
    suffix: &str,
    program: &str,
    syntax_arguments: &[&str],
) -> Result<(SnippetStatus, Option<String>)> {
    let scratch_dir = session.map(ScratchDir::for_session).transpose()?;
    let mut source = match &scratch_dir {
        Some(dir) => tempfile::Builder::new().suffix(suffix).tempfile_in(dir.path())?,
        None => tempfile::Builder::new().suffix(suffix).tempfile()?,
    };
    source.write_all(snippet.code.as_bytes())?;
    source.flush()?;
    let mut command = std::process::Command::new(program);
    if level == ValidationLevel::Run {
        command.arg(source.path());
    } else {
        command.args(syntax_arguments).arg(source.path());
    }
    if let Some(value) = session {
        value.apply(&mut command);
        command.env("RUBYLIB", &value.working_directory);
        if value.language == Language::R {
            append_r_library_path(&mut command, &value.working_directory)?;
        }
    }
    let (success, output) = run_command(&mut command, timeout_secs)?;
    Ok(if success {
        (SnippetStatus::Pass, None)
    } else {
        (SnippetStatus::Fail, Some(output))
    })
}

#[cfg(test)]
mod all_error_lines_match_tests {
    use super::all_error_lines_match;

    #[test]
    fn empty_when_no_line_matches_the_diagnostic_predicate() {
        assert!(!all_error_lines_match(
            "nothing to see here\n",
            |line| line.contains("error"),
            |_| true
        ));
    }

    #[test]
    fn true_only_when_every_diagnostic_line_matches() {
        let output = "a: error: X missing\nb: error: X missing\n";
        assert!(all_error_lines_match(
            output,
            |line| line.contains("error"),
            |line| { line.contains("missing") }
        ));
    }

    #[test]
    fn false_when_one_diagnostic_line_does_not_match() {
        let output = "a: error: X missing\nb: error: real defect\n";
        assert!(!all_error_lines_match(
            output,
            |line| line.contains("error"),
            |line| { line.contains("missing") }
        ));
    }

    #[test]
    fn non_diagnostic_lines_are_never_consulted_by_the_match_predicate() {
        let output = "a: error: X missing\nsome unrelated context line\n";
        assert!(all_error_lines_match(
            output,
            |line| line.contains("error"),
            |line| { line.contains("missing") }
        ));
    }
}

#[cfg(all(test, unix))]
mod tests {
    fn script_session(working_directory: std::path::PathBuf) -> crate::snippets::session::ValidationSession {
        crate::snippets::session::ValidationSession {
            language: crate::snippets::types::Language::Bash,
            working_directory,
            manifest: None,
            fingerprint: "run-script-scratch-fixture".into(),
            env: std::collections::BTreeMap::new(),
            include_paths: Vec::new(),
            rust_features: Vec::new(),
            rust_dependencies: std::collections::BTreeMap::new(),
        }
    }

    fn script_snippet(code: &str) -> crate::snippets::types::Snippet {
        crate::snippets::types::Snippet {
            id: None,
            path: "example.md".into(),
            language: crate::snippets::types::Language::Bash,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: crate::snippets::types::SnippetMetadata::default(),
            source_origin: crate::snippets::types::SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    /// Regression: `run_script` (shared by bash/php/r/ruby) used to write its scratch file
    /// directly into `session.working_directory` via a bare `tempfile_in`, producing an
    /// untracked `.tmp<random><suffix>` file with no `.gitignore` coverage — the exact shape of
    /// the git-visible litter this fix closes. It must resolve under the session's own cache
    /// tree instead, leaving nothing behind at the top level of `working_directory` at all. ~keep
    #[test]
    fn run_script_resolves_scratch_under_the_cache_root_not_directly_in_working_directory() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("true\n");

        let (status, _) = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "true",
            &[],
        )
        .expect("run_script runs");

        assert_eq!(status, super::SnippetStatus::Pass);
        let top_level_entries: Vec<_> = std::fs::read_dir(working.path())
            .expect("read working directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .filter(|name| name != ".alef")
            .collect();
        assert!(
            top_level_entries.is_empty(),
            "run_script must not leave any scratch entry directly in working_directory: {top_level_entries:?}"
        );
    }

    /// Pins cleanup on the failure path specifically: a snippet that fails validation must not
    /// leave its scratch directory behind any more than a passing one does.
    #[test]
    fn run_script_removes_scratch_after_a_run_that_fails() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("false\n");

        let (status, _) = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "false",
            &[],
        )
        .expect("run_script runs");

        assert_eq!(status, super::SnippetStatus::Fail);
        let scratch_root = working.path().join(".alef/snippets/tmp");
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.filter_map(|entry| entry.ok()).count())
            .unwrap_or(0);
        assert_eq!(
            remaining, 0,
            "scratch left behind under the cache root after a failing snippet validation"
        );
    }

    /// The load-bearing exit path, and the one explicit cleanup calls always missed: `run_script`
    /// returns `Err` from `?` — here because the toolchain is not installed at all, elsewhere
    /// because the child timed out — long before any cleanup statement at the bottom of the
    /// function could run. Only a `Drop` guard covers this, so if scratch ever survives here the
    /// mechanism has silently regressed to explicit cleanup. ~keep
    #[test]
    fn run_script_removes_scratch_when_the_run_returns_an_error() {
        let working = tempfile::tempdir().expect("working directory");
        let session = script_session(working.path().to_path_buf());
        let snippet = script_snippet("true\n");

        let error = super::run_script(
            &snippet,
            super::ValidationLevel::Syntax,
            5,
            Some(&session),
            ".sh",
            "alef-nonexistent-toolchain-for-scratch-test",
            &[],
        )
        .expect_err("a missing toolchain must surface as an error, not a status");

        assert!(matches!(error, crate::snippets::error::Error::Other(_)));
        let scratch_root = session.scratch_root();
        let remaining = std::fs::read_dir(&scratch_root)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        assert_eq!(
            remaining,
            0,
            "scratch left behind under {} after run_script returned an error",
            scratch_root.display()
        );
    }
}
