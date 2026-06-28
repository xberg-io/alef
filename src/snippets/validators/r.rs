use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct RValidator;

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
        let mut source = NamedTempFile::with_suffix(".R")?;
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let path = source.path().to_string_lossy().to_string();

        let mut command = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => {
                let mut command = std::process::Command::new("Rscript");
                command.args(["-e", &format!("parse(file = '{path}')")]);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("Rscript");
                command.arg(&path);
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

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("could not find function")
            || output.contains("there is no package called")
            || output.contains("cannot open file")
    }
}
