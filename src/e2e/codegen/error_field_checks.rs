//! The shared front half of every backend's `error.<field>` equals check.
//!
//! A fixture can assert `error.status_code` against the failed call. The crate lists the
//! error introspection methods a binding carries in `ErrorDef.methods`, and each backend
//! decides which of them its own error value exposes. This module keeps the three steps that
//! do not depend on the language in one place:
//!
//! 1. [`reachable_errors`] narrows the crate's errors to the methods one backend can read, so
//!    the skip funnel (`error_path_assertions`) and the renderer agree on which assertions are
//!    rendered. An assertion the renderer takes is never also marked as skipped. ~keep
//! 2. [`render`] walks the fixture's `equals` assertions on those methods and asks the backend
//!    for one statement per assertion.
//! 3. A value the check cannot express (anything but a whole number) keeps the shared skip
//!    marker, so it is never dropped without a trace.

use std::fmt::Write as FmtWrite;

use crate::core::ir::{ErrorDef, MethodDef};
use crate::e2e::fixture::{Assertion, Fixture};

/// The one introspection method every backend reads off its thrown error. A backend that reads
/// more adds its own list.
pub(crate) const STATUS_CODE: &[&str] = &["status_code"];

/// The crate's visible errors, each narrowed to the introspection methods in `supported`.
///
/// `supported` holds the Rust method names a backend can read off its thrown error, for example
/// `["status_code"]`. A method the binding sanitized away is dropped too, because the binding
/// does not carry it.
pub(crate) fn reachable_errors(errors: &[ErrorDef], supported: &[&str]) -> Vec<ErrorDef> {
    crate::codegen::binding_projection::visible_errors(errors)
        .cloned()
        .map(|mut error| {
            error
                .methods
                .retain(|method| !method.sanitized && supported.contains(&method.name.as_str()));
            error
        })
        .collect()
}

/// The `equals` assertions on a reachable `error.<method>` field, in fixture order.
///
/// Each item pairs the error type and method that answer the assertion with the expected whole
/// number, or `None` when the value is not a whole number and no backend can compare it.
pub(crate) fn checks<'a>(
    fixture: &'a Fixture,
    reachable: &'a [ErrorDef],
) -> Vec<(&'a ErrorDef, &'a MethodDef, Option<i64>, &'a Assertion)> {
    fixture
        .assertions
        .iter()
        .filter_map(|assertion| {
            let (error, method) =
                crate::e2e::codegen::error_field_reachability::resolvable_equals_error_field(assertion, reachable)?;
            let expected = assertion.value.as_ref().and_then(serde_json::Value::as_i64);
            Some((error, method, expected, assertion))
        })
        .collect()
}

/// The shared skip marker for a check whose value is not a whole number.
pub(crate) fn unrenderable_marker(
    marker_prefix: &str,
    language: &str,
    fixture: &Fixture,
    assertion: &Assertion,
) -> String {
    let field = assertion.field.as_deref().unwrap_or("<none>");
    let line = format!(
        "{marker_prefix}skipped: assertion type 'equals' has no accessor for error field {field} in this backend"
    );
    crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&line, language, &fixture.id);
    line
}

/// One statement for every `equals` assertion on a reachable `error.<method>` field.
///
/// `statement` receives the error type, the Rust method name and the expected whole number, and
/// returns the finished statement text, indentation and trailing newline included. Assertions
/// with a value that is not a whole number get the shared skip marker behind `marker_prefix`
/// instead.
pub(crate) fn render(
    fixture: &Fixture,
    reachable: &[ErrorDef],
    marker_prefix: &str,
    language: &str,
    statement: impl Fn(&ErrorDef, &str, i64) -> String,
) -> String {
    let mut out = String::new();
    for (error, method, expected, assertion) in checks(fixture, reachable) {
        match expected {
            Some(expected) => out.push_str(&statement(error, &method.name, expected)),
            None => {
                let _ = writeln!(
                    out,
                    "{}",
                    unrenderable_marker(marker_prefix, language, fixture, assertion)
                );
            }
        }
    }
    out
}

