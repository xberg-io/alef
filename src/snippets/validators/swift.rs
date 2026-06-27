use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct SwiftValidator;

impl SnippetValidator for SwiftValidator {
    fn language(&self) -> Language {
        Language::Swift
    }

    fn is_available(&self) -> bool {
        which::which("swiftc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.swift");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("swiftc");
        match level {
            ValidationLevel::Syntax => {
                command.args(["-parse"]).arg(&file);
            }
            ValidationLevel::Compile => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
            }
            // Strict type-check without code generation or linking: `-typecheck` runs the full type
            // checker but emits no binary, so generated Swift is verified without the native library,
            // and `-warnings-as-errors` makes any diagnostic fail the gate.
            ValidationLevel::TypeCheck => {
                command.args(["-typecheck", "-warnings-as-errors"]).arg(&file);
            }
            ValidationLevel::Run => {
                let out = dir.path().join("snippet");
                command.args(["-o"]).arg(&out).arg(&file);
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
        // `-typecheck` is the deepest reliable level: a full `-o` compile links and a bare snippet
        // generally cannot be executed, so type-checking is the static ceiling.
        ValidationLevel::TypeCheck
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("no such module") || output.contains("cannot find") && output.contains("in scope")
    }
}
