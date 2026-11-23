//! Which typed exception classes a rendered test file's error assertions reference.
//!
//! `pyo3::create_exception!` gives every `ErrorVariant` its own exception class unconditionally
//! (`declared_error_variant::substantiates_variant_identity`'s `"python" => true` arm), so
//! `error_assertions::emit_error_assertion` catches that class directly —
//! `pytest.raises(BadRequestError)` — for a fixture whose declared `error` value names a real
//! variant, rather than the broad `Exception`. That class name must be imported
//! into the generated file exactly like any other type it references.

use std::collections::BTreeSet;

use crate::e2e::fixture::Fixture;

/// The base exception class `pytest.raises(...)` names for a fixture that declares an error
/// without any value to discriminate on: the one `ErrorDef`'s own class, which every variant
/// class derives from. `None` when the crate has no error type or several (no way to know which
/// one the call raises), where the caller falls back to a bound `exc_info`.
pub(crate) fn sole_base_error_type(errors: &[crate::core::ir::ErrorDef]) -> Option<String> {
    let mut visible = crate::codegen::binding_projection::visible_errors(errors);
    let first = visible.next()?;
    visible.next().is_none().then(|| first.name.clone())
}

/// The typed exception class names `emit_error_assertion` will reference in `pytest.raises(...)`
/// for any fixture in `fixtures` whose declared `error` value names a real `ErrorVariant`.
///
/// HTTP fixtures route through `render_http_test_function`, not `emit_error_assertion`, so they
/// are excluded here to match which fixtures can actually emit that class name.
pub(crate) fn collect_used_error_types(
    fixtures: &[&Fixture],
    errors: &[crate::core::ir::ErrorDef],
) -> BTreeSet<String> {
    let mut used_error_types = BTreeSet::new();
    for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
        if let Some(branch) = crate::e2e::codegen::snippet_error_branch::for_fixture("python", fixture, errors) {
            used_error_types.insert(branch.host_type);
        }
    }
    if fixtures
        .iter()
        .any(|f| !f.is_http_test() && super::error_assertions::renders_bare_error_catch(f, errors))
        && let Some(base) = sole_base_error_type(errors)
    {
        used_error_types.insert(base);
    }
    used_error_types
}

#[cfg(test)]
mod tests {
    use super::{collect_used_error_types, sole_base_error_type};
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn error_def_with_variant(error_name: &str, variant_name: &str) -> ErrorDef {
        ErrorDef {
            exception_base: None,
            name: error_name.to_string(),
            rust_path: format!("lib::{error_name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn fixture_declaring(value: &str) -> Fixture {
        Fixture {
            id: "auth_fails".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    /// The regression this module exists to close: before it, nothing collected the class name
    /// `emit_error_assertion` now renders, so `pytest.raises(AuthenticationError)` resolved
    /// against no import at all — a `NameError`, not a passing test.
    #[test]
    fn a_fixture_naming_a_real_variant_yields_its_exception_class() {
        let fixture = fixture_declaring("Authentication");
        let errors = vec![error_def_with_variant("ApiError", "Authentication")];
        let fixtures = vec![&fixture];
        assert_eq!(
            collect_used_error_types(&fixtures, &errors),
            ["AuthenticationError".to_string()].into_iter().collect()
        );
    }

    /// A message-style declared value (not a known variant name) names no class — must not
    /// spuriously import anything.
    #[test]
    fn a_message_style_value_yields_nothing() {
        let fixture = fixture_declaring("size must be positive");
        let errors = vec![error_def_with_variant("ApiError", "Authentication")];
        let fixtures = vec![&fixture];
        assert!(collect_used_error_types(&fixtures, &errors).is_empty());
    }

    #[test]
    fn a_single_error_type_yields_its_base_class_for_bare_error_fixtures() {
        let errors = vec![error_def_with_variant("CrawlError", "Timeout")];
        assert_eq!(sole_base_error_type(&errors).as_deref(), Some("CrawlError"));
        let mut fixture = fixture_declaring("unused");
        fixture.assertions[0].value = None;
        let fixtures = vec![&fixture];
        assert!(collect_used_error_types(&fixtures, &errors).contains("CrawlError"));
        let message_fixture = fixture_declaring("size must be positive");
        let message_fixtures = vec![&message_fixture];
        assert!(collect_used_error_types(&message_fixtures, &errors).is_empty());
    }

    #[test]
    fn several_error_types_yield_no_base_class() {
        let errors = vec![
            error_def_with_variant("CrawlError", "Timeout"),
            error_def_with_variant("ConfigError", "Invalid"),
        ];
        assert_eq!(sole_base_error_type(&errors), None);
    }
}
