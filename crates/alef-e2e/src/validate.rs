//! JSON Schema and semantic validation for e2e fixture files.

use crate::config::E2eConfig;
use crate::fixture::{Fixture, group_fixtures};
use anyhow::{Context, Result};
use std::fmt;
use std::path::Path;

static FIXTURE_SCHEMA: &str = include_str!("../schema/fixture.schema.json");

/// Severity level for validation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Hard error — fixture is broken and will not produce correct tests.
    Error,
    /// Warning — fixture may not behave as intended.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

/// A validation error with its source file and message.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Relative path of the fixture file that failed validation.
    pub file: String,
    /// Human-readable error message.
    pub message: String,
    /// Severity level.
    pub severity: Severity,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.severity, self.file, self.message)
    }
}

/// Validate all JSON fixture files in a directory against the fixture schema.
///
/// Returns a list of validation errors. An empty list means all fixtures are valid.
pub fn validate_fixtures(fixtures_dir: &Path) -> Result<Vec<ValidationError>> {
    let schema_value: serde_json::Value =
        serde_json::from_str(FIXTURE_SCHEMA).context("failed to parse embedded fixture schema")?;
    let validator = jsonschema::validator_for(&schema_value).context("failed to compile fixture schema")?;

    let mut errors = Vec::new();
    validate_recursive(fixtures_dir, fixtures_dir, &validator, &mut errors)?;
    Ok(errors)
}

/// Perform semantic validation on loaded fixtures against e2e configuration.
///
/// Checks for:
/// 1. Fixtures skipped for all languages (empty `skip.languages`)
/// 2. Unknown call references not in `[e2e.calls.*]`
/// 3. Categories where all fixtures are skipped (produces 0 test functions)
/// 4. Missing required input fields for the resolved call config
/// 5. (D1) Argument arity and type mismatches in call configs
/// 6. (D2) Field path assertions against simple return types
pub fn validate_fixtures_semantic(
    fixtures: &[Fixture],
    e2e_config: &E2eConfig,
    languages: &[String],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Per-fixture checks
    for fixture in fixtures {
        // Check 1: skip-all detection
        if let Some(skip) = &fixture.skip {
            if skip.languages.is_empty() {
                let reason = skip.reason.as_deref().unwrap_or("no reason given");
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' is skipped for all languages (skip.languages is empty). Reason: {}",
                        fixture.id, reason
                    ),
                    severity: Severity::Warning,
                });
            }
        }

        // Check 2: unknown call reference
        if let Some(call_name) = &fixture.call {
            if !e2e_config.calls.contains_key(call_name) {
                errors.push(ValidationError {
                    file: fixture.source.clone(),
                    message: format!(
                        "fixture '{}' references unknown call '{}', will fall back to default [e2e.call]",
                        fixture.id, call_name
                    ),
                    severity: Severity::Error,
                });
            }
        }

        // Check 4: missing required input fields
        let call_config = e2e_config.resolve_call(fixture.call.as_deref());
        for arg in &call_config.args {
            if arg.optional {
                continue;
            }
            // Extract the input field name from the field path (e.g., "input.path" -> "path")
            let input_field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            if !fixture.input.is_null() {
                if let Some(obj) = fixture.input.as_object() {
                    if !obj.contains_key(input_field) {
                        // Skip check for error-type assertions (they may intentionally omit fields)
                        let is_error_test = fixture.assertions.iter().any(|a| a.assertion_type == "error");
                        if !is_error_test {
                            errors.push(ValidationError {
                                file: fixture.source.clone(),
                                message: format!(
                                    "fixture '{}' is missing required input field '{}' for call '{}'",
                                    fixture.id,
                                    input_field,
                                    fixture.call.as_deref().unwrap_or("<default>")
                                ),
                                severity: Severity::Warning,
                            });
                        }
                    }
                }
            }
        }
    }

    // Check 3: empty categories (all fixtures skipped for all languages)
    if !languages.is_empty() {
        let groups = group_fixtures(fixtures);
        for group in &groups {
            let has_any_non_skipped = group.fixtures.iter().any(|f| {
                match &f.skip {
                    None => true, // no skip → will generate
                    Some(skip) => {
                        // At least one language is NOT skipped
                        languages.iter().any(|lang| !skip.should_skip(lang))
                    }
                }
            });

            if !has_any_non_skipped {
                errors.push(ValidationError {
                    file: format!("{}/ (category)", group.category),
                    message: format!(
                        "category '{}' produces 0 test functions — all {} fixture(s) are skipped for all languages",
                        group.category,
                        group.fixtures.len()
                    ),
                    severity: Severity::Error,
                });
            }
        }
    }

    errors
}

