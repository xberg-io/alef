use crate::snippets::error::Result;
use crate::snippets::scratch::ScratchDir;
use crate::snippets::session::ValidationSession;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{BatchValidation, SnippetValidator, run_command, run_script};

pub struct RubyValidator;

const BATCH_FILE_PREFIX: &str = "snippet_batch_";
const BATCH_CHECKER_NAME: &str = "alef_batch_check.rb";
const BATCH_FAILED_WITHOUT_DIAGNOSTIC: &str = "the Ruby batch checker failed without a per-snippet diagnostic";

/// One interpreter start for the whole batch instead of one per snippet. `ruby -c` cannot be
/// pointed at several files — it syntax-checks the first and hands the rest to the script as
/// `ARGV`, reporting a single `Syntax OK` that says nothing about them — so the check is driven
/// from a script that parses every file it is handed and prints one JSON line each. It always
/// exits 0, which is why a missing line, not the exit status, is what marks a snippet unjudged.
/// `RubyVM::AbstractSyntaxTree` is the same parser `ruby -c` runs; `Ripper` stands in on the
/// implementations that do not expose it, at the cost of the parser's own message. ~keep
const BATCH_CHECKER_SOURCE: &str = r##"require 'json'
require 'ripper'
ARGV.each do |path|
  begin
    source = File.read(path)
    if defined?(RubyVM::AbstractSyntaxTree)
      RubyVM::AbstractSyntaxTree.parse(source)
    elsif Ripper.sexp(source).nil?
      raise SyntaxError, 'syntax error'
    end
    puts JSON.generate({ 'path' => path, 'ok' => true, 'error' => '' })
  rescue SyntaxError, StandardError => error
    puts JSON.generate({ 'path' => path, 'ok' => false, 'error' => "#{error.class}: #{error.message}" })
  end
end
"##;

