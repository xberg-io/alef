//! Validation for `[crates.e2e] wasm_config_overrides` field paths.

use super::{Severity, ValidationError};
use crate::e2e::config::E2eConfig;

/// Report a `wasm_config_overrides` key that is empty or contains an empty dotted-path segment.
///
/// `apply_wasm_config_overrides` (the typescript/wasm e2e generator) walks a key like
/// `"ssrf.deny_private"` by splitting on `.` and creating any missing intermediate object --  it
/// has no way to tell a typo from an intentional field literally named `""`, so `""`, `"a."`,
/// `".a"`, and `"a..b"` would all silently set some field named the empty string instead of doing
/// nothing. Catching this at validation time surfaces the mistake immediately, rather than only
/// after reading generated wasm setup code that mysteriously never overrides the intended field.
pub(super) fn validate_wasm_config_overrides(e2e_config: &E2eConfig, errors: &mut Vec<ValidationError>) {
    for path in e2e_config.wasm_config_overrides.keys() {
        if path.is_empty() || path.split('.').any(str::is_empty) {
            errors.push(ValidationError {
                file: "alef.toml".to_string(),
                message: format!(
                    "[crates.e2e] wasm_config_overrides key '{path}' is not a valid dotted field \
                     path -- it must be non-empty with no empty segment (e.g. 'ssrf.deny_private')"
                ),
                severity: Severity::Error,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::validate::Severity;

    fn config_with_override(key: &str) -> E2eConfig {
        E2eConfig {
            wasm_config_overrides: [(key.to_string(), toml::Value::Boolean(false))].into_iter().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn accepts_a_well_formed_dotted_path() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override("ssrf.deny_private"), &mut errors);
        assert!(
            errors.is_empty(),
            "a well-formed dotted path must validate, got: {errors:?}"
        );
    }

    #[test]
    fn accepts_a_single_segment_path() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override("enabled"), &mut errors);
        assert!(
            errors.is_empty(),
            "a single-segment path must validate, got: {errors:?}"
        );
    }

    #[test]
    fn rejects_an_empty_key() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override(""), &mut errors);
        assert_eq!(errors.len(), 1, "an empty key must be rejected, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
    }

    #[test]
    fn rejects_a_leading_empty_segment() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override(".deny_private"), &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "a leading empty segment must be rejected, got: {errors:?}"
        );
    }

    #[test]
    fn rejects_a_trailing_empty_segment() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override("ssrf."), &mut errors);
        assert_eq!(
            errors.len(),
            1,
            "a trailing empty segment must be rejected, got: {errors:?}"
        );
    }

    #[test]
    fn rejects_a_doubled_separator() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&config_with_override("ssrf..deny_private"), &mut errors);
        assert_eq!(errors.len(), 1, "a doubled separator must be rejected, got: {errors:?}");
    }

    #[test]
    fn default_config_has_no_overrides_and_validates_clean() {
        let mut errors = Vec::new();
        validate_wasm_config_overrides(&E2eConfig::default(), &mut errors);
        assert!(
            errors.is_empty(),
            "the default (empty) config must validate clean, got: {errors:?}"
        );
    }
}
