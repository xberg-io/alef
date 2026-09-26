//! Validation for `{{mock_*}}` origin-substitution placeholders and `[crates.e2e] alt_host`.
//!
//! Split out of `validate.rs` (already over the file-modularization cap) so this check has its
//! own home rather than growing the parent further -- see CLAUDE.md's `file-modularization`
//! rule. Detection of the tokens themselves lives in `crate::e2e::fixture::origin_tokens`,
//! which also drives `Fixture::has_host_root_route`'s dedicated-listener decision; this module
//! is the validation-time consumer of that same predicate.

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use crate::e2e::fixture::origin_tokens::{find_unknown_origin_token, known_tokens_display};
use crate::e2e::validate::{Severity, ValidationError};

/// Report a `mock_responses` entry whose `body_inline` or header value contains a
/// `{{mock_...}}`-shaped placeholder that is not one of the known origin-substitution tokens.
///
/// Without this check, a typo'd or made-up token (`{{mock_alt_orgin}}`, say) is served to the
/// client under test as literal, unresolved text instead of failing loudly at generation time.
pub(super) fn check_origin_tokens(fixture: &Fixture, errors: &mut Vec<ValidationError>) {
    let Some(entries) = fixture.input.get("mock_responses").and_then(|v| v.as_array()) else {
        return;
    };
    for entry in entries {
        if let Some(body) = entry.get("body_inline").and_then(|v| v.as_str())
            && let Some(bad_token) = find_unknown_origin_token(body)
        {
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' mock_responses body_inline contains unrecognized origin token \
                     '{bad_token}' -- expected one of: {}",
                    fixture.id,
                    known_tokens_display()
                ),
                severity: Severity::Error,
            });
        }
        let Some(headers) = entry.get("headers").and_then(|v| v.as_object()) else {
            continue;
        };
        for (header_name, header_value) in headers {
            let Some(value) = header_value.as_str() else {
                continue;
            };
            if let Some(bad_token) = find_unknown_origin_token(value) {
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' mock_responses header '{header_name}' contains unrecognized origin token \
                         '{bad_token}' -- expected one of: {}",
                        fixture.id,
                        known_tokens_display()
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }
}

/// Report `[crates.e2e] alt_host` when it is not a bare hostname.
///
/// `alt_host` is spliced verbatim into `http://<alt_host>:<port>` (`{{mock_alt_origin}}`) and
/// `http://sub.<alt_host>:<port>` (`{{mock_sub_origin}}`) by the generated mock server, so a
/// scheme, port, path, or stray whitespace here would silently produce a malformed URL that a
/// client under test then fails to parse for a reason that has nothing to do with the fixture
/// it is running.
pub(super) fn validate_alt_host(e2e_config: &E2eConfig, errors: &mut Vec<ValidationError>) {
    let host = e2e_config.alt_host.as_str();
    let is_bare_hostname = !host.is_empty()
        && !host.contains("://")
        && !host.contains('/')
        && !host.contains(':')
        && !host.chars().any(char::is_whitespace)
        && !host.starts_with('.')
        && !host.starts_with('-')
        && !host.ends_with('.')
        && !host.ends_with('-')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if is_bare_hostname {
        return;
    }
    errors.push(ValidationError {
        file: "alef.toml".to_string(),
        message: format!(
            "[crates.e2e] alt_host = '{host}' is not a valid bare hostname (no scheme, port, path, or \
             whitespace) -- it is used as-is in the mock_alt_origin and mock_sub_origin origin-token \
             substitutions, e.g. 'localhost' or a name registered in /etc/hosts"
        ),
        severity: Severity::Error,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::validate::Severity;

    fn fixture_from(json: &str) -> Fixture {
        serde_json::from_str(json).expect("fixture parses")
    }

    #[test]
    fn check_origin_tokens_accepts_known_token_in_body() {
        let fixture = fixture_from(
            r#"{
                "id": "known_body_token",
                "description": "Uses a real origin token",
                "input": {
                    "mock_responses": [
                        {"path": "/", "status_code": 200, "body_inline": "<a href=\"{{mock_alt_origin}}/x\">x</a>"}
                    ]
                },
                "assertions": []
            }"#,
        );
        let mut errors = Vec::new();
        check_origin_tokens(&fixture, &mut errors);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn check_origin_tokens_rejects_unknown_token_in_body() {
        let fixture = fixture_from(
            r#"{
                "id": "typo_body_token",
                "description": "Typos the alt origin token",
                "input": {
                    "mock_responses": [
                        {"path": "/", "status_code": 200, "body_inline": "<a href=\"{{mock_alt_orgin}}/x\">x</a>"}
                    ]
                },
                "assertions": []
            }"#,
        );
        let mut errors = Vec::new();
        check_origin_tokens(&fixture, &mut errors);
        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert!(errors[0].message.contains("mock_alt_orgin"), "{}", errors[0].message);
        assert!(errors[0].message.contains("typo_body_token"), "{}", errors[0].message);
    }

    #[test]
    fn check_origin_tokens_rejects_unknown_token_in_header() {
        let fixture = fixture_from(
            r#"{
                "id": "typo_header_token",
                "description": "Typos the sub origin token in a header",
                "input": {
                    "mock_responses": [
                        {
                            "path": "/",
                            "status_code": 302,
                            "headers": {"Location": "{{mock_subb_origin}}/x"},
                            "body_inline": ""
                        }
                    ]
                },
                "assertions": []
            }"#,
        );
        let mut errors = Vec::new();
        check_origin_tokens(&fixture, &mut errors);
        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(errors[0].message.contains("Location"), "{}", errors[0].message);
    }

    #[test]
    fn check_origin_tokens_ignores_fixtures_without_mock_responses() {
        let fixture = fixture_from(
            r#"{"id": "no_mock_responses", "description": "No route array", "input": {}, "assertions": []}"#,
        );
        let mut errors = Vec::new();
        check_origin_tokens(&fixture, &mut errors);
        assert!(errors.is_empty());
    }

    #[test]
    fn validate_alt_host_accepts_default() {
        let config = E2eConfig::default();
        let mut errors = Vec::new();
        validate_alt_host(&config, &mut errors);
        assert!(errors.is_empty(), "default alt_host must validate, got: {errors:?}");
    }

    #[test]
    fn validate_alt_host_accepts_dotted_hostname() {
        let config = E2eConfig {
            alt_host: "alt.test".to_string(),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_alt_host(&config, &mut errors);
        assert!(errors.is_empty(), "dotted hostname must validate, got: {errors:?}");
    }

    #[test]
    fn validate_alt_host_rejects_scheme() {
        let config = E2eConfig {
            alt_host: "http://localhost".to_string(),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_alt_host(&config, &mut errors);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("alt_host"));
    }

    #[test]
    fn validate_alt_host_rejects_port() {
        let config = E2eConfig {
            alt_host: "localhost:8080".to_string(),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_alt_host(&config, &mut errors);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn validate_alt_host_rejects_empty() {
        let config = E2eConfig {
            alt_host: String::new(),
            ..Default::default()
        };
        let mut errors = Vec::new();
        validate_alt_host(&config, &mut errors);
        assert_eq!(errors.len(), 1);
    }
}