impl RubyValidator {
    fn validate_batch_with_context(
        snippets: &[&Snippet],
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<BatchValidation> {
        let checked: Vec<usize> = snippets
            .iter()
            .enumerate()
            .filter(|(_, snippet)| !is_api_signature(snippet.code.trim()))
            .map(|(index, _)| index)
            .collect();
        if checked.is_empty() {
            return Ok(vec![(SnippetStatus::Pass, None); snippets.len()]);
        }
        let dir = match session {
            Some(session) => ScratchDir::for_session(session)?,
            None => ScratchDir::isolated()?,
        };
        let mut file_names = Vec::with_capacity(checked.len());
        let mut paths = Vec::with_capacity(checked.len());
        for index in &checked {
            let file_name = format!("{BATCH_FILE_PREFIX}{index}.rb");
            let path = dir.path().join(&file_name);
            std::fs::write(&path, &snippets[*index].code)?;
            file_names.push(file_name);
            paths.push(path);
        }
        let checker_path = dir.path().join(BATCH_CHECKER_NAME);
        std::fs::write(&checker_path, BATCH_CHECKER_SOURCE)?;
        let mut command = std::process::Command::new("ruby");
        command.arg(&checker_path).args(&paths);
        if let Some(session) = session {
            session.apply(&mut command);
            command.env("RUBYLIB", &session.working_directory);
        }
        let (_, output) = run_command(&mut command, timeout_secs)?;
        Ok(Self::place(
            snippets.len(),
            &checked,
            Self::checker_results(&file_names, &output),
        ))
    }

    /// Restores input order across the snippets the checker never saw: an API-signature snippet
    /// passes without a parse exactly as it does on the per-snippet path, and holding its slot
    /// here is what keeps result *i* the verdict on snippet *i*. ~keep
    fn place(total: usize, checked: &[usize], results: BatchValidation) -> BatchValidation {
        let mut placed = vec![(SnippetStatus::Pass, None); total];
        for (position, index) in checked.iter().enumerate() {
            if let Some(value) = results.get(position) {
                placed[*index] = value.clone();
            }
        }
        placed
    }

    /// Maps the checker's JSON lines back to the snippet that owns each file. A snippet the
    /// checker never reported on fails carrying the real output rather than passing by default. ~keep
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
        let entry = serde_json::from_str::<serde_json::Value>(line).ok()?;
        let path = entry.get("path").and_then(serde_json::Value::as_str)?;
        let index = Self::owner(file_names, path)?;
        let failed = entry.get("ok").and_then(serde_json::Value::as_bool) != Some(true);
        let message = entry
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let value = if failed {
            (SnippetStatus::Fail, Some(message))
        } else {
            (SnippetStatus::Pass, None)
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

impl SnippetValidator for RubyValidator {
    fn language(&self) -> Language {
        Language::Ruby
    }

    fn is_available(&self) -> bool {
        which::which("ruby").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }

        run_script(snippet, level, timeout_secs, None, ".rb", "ruby", &["-c"])
    }

    fn validate_in_session(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
        session: Option<&ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        if is_api_signature(snippet.code.trim()) {
            return Ok((SnippetStatus::Pass, None));
        }
        run_script(snippet, level, timeout_secs, session, ".rb", "ruby", &["-c"])
    }

    /// `Run` is declined: each snippet must execute in its own process so its output, exit status
    /// and side effects belong to it alone. Every other level runs the same syntax check `ruby -c`
    /// performs, which is what the batch checker applies per file. ~keep
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

    // `ruby -c` (the only check `validate` ever runs below `Run`, see above) is a syntax check: it
    // never resolves a constant, method, or class, and `run_script` sends it the identical `-c`
    // invocation for both `Syntax` and `Compile` — there is no separate compile step at all, so a
    // `Compile` request silently got the same result as `Syntax` while being reported as if it had
    // validated further. No real Ruby type-checker is wired up here either — Sorbet and RBS both
    // require project-wide `# typed:` sigils/signatures the generated snippets don't carry, so
    // pointing either at a lone temp file would either no-op (untyped files get minimal checking)
    // or hard-error on the missing project setup, neither of which is an improvement over an
    // honest downgrade. Until that's built, neither `compile` nor `typecheck` may be claimed. ~keep
    fn achievable_level(&self, requested: ValidationLevel) -> ValidationLevel {
        if matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck) {
            ValidationLevel::Syntax
        } else {
            ValidationLevel::Run
        }
    }

    // The compile/typecheck gap above is a property of this validator's implementation (no
    // distinct compile step and no checker is wired up), not of the machine running it — no
    // environment will ever make `ruby -c` resolve a constant. Structural, so it's exempted from
    // `Downgraded` the same way `max_level` is. ~keep
    fn achievable_level_is_structural(&self, requested: ValidationLevel) -> bool {
        matches!(requested, ValidationLevel::Compile | ValidationLevel::TypeCheck)
    }
}

fn is_api_signature(code: &str) -> bool {
    code.lines().count() <= 3 && code.contains(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::runner::{RunnerConfig, run_validation};
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};
    use crate::snippets::validators::ValidatorRegistry;

    /// Whether `ruby` runs, not merely resolves: a version-manager shim (e.g. rbenv, rvm, asdf)
    /// spawns fine then exits non-zero, so `RubyValidator::is_available`'s PATH-only check would
    /// leave the skip below unreachable and fire the assert everywhere Ruby is absent. ~keep
    fn ruby_is_runnable() -> bool {
        static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNABLE.get_or_init(|| {
            std::process::Command::new("ruby")
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
            language: Language::Ruby,
            title: None,
            code: "ThisConstantDoesNotExistAnywhere12345.call_me\n".into(),
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

    fn ruby_snippet(code: &str) -> Snippet {
        let mut value = undefined_symbol_snippet();
        value.code = code.into();
        value
    }

    #[test]
    fn batch_declines_run_so_each_snippet_executes_on_its_own() {
        let only = ruby_snippet("puts \"one\"\n");

        let declined = RubyValidator.validate_batch_in_session(&[&only], ValidationLevel::Run, 10, None);

        assert!(declined.is_none());
    }

    #[test]
    fn batch_returns_one_result_per_snippet_in_input_order() {
        if !ruby_is_runnable() {
            return;
        }
        let first = ruby_snippet("puts \"one\"\n");
        let second = ruby_snippet("puts \"two\"\n");
        let third = ruby_snippet("puts \"three\"\n");

        let results =
            RubyValidator::validate_batch_with_context(&[&first, &second, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
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
        if !ruby_is_runnable() {
            return;
        }
        let first = ruby_snippet("puts \"one\"\n");
        let broken = ruby_snippet("def broken(\n");
        let third = ruby_snippet("puts \"three\"\n");

        let results =
            RubyValidator::validate_batch_with_context(&[&first, &broken, &third], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("batch validation runs");

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (SnippetStatus::Pass, None), "{:?}", results[0]);
        assert_eq!(results[1].0, SnippetStatus::Fail);
        assert!(
            results[1]
                .1
                .as_deref()
                .is_some_and(|message| message.contains("SyntaxError")),
            "{:?}",
            results[1].1
        );
        assert_eq!(results[2], (SnippetStatus::Pass, None), "{:?}", results[2]);
    }

    /// The checker parses each file and evaluates none of it, so two snippets reopening the same
    /// class — which one interpreter evaluating both would merge into a single definition — are
    /// judged independently. ~keep
    #[test]
    fn batch_passes_two_snippets_defining_the_same_class() {
        if !ruby_is_runnable() {
            return;
        }
        let first = ruby_snippet("class Same\n  def value\n    1\n  end\nend\n");
        let second = ruby_snippet("class Same\n  def value\n    2\n  end\nend\n");

        let results = RubyValidator::validate_batch_with_context(&[&first, &second], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
            .expect("batch validation runs");

        assert_eq!(results, vec![(SnippetStatus::Pass, None), (SnippetStatus::Pass, None)]);
    }

    /// An API-signature snippet passes without reaching the interpreter on the per-snippet path,
    /// so the batch must hold its slot rather than shift every later snippet's verdict up by one. ~keep
    #[test]
    fn batch_keeps_input_order_when_a_snippet_never_reaches_the_interpreter() {
        if !ruby_is_runnable() {
            return;
        }
        let signature = ruby_snippet("value(name) -> String\n");
        let broken = ruby_snippet("def broken(\n");
        assert!(is_api_signature(signature.code.trim()));

        let results =
            RubyValidator::validate_batch_with_context(&[&signature, &broken], TOOLCHAIN_TEST_TIMEOUT_SECS, None)
                .expect("batch validation runs");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (SnippetStatus::Pass, None));
        assert_eq!(results[1].0, SnippetStatus::Fail);
    }

    /// A checker that never reported on a snippet must not let it pass by default: the snippet
    /// fails carrying whatever the toolchain actually said. ~keep
    #[test]
    fn checker_results_fail_every_unreported_snippet_with_the_real_output() {
        let file_names = vec!["snippet_batch_0.rb".to_string(), "snippet_batch_1.rb".to_string()];

        let results = RubyValidator::checker_results(&file_names, "ruby: no such file or directory\n");

        assert_eq!(
            results,
            vec![
                (SnippetStatus::Fail, Some("ruby: no such file or directory".to_string())),
                (SnippetStatus::Fail, Some("ruby: no such file or directory".to_string())),
            ]
        );
    }

    #[test]
    fn achievable_level_caps_compile_and_typecheck_to_syntax() {
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::TypeCheck),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Compile),
            ValidationLevel::Syntax
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Syntax),
            ValidationLevel::Run
        );
        assert_eq!(
            RubyValidator.achievable_level(ValidationLevel::Run),
            ValidationLevel::Run
        );
    }

    #[test]
    fn achievable_level_compile_and_typecheck_gap_is_structural() {
        assert!(RubyValidator.achievable_level_is_structural(ValidationLevel::TypeCheck));
        assert!(RubyValidator.achievable_level_is_structural(ValidationLevel::Compile));
        assert!(!RubyValidator.achievable_level_is_structural(ValidationLevel::Run));
    }

    /// Mirrors the php.rs regression: `ruby -c` accepts this file (it never resolves constants),
    /// so `achievable_level` caps it to `syntax`; because that gap is structural, it is exempted
    /// from `Downgraded` the same way a `max_level` ceiling is — a capability-capped `Pass`, not
    /// a claim of `typecheck`. ~keep
    #[test]
    fn typecheck_request_for_an_undefined_symbol_does_not_pass_as_typecheck() {
        if !ruby_is_runnable() {
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
    /// `ruby -c` accepts it, and nothing distinguished `Compile` from `Syntax`, so the result
    /// carried no `capability_capped` flag and no `downgrade_reason` at all. That is precisely the
    /// silent downgrade this validator must never produce again. ~keep
    #[test]
    fn compile_request_for_an_undefined_symbol_does_not_pass_as_compile() {
        if !ruby_is_runnable() {
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
