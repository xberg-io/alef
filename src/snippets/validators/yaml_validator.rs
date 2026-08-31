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
        // `IgnoredAny` still drives a full parse, and unlike `serde_json::Value` it does not
        // reject non-string keys — legal YAML, plausible in hand-written snippets. ~keep
        let mut options = serde_saphyr::Options::default();
        // `.nan` / `.inf` are valid scalars; serde_yaml passed them.
        options.reject_non_finite_typeless_float = false;
        match serde_saphyr::from_str_with_options::<serde::de::IgnoredAny>(&snippet.code, options) {
            Ok(_) => Ok((SnippetStatus::Pass, None)),
            Err(err) => Ok((SnippetStatus::Fail, Some(err.to_string()))),
        }
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Syntax
    }
}

/// ~keep Pins serde-saphyr behaviour, not ours: that `IgnoredAny` still parses in full, and
/// that an empty snippet stays a `Pass`. Both can regress on a dependency bump.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::snippets::types::{SnippetMetadata, SourceOrigin};

    fn yaml_snippet(code: &str) -> Snippet {
        Snippet {
            id: None,
            path: "example.md".into(),
            language: Language::Yaml,
            title: None,
            code: code.into(),
            start_line: 1,
            block_index: 0,
            annotation: None,
            metadata: SnippetMetadata::default(),
            source_origin: SourceOrigin {
                path: "example.md".into(),
                line: 1,
                block_index: 0,
            },
        }
    }

    fn status(code: &str) -> SnippetStatus {
        YamlValidator
            .validate(&yaml_snippet(code), ValidationLevel::Syntax, 10)
            .expect("the YAML validator never fails to run")
            .0
    }

    #[test]
    fn malformed_yaml_still_fails_even_though_the_parsed_value_is_discarded() {
        for broken in [
            "a: [1, 2",            // unclosed flow sequence
            "a: {b: 1",            // unclosed flow mapping
            "a: 1\n  b: 2\n",      // indentation that opens no block
            "a:\n\t- 1\n",         // tab used as block indentation
            "a: \"unterminated\n", // quoted scalar with no closing quote
            "a: 1\nb\n",           // trailing junk after a valid mapping
            "a: *nonexistent\n",   // alias with no matching anchor
            "jobs:\n  build:\n    steps: [ {uses: x}\n",
        ] {
            assert_eq!(
                status(broken),
                SnippetStatus::Fail,
                "malformed YAML must not pass validation: {broken:?}"
            );
        }
    }

    #[test]
    fn valid_yaml_passes_including_shapes_a_json_shaped_target_would_reject() {
        for valid in [
            "",                   // empty snippet
            "# just a comment\n", // comment-only snippet
            "~",                  // explicit null document
            "a: 1\nb: [x, y]\n",  // ordinary mapping
            "1: one\n",           // non-string mapping key
            "a: !mytag hello\n",  // application-specific tag
            "a: .nan\nb: .inf\n", // non-finite floats
        ] {
            assert_eq!(
                status(valid),
                SnippetStatus::Pass,
                "valid YAML must pass validation: {valid:?}"
            );
        }
    }
}
