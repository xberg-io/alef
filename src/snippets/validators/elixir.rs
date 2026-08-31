use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command};

pub struct ElixirValidator;

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_CHECKER_NAME: &str = "alef_batch_check.exs";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the Elixir batch checker failed without a per-snippet diagnostic";
const BATCH_LINE_PREFIX: &str = "ALEF|";
const BATCH_LINE_FIELD_COUNT: usize = 3;
const BATCH_LINE_SEPARATOR: char = '|';
const BATCH_OK: &str = "ok";

/// The checker prints one line per file and the parse errors it reports are multi-line, so the
/// message travels with its newlines escaped and is restored on the Rust side. ~keep
const ESCAPED_NEWLINE: &str = "\\n";

/// One BEAM start for the whole batch instead of one per snippet — the VM boot, not the parse, is
/// what a per-snippet Elixir run spends its time on. The checker parses every file it is handed
/// and prints a line for each, so one snippet's parse error neither aborts the run nor leaks into
/// another snippet's result; it always exits 0, which is why a missing line, not the exit status,
/// is what marks a snippet unjudged. ~keep
const BATCH_CHECKER_SOURCE: &str = r#"System.argv()
|> Enum.each(fn path ->
  {status, message} =
    case File.read(path) do
      {:ok, content} ->
        case Code.string_to_quoted(content, file: path) do
          {:ok, _} -> {"ok", ""}
          {:error, reason} -> {"err", inspect(reason)}
        end

      {:error, reason} ->
        {"err", "read error: " <> inspect(reason)}
    end

  IO.puts("ALEF|" <> path <> "|" <> status <> "|" <> String.replace(message, "\n", "\\n"))
end)
"#;

impl ElixirValidator {
    fn validate_batch_with_context(
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
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.exs");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, &snippet.code)?;
            file_names.push(file_name);
            paths.push(path);
        }
        let checker_path = dir.path().join(BATCH_CHECKER_NAME);
        std::fs::write(&checker_path, BATCH_CHECKER_SOURCE)?;
        let mut command = std::process::Command::new("elixir");
        command.arg(&checker_path).args(&paths);
        if let Some(session) = session {
            session.apply(&mut command);
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
}

impl SnippetValidator for ElixirValidator {
    fn language(&self) -> Language {
        Language::Elixir
    }

    fn is_available(&self) -> bool {
        which::which("elixir").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = ScratchDir::isolated()?;
        let snippet_path = dir.path().join("snippet.exs");
        std::fs::write(&snippet_path, &snippet.code)?;

        let mut command = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => {
                let checker_path = dir.path().join("check.exs");
                let checker = format!(
                    r#"path = "{}"
case File.read(path) do
  {{:ok, content}} ->
    case Code.string_to_quoted(content) do
      {{:ok, _}} -> System.halt(0)
      {{:error, reason}} ->
        IO.puts("parse error: #{{inspect(reason)}}")
        System.halt(1)
    end
  {{:error, reason}} ->
    IO.puts("file read error: #{{inspect(reason)}}")
    System.halt(1)
end"#,
                    snippet_path.to_string_lossy().replace('\\', "\\\\")
                );
                std::fs::write(&checker_path, checker)?;

                let mut command = std::process::Command::new("elixir");
                command.arg(checker_path);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("elixir");
                command.arg(&snippet_path);
                command
            }
        };

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    // `Code.string_to_quoted` (the only check run at `Syntax`/`Compile`/`TypeCheck`, see the
    // match above and in `validate_in_session` below) parses the AST and nothing more — Elixir
    // resolves remote calls like `Mod.fun()` at runtime, not parse time, so a made-up module
    // parses cleanly. The same call is used for `Syntax` and `Compile` alike, so a `Compile`
    // request silently got the same result as `Syntax` while being reported as if it had
    // validated further. No real check is wired up here: `mix dialyzer` needs an out-of-band PLT
    // this harness doesn't build (first run can take minutes, easily blowing the per-snippet
    // timeout), and compiling with `elixirc` can't tell a genuinely undefined module from one
    // that's legitimately defined elsewhere on a code path this isolated snippet doesn't have —
    // that would trade the false-pass this fixes for false-fails on correct code. Until a real
    // checker is wired up with proper project/PLT context, neither `compile` nor `typecheck` may
    // be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `Code.string_to_quoted` resolve a remote call. Structural, so
    // it's exempted from `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck)
    }

