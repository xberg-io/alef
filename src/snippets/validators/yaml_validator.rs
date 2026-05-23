use crate::snippets::error::Result;
use crate::snippets::types::{Language, Snippet, SnippetStatus, ValidationLevel};
use crate::snippets::validators::SnippetValidator;

pub struct YamlValidator;

impl SnippetValidator for YamlValidator {
    fn language(&self) -> Language {
        Language::Yaml
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
        match serde_yaml::from_str::<serde_yaml::Value>(&snippet.code) {
            Ok(_) => Ok((SnippetStatus::Pass, None)),
            Err(err) => Ok((SnippetStatus::Fail, Some(err.to_string()))),
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Syntax
    }
}
