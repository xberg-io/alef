use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::{SnippetValidator, run_command};
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

    fn has_class_or_interface(code: &str) -> bool {
        code.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("class ")
                || trimmed.starts_with("public class ")
                || trimmed.starts_with("final class ")
                || trimmed.starts_with("public final class ")
                || trimmed.starts_with("interface ")
                || trimmed.starts_with("public interface ")
                || trimmed.starts_with("public enum ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("public record ")
                || trimmed.starts_with("record ")
        })
    }

    fn split_imports(code: &str) -> (String, String) {
        let mut imports = Vec::new();
        let mut body = Vec::new();
        let mut past_imports = false;
        for line in code.lines() {
            let trimmed = line.trim();
            if !past_imports
                && (trimmed.is_empty() || trimmed.starts_with("import ") || trimmed.starts_with("package "))
            {
                imports.push(line);
            } else {
                past_imports = true;
                body.push(line);
            }
        }
        (imports.join("\n"), body.join("\n"))
    }

    fn wrap_if_fragment(code: &str) -> String {
        let trimmed = code.trim();
        if Self::has_class_or_interface(trimmed) {
            return code.to_string();
        }

        let (imports, body) = Self::split_imports(trimmed);
        let body_trimmed = body.trim();
        let only_comments = !body_trimmed.is_empty()
            && body_trimmed
                .lines()
                .all(|line| line.trim().is_empty() || line.trim().starts_with("//"));

        let body_inner = if body_trimmed.is_empty() || only_comments {
            format!("{body_trimmed}\nint _placeholder = 0;")
        } else {
            body.to_string()
        };
        let imports_block = if imports.trim().is_empty() {
            String::new()
        } else {
            format!("{imports}\n\n")
        };
        format!(
            "{imports_block}public class Snippet {{\n    public static void main(String[] args) throws Exception {{\n{body_inner}\n    }}\n}}\n"
        )
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
        let wrapped = Self::wrap_if_fragment(&snippet.code);
        let class_name = Self::extract_class_name(&wrapped);
        let file = dir.path().join(format!("{class_name}.java"));
        std::fs::write(&file, &wrapped)?;

        let mut command = match level {
            ValidationLevel::Syntax | ValidationLevel::Compile | ValidationLevel::TypeCheck => {
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
        output.contains("cannot find symbol") || output.contains("package") && output.contains("does not exist")
    }
}