    /// `Run` is declined: each snippet must execute in its own process so its output, exit status
    /// and side effects belong to it alone. Every other level runs the same `Code.string_to_quoted`
    /// parse, which is exactly what the batch checker performs per file. ~keep
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

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let Some(session) = session else {
            return self.validate(snippet, level, timeout_secs);
        };
        let dir = session.scratch_dir()?;
        let file = dir.path().join("snippet.exs");
        std::fs::write(&file, &snippet.code)?;
        let mut command = if level == ValidationLevel::Run {
            let mut value = std::process::Command::new("elixir");
            value.arg(&file);
            value
        } else {
            let mut value = std::process::Command::new("elixir");
            value.args([
                "-e",
                &format!("Code.string_to_quoted!(File.read!({:?}))", file.to_string_lossy()),
            ]);
            value
        };
        session.apply(&mut command);
        let (success, output) = run_command(&mut command, timeout_secs)?;
        Ok(if success {
            (SnippetStatus::Pass, None)
        } else {
            (SnippetStatus::Fail, Some(output))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::runner::{RunnerConfig, run_validation};
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use crate::snippets::validators::ValidatorRegistry;

    /// Whether `elixir` runs, not merely resolves: a version-manager shim (e.g. asdf, mise) spawns
    /// fine then exits non-zero, so `ElixirValidator::is_available`'s PATH-only check would leave
    /// the skip below unreachable and fire the assert everywhere Elixir is absent. ~keep
    fn elixir_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("elixir")
                .arg("--version")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn undefined_symbol_snippet() -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::Elixir,
            title: None,
            code: "ThisModuleDoesNotExistAnywhere12345.call_me()\n".into(),
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

    fn elixir_snippet(code: &str) -> Snippet {
        let mut value = undefined_symbol_snippet();
        value.code = code.into();
        value
    }

    #[test]
    fn batch_declines_run_so_each_snippet_executes_on_its_own() {
        let only = elixir_snippet("IO.puts(\"one\")\n");

        let declined = ElixirValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !elixir_is_runnable() {
            return;
        }
        let first = elixir_snippet("IO.puts(\"one\")\n");
        let second = elixir_snippet("IO.puts(\"two\")\n");
        let third = elixir_snippet("IO.puts(\"three\")\n");

        let results =
            ElixirValidator::validate_batch_with_context(&[&first, &second, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
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
        if !elixir_is_runnable() {
            return;
        }
        let first = elixir_snippet("IO.puts(\"one\")\n");
        let broken = elixir_snippet("defmodule Broken do\n  def value( do\nend\n");
        let third = elixir_snippet("IO.puts(\"three\")\n");

        let results =
            ElixirValidator::validate_batch_with_context(&[&first, &broken, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("missing terminator")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// The checker parses each file into an AST and defines nothing, so two snippets declaring the
    /// same module name — a redefinition the moment anything compiled them together — are judged
    /// independently. ~keep
    #[test]
    fn batch_passes_two_snippets_declaring_the_same_module() {
        if !elixir_is_runnable() {
            return;
        }
        let first = elixir_snippet("defmodule Same do\n  def value, do: 1\nend\n");
        let second = elixir_snippet("defmodule Same do\n  def value, do: 2\nend\n");

        let results =
            ElixirValidator::validate_batch_with_context(&[&first, &second], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// A checker that never reported on a snippet must not let it pass by default: the snippet
    /// fails carrying whatever the toolchain actually said. ~keep
    #[test]
    fn checker_results_fail_every_unreported_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.exs".to_string(), "snippet_batch_1.exs".to_string()];

        let results = ElixirValidator::checker_results(&file_names, "** (RuntimeError) elixir is not installed\n");

        assert_eq!(
            results,
            vec![
                (
                    SnippetStatus::Fail,
                    Some("** (RuntimeError) elixir is not installed".to_string())
                ),
                (
                    SnippetStatus::Fail,
                    Some("** (RuntimeError) elixir is not installed".to_string())
                ),
            ]
        );
    }

    #[test]
    fn checker_results_restore_the_newlines_a_reported_message_carried() {
        let file_names = vec!["snippet_batch_0.exs".to_string()];

        let results =
            ElixirValidator::checker_results(&file_names, "ALEF|/tmp/s/snippet_batch_0.exs|err|first\\nsecond\n");

        assert_eq!(results, vec![(SnippetStatus::Fail, Some("first\nsecond".to_string()))]);
    }

    #[test]
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            ElixirValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(ElixirValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(ElixirValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!ElixirValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// Mirrors the php.rs/ruby.rs regressions: `Code.string_to_quoted` accepts this file (Elixir
    /// resolves remote calls at runtime, not parse time), so `achievable_level` caps it to
    /// `syntax`; because that gap is structural, it is exempted from `Downgraded` the same way a
    /// `max_level` ceiling is — a capability-capped `Pass`, not a claim of `typecheck`. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_symbol_does_not_pass_as_typecheck() {
        if !elixir_is_runnable() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::TypeCheck,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[undefined_symbol_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::TypeCheck),
            "undefined-symbol snippet must not pass claiming typecheck: {result:?}"
        );
        assert_eq!(result.status, SnippetStatus::Pass);
        assert!(result.capability_capped);
        assert_eq!(result.effective_level, ValidationLevel::Syntax);
        assert_eq!(summary.downgraded, 0);
        assert_eq!(summary.capability_capped, 1);
    }

    /// The regression this fix closes: before `achievable_level` also capped `Compile`, this
    /// undefined-symbol snippet passed a `Compile` request as an ordinary, unqualified `Pass` —
    /// `Code.string_to_quoted` accepts it, and nothing distinguished `Compile` from `Syntax`, so
    /// the result carried no `capability_capped` flag and no `downgrade_reason` at all. That is
    /// precisely the silent downgrade this validator must never produce again. ~keep
    #[test]
    fn compile_request_for_an_undefined_symbol_does_not_pass_as_compile() {
        if !elixir_is_runnable() {
            return;
        }
        let registry = ValidatorRegistry::new();
        let config = RunnerConfig {
            level: ValidationLevel::Compile,
            parallelism: 1,
            cache_dir: None,
            ..RunnerConfig::default()
        };

        let summary = run_validation(&[undefined_symbol_snippet()], &registry, &config).expect("validation completes");

        let result = &summary.results[0];
        assert_ne!(
            (result.status, result.effective_level),
            (SnippetStatus::Pass, ValidationLevel::Compile),
            "undefined-symbol snippet must not pass claiming compile: {result:?}"
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
}