/// Fixtures shared by the backend tests that cover the rendered `error.status_code` check.
#[cfg(test)]
pub(crate) mod test_fixtures {
    use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, PrimitiveType, TypeRef};
    use crate::e2e::fixture::{Assertion, Fixture};

    /// An error type named `name` whose introspection methods are `methods`.
    pub(crate) fn error_with_methods(name: &str, methods: &[&str]) -> Vec<ErrorDef> {
        vec![ErrorDef {
            exception_base: None,
            name: name.to_string(),
            rust_path: format!("lib::{name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "Authentication".to_string(),
                error_code: Some(100),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: methods
                .iter()
                .map(|method| MethodDef {
                    name: (*method).to_string(),
                    return_type: TypeRef::Primitive(PrimitiveType::U16),
                    ..MethodDef::default()
                })
                .collect(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    /// A crate error type that lists `status_code`, as the liter-llm error does.
    pub(crate) fn status_code_errors() -> Vec<ErrorDef> {
        error_with_methods("ApiError", &["status_code"])
    }

    /// An error fixture that expects a failed call and asserts `error.status_code == status`.
    pub(crate) fn status_code_fixture(status: i64) -> Fixture {
        Fixture {
            id: "error_auth_401".to_string(),
            assertions: vec![
                Assertion {
                    assertion_type: "error".to_string(),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("error.status_code".to_string()),
                    value: Some(serde_json::json!(status)),
                    ..Assertion::default()
                },
            ],
            ..Fixture::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ErrorVariant, MethodDef, TypeRef};
    use crate::e2e::fixture::Assertion;

    fn error_with(methods: &[&str]) -> Vec<ErrorDef> {
        vec![ErrorDef {
            exception_base: None,
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant::default()],
            doc: String::new(),
            methods: methods
                .iter()
                .map(|name| MethodDef {
                    name: (*name).to_string(),
                    return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
                    ..MethodDef::default()
                })
                .collect(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    fn equals(field: &str, value: serde_json::Value) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(value),
            ..Assertion::default()
        }
    }

    fn fixture(assertions: Vec<Assertion>) -> Fixture {
        Fixture {
            id: "rate_limited".to_string(),
            assertions,
            ..Fixture::default()
        }
    }

    #[test]
    fn reachable_errors_keeps_only_the_supported_methods() {
        let narrowed = reachable_errors(&error_with(&["status_code", "is_transient"]), &["status_code"]);
        let names: Vec<_> = narrowed[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["status_code"]);
    }

    #[test]
    fn a_whole_number_value_is_rendered_by_the_backend_statement() {
        let reachable = reachable_errors(&error_with(&["status_code"]), &["status_code"]);
        let fixture = fixture(vec![equals("error.status_code", serde_json::json!(429))]);
        let out = render(&fixture, &reachable, "// ", "go", |_, method, value| {
            format!("check {method} == {value}\n")
        });
        assert_eq!(out, "check status_code == 429\n");
    }

    #[test]
    fn a_method_the_backend_does_not_support_renders_nothing() {
        let reachable = reachable_errors(&error_with(&["status_code", "is_transient"]), &["status_code"]);
        let fixture = fixture(vec![equals("error.is_transient", serde_json::json!(true))]);
        let out = render(&fixture, &reachable, "// ", "go", |_, _, _| "unexpected\n".to_string());
        assert_eq!(out, "");
    }

    #[test]
    fn a_value_that_is_not_a_whole_number_keeps_the_skip_marker() {
        let reachable = reachable_errors(&error_with(&["status_code"]), &["status_code"]);
        let fixture = fixture(vec![equals("error.status_code", serde_json::json!("429"))]);
        let _ = crate::e2e::codegen::take_skip_records();
        let out = render(&fixture, &reachable, "// ", "go", |_, _, _| "unexpected\n".to_string());
        assert_eq!(
            out,
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend\n"
        );
        assert_eq!(crate::e2e::codegen::take_skip_records().len(), 1);
    }
}
