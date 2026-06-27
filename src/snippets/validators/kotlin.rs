use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct KotlinValidator;

impl SnippetValidator for KotlinValidator {
    fn language(&self) -> Language {
        Language::Kotlin
    }

    fn is_available(&self) -> bool {
        which::which("kotlinc").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.kt");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = std::process::Command::new("kotlinc");
        match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                let out = dir.path().join("out");
                command.args(["-nowarn", "-d"]).arg(&out).arg(&file);
            }
            // Strict type-check: treat warnings as errors. `kotlinc` resolves and checks types when
            // producing class output without needing the native library, so this is the type-check
            // gate for generated Kotlin.
            ValidationLevel::TypeCheck => {
                let out = dir.path().join("out");
                command.args(["-Werror", "-d"]).arg(&out).arg(&file);
            }
            ValidationLevel::Run => {
                let out = dir.path().join("out.jar");
                command.args(["-include-runtime", "-d"]).arg(&out).arg(&file);
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
        // `kotlinc` type-checks while producing class files; running a bare snippet needs a runtime
        // jar and an entry point, so the static type-check is the deepest reliable level.
        ValidationLevel::TypeCheck
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("unresolved reference") || output.contains("expecting an element")
    }
}
