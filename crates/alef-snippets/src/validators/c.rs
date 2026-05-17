use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::{SnippetValidator, run_command};
use std::io::Write;
use tempfile::NamedTempFile;

pub struct CValidator;

fn compiler() -> Option<String> {
    for candidate in ["cc", "clang", "gcc"] {
        if which::which(candidate).is_ok() {
            return Some(candidate.to_string());
        }
    }
    None
}

impl SnippetValidator for CValidator {
    fn language(&self) -> Language {
        Language::C
    }

    fn is_available(&self) -> bool {
        compiler().is_some()
    }

    fn validate(
        &self,
        snippet: &Snippet,
        level: ValidationLevel,
        timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        let Some(cc) = compiler() else {
            return Ok((SnippetStatus::Unavailable, Some("no C compiler on PATH".into())));
        };

        let mut source = NamedTempFile::with_suffix(".c")?;
        source.write_all(snippet.code.as_bytes())?;
        source.flush()?;
        let source_path = source.path().to_string_lossy().to_string();

        let mut command = std::process::Command::new(&cc);
        match level {
            ValidationLevel::Syntax => {
                command.args(["-fsyntax-only", &source_path]);
            }
            ValidationLevel::Compile | ValidationLevel::Run => {
                let out = NamedTempFile::new()?;
                let out_path = out.path().to_string_lossy().to_string();
                drop(out);
                command.args(["-o", &out_path, &source_path]);
                let (success, output) = run_command(&mut command, timeout_secs)?;
                if !success {
                    return Ok((SnippetStatus::Fail, Some(output)));
                }
                if matches!(level, ValidationLevel::Run) {
                    let mut run = std::process::Command::new(&out_path);
                    let (ran_ok, run_output) = run_command(&mut run, timeout_secs)?;
                    let _ = std::fs::remove_file(&out_path);
                    return Ok(if ran_ok {
                        (SnippetStatus::Pass, None)
                    } else {
                        (SnippetStatus::Fail, Some(run_output))
                    });
                }
                let _ = std::fs::remove_file(&out_path);
                return Ok((SnippetStatus::Pass, None));
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
        output.contains("file not found")
            || output.contains("No such file or directory")
            || output.contains("undeclared identifier")
            || output.contains("implicit declaration")
            || output.contains("unknown type name")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Snippet, SnippetMetadata};
    use std::path::PathBuf;

    fn snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: PathBuf::from("test.c"),
            language: Language::C,
            title: None,
            code: code.to_string(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
        }
    }

    #[test]
    fn syntax_ok() {
        let v = CValidator;
        if !v.is_available() {
            return;
        }
        let s = snippet("int main(void) { return 0; }\n");
        let (status, _) = v.validate(&s, ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Pass);
    }

    #[test]
    fn syntax_fail() {
        let v = CValidator;
        if !v.is_available() {
            return;
        }
        let s = snippet("int main(void) { @@@ }\n");
        let (status, _) = v.validate(&s, ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Fail);
    }
}