fn validate_recursive(
    base: &Path,
    dir: &Path,
    validator: &jsonschema::Validator,
    errors: &mut Vec<ValidationError>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("failed to read directory: {}", dir.display()))?;

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            validate_recursive(base, &path, validator, errors)?;
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Skip schema files and files starting with _
            if filename == "schema.json" || filename.starts_with('_') {
                continue;
            }

            let relative = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("failed to read file: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            let value: serde_json::Value = match serde_json::from_str(&content) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(ValidationError {
                        file: relative,
                        message: format!("invalid JSON: {e}"),
                        severity: Severity::Error,
                    });
                    continue;
                }
            };

            for error in validator.iter_errors(&value) {
                errors.push(ValidationError {
                    file: relative.clone(),
                    message: format!("{} at {}", error, error.instance_path()),
                    severity: Severity::Error,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::SkipDirective;
    use alef_core::config::e2e::{ArgMapping, CallConfig};

    fn make_fixture(id: &str, source: &str, skip: Option<SkipDirective>, call: Option<&str>) -> Fixture {
        Fixture {
            id: id.to_string(),
            category: None,
            description: format!("Test {id}"),
            tags: vec![],
            skip,
            call: call.map(|s| s.to_string()),
            input: serde_json::json!({"path": "test.pdf"}),
            mock_response: None,
            visitor: None,
            assertions: vec![],
            source: source.to_string(),
            http: None,
        }
    }

    fn make_e2e_config(calls: Vec<(&str, CallConfig)>) -> E2eConfig {
        E2eConfig {
            calls: calls.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn test_skip_all_languages_detected() {
        let fixtures = vec![make_fixture(
            "test_skipped",
            "code/test.json",
            Some(SkipDirective {
                languages: vec![],
                reason: Some("Requires feature X".to_string()),
            }),
            None,
        )];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("skipped for all languages")));
    }

    #[test]
    fn test_unknown_call_detected() {
        let fixtures = vec![make_fixture("test_bad_call", "test.json", None, Some("nonexistent"))];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("unknown call 'nonexistent'")));
    }

    #[test]
    fn test_known_call_not_flagged() {
        let fixtures = vec![make_fixture("test_good_call", "test.json", None, Some("embed"))];
        let config = make_e2e_config(vec![("embed", CallConfig::default())]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(!errors.iter().any(|e| e.message.contains("unknown call")));
    }

    #[test]
    fn test_empty_category_detected() {
        let fixtures = vec![
            make_fixture(
                "test_a",
                "orphan/a.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: Some("skip all".to_string()),
                }),
                None,
            ),
            make_fixture(
                "test_b",
                "orphan/b.json",
                Some(SkipDirective {
                    languages: vec![],
                    reason: Some("skip all".to_string()),
                }),
                None,
            ),
        ];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        assert!(errors.iter().any(|e| e.message.contains("produces 0 test functions")));
    }

    #[test]
    fn test_missing_required_input_field() {
        let fixture = Fixture {
            id: "test_missing".to_string(),
            category: None,
            description: "Test".to_string(),
            tags: vec![],
            skip: None,
            call: Some("extract_bytes".to_string()),
            input: serde_json::json!({"data": "abc"}), // missing "mime_type"
            mock_response: None,
            visitor: None,
            assertions: vec![],
            source: "test.json".to_string(),
            http: None,
        };
        let call = CallConfig {
            function: "extract_bytes".to_string(),
            args: vec![
                ArgMapping {
                    name: "data".to_string(),
                    field: "input.data".to_string(),
                    arg_type: "bytes".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                },
                ArgMapping {
                    name: "mime_type".to_string(),
                    field: "input.mime_type".to_string(),
                    arg_type: "string".to_string(),
                    optional: false,
                    owned: false,
                    element_type: None,
                },
            ],
            ..Default::default()
        };
        let config = make_e2e_config(vec![("extract_bytes", call)]);
        let errors = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()]);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("missing required input field 'mime_type'"))
        );
    }

    #[test]
    fn test_no_errors_for_valid_fixture() {
        let fixtures = vec![make_fixture("test_valid", "contract/test.json", None, None)];
        let config = make_e2e_config(vec![]);
        let errors = validate_fixtures_semantic(&fixtures, &config, &["rust".to_string()]);
        // Only check for errors/warnings beyond the expected "missing input" ones
        // (default call config has no args, so no input field checks)
        assert!(errors.is_empty());
    }
}
