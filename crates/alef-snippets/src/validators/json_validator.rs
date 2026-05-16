use crate::error::Result;
use crate::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::validators::SnippetValidator;

pub struct JsonValidator;

impl SnippetValidator for JsonValidator {
    fn language(&self) -> Language {
        Language::Json
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        match serde_json::from_str::<serde_json::Value>(&snippet.code) {
            Ok(_) => Ok((SnippetStatus::Pass, None)),
            Err(err) => Ok((SnippetStatus::Fail, Some(err.to_string()))),
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Syntax
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
            path: PathBuf::from("test.json"),
            language: Language::Json,
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
        let v = JsonValidator;
        let (status, _) = v.validate(&snippet(r#"{"key": "value"}"#), ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Pass);
    }

    #[test]
    fn syntax_fail() {
        let v = JsonValidator;
        let (status, _) = v.validate(&snippet(r#"{"key": "value""#), ValidationLevel::Syntax, 30).unwrap();
        assert_eq!(status, SnippetStatus::Fail);
    }
}
