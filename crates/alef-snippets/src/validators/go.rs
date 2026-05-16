use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct GoValidator;

impl SnippetValidator for GoValidator {
    fn language(&self) -> Language {
        Language::Go
    }

    fn is_available(&self) -> bool {
        which::which("go").is_ok() || which::which("gofmt").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let file = dir.path().join("snippet.go");
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = match level {
            ValidationLevel::Syntax => {
                let mut command = std::process::Command::new("gofmt");
                command.args(["-e", "-l"]).arg(&file);
                command
            }
            ValidationLevel::Compile => {
                std::fs::write(dir.path().join("go.mod"), "module snippet\n\ngo 1.21\n")?;
                let mut command = std::process::Command::new("go");
                command.args(["build", "-o", "/dev/null", "./..."]).current_dir(dir.path());
                command
            }
            ValidationLevel::Run => {
                std::fs::write(dir.path().join("go.mod"), "module snippet\n\ngo 1.21\n")?;
                let mut command = std::process::Command::new("go");
                command.arg("run").arg(&file);
                command
            }
        };

        let (success, output) = run_command(&mut command, timeout_secs)?;
        if success && output.trim().is_empty() {
            Ok((SnippetStatus::Pass, None))
        } else if success {
            Ok((SnippetStatus::Pass, None))
        } else {
            Ok((SnippetStatus::Fail, Some(output)))
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("undefined:") || output.contains("cannot find package") || output.contains("no required module")
    }
}
