use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct DartValidator;

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
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.dart");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("dart");
        match level {
            ValidationLevel::Syntax => {
                command.args(["analyze", "--no-fatal-warnings"]).arg(&file);
            }
            ValidationLevel::Compile | ValidationLevel::TypeCheck => {
                let out = dir.path().join("snippet.aot");
                command.args(["compile", "exe", "-o"]).arg(&out).arg(&file);
            }
            ValidationLevel::Run => {
                command.arg("run").arg(&file);
            }
        }

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

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("uri_does_not_exist") || output.contains("undefined_identifier")
    }
}
