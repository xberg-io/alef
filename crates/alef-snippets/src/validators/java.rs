use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use tempfile::TempDir;

pub struct JavaValidator;

impl JavaValidator {
    fn extract_class_name(code: &str) -> String {
        for line in code.lines() {
            let trimmed = line.trim();
            for keyword in ["public class ", "class ", "public final class ", "final class "] {
                if let Some(rest) = trimmed.strip_prefix(keyword) {
                    let name = rest
                        .split(|c: char| c.is_whitespace() || c == '{' || c == '<')
                        .next()
                        .unwrap_or("Snippet");
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
            }
        }
        "Snippet".to_string()
    }
}

impl SnippetValidator for JavaValidator {
    fn language(&self) -> Language {
        Language::Java
    }

    fn is_available(&self) -> bool {
        which::which("javac").is_ok()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let dir = TempDir::new()?;
        let class_name = Self::extract_class_name(&snippet.code);
        let file = dir.path().join(format!("{class_name}.java"));
        std::fs::write(&file, snippet.code.trim())?;

        let mut command = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile => {
                let mut command = std::process::Command::new("javac");
                command
                    .args(["-Xlint:none", "-nowarn", "-d"])
                    .arg(dir.path())
                    .arg(&file);
                command
            }
            ValidationLevel::Run => {
                let mut command = std::process::Command::new("java");
                command.arg(&file);
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
        ValidationLevel::Compile
    }

    fn is_dependency_error(&self, output: &str) -> bool {
        output.contains("cannot find symbol")
            || output.contains("package") && output.contains("does not exist")
    }
}
