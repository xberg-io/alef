//! The properties an error fixture expects on the thrown JavaScript error.
//!
//! A failed call throws an error that carries `kind` (the variant name) and `statusCode` as own
//! properties: the NAPI shim in `binding.cjs` copies them onto the error, and the wasm error
//! builder sets them on the `Error` it throws. A declared variant is compared with `kind` and an
//! `error.status_code` assertion with `statusCode`, so a binding that throws the wrong variant or
//! status fails the test instead of passing on the message alone.

use heck::ToLowerCamelCase;

use crate::core::ir::ErrorDef;
use crate::e2e::fixture::Fixture;

/// Whether `lang` is one of the two backends that render through this module.
fn throws_structured_errors(lang: &str) -> bool {
    matches!(lang, "node" | "wasm")
}

/// The errors narrowed to the methods the thrown error carries, or none for a backend that
/// throws plain errors. The skip funnel takes this list, so it names exactly the assertions
/// [`collect`] does not turn into properties.
pub(super) fn reachable(errors: &[ErrorDef], lang: &str) -> Vec<ErrorDef> {
    if throws_structured_errors(lang) {
        crate::e2e::codegen::error_field_checks::reachable_errors(
            errors,
            crate::e2e::codegen::error_field_checks::STATUS_CODE,
        )
    } else {
        Vec::new()
    }
}

/// The `(property, JavaScript literal)` pairs the fixture expects, and the skip markers for the
/// assertions that cannot be compared.
pub(super) fn collect(fixture: &Fixture, errors: &[ErrorDef], lang: &str) -> (Vec<(String, String)>, String) {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, declared_variant};

    let mut properties = Vec::new();
    let mut markers = String::new();
    if !throws_structured_errors(lang) {
        return (properties, markers);
    }
    if let DeclaredErrorAssertion::Assert(declared) = classify(lang, fixture, errors)
        && declared_variant(fixture, errors).is_some()
        && let Ok(literal) = serde_json::to_string(declared)
    {
        properties.push(("kind".to_string(), literal));
    }
    let live = reachable(errors, lang);
    for (_, method, expected, assertion) in crate::e2e::codegen::error_field_checks::checks(fixture, &live) {
        match expected {
            Some(expected) => properties.push((method.name.to_lower_camel_case(), expected.to_string())),
            None => {
                markers.push_str(&crate::e2e::codegen::error_field_checks::unrenderable_marker(
                    "\t\t// ", lang, fixture, assertion,
                ));
                markers.push('\n');
            }
        }
    }
    (properties, markers)
}

/// The body of an object literal for `expect.objectContaining`, or an empty string.
pub(super) fn object_literal(properties: &[(String, String)]) -> String {
    properties
        .iter()
        .map(|(key, literal)| format!("{key}: {literal}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The properties as ` && error?.key === literal` conditions, or an empty string.
pub(super) fn conditions(properties: &[(String, String)]) -> String {
    properties
        .iter()
        .map(|(key, literal)| format!(" && error?.{key} === {literal}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::error_field_checks::test_fixtures::{status_code_errors, status_code_fixture};

    #[test]
    fn a_status_code_assertion_becomes_a_status_code_property() {
        let (properties, markers) = collect(&status_code_fixture(401), &status_code_errors(), "node");
        assert_eq!(properties, vec![("statusCode".to_string(), "401".to_string())]);
        assert_eq!(markers, "");
    }

    #[test]
    fn a_declared_variant_becomes_a_kind_property_in_node_and_wasm() {
        let mut fixture = status_code_fixture(401);
        fixture.assertions[0].value = Some(serde_json::json!("Authentication"));
        for lang in ["node", "wasm"] {
            let (properties, _) = collect(&fixture, &status_code_errors(), lang);
            assert_eq!(
                properties[0],
                ("kind".to_string(), "\"Authentication\"".to_string()),
                "{lang}"
            );
            assert_eq!(object_literal(&properties), "kind: \"Authentication\", statusCode: 401");
        }
    }

    #[test]
    fn a_backend_that_throws_plain_errors_gets_no_properties() {
        let (properties, markers) = collect(&status_code_fixture(401), &status_code_errors(), "typescript");
        assert!(properties.is_empty());
        assert_eq!(markers, "");
    }

    #[test]
    fn conditions_join_every_property() {
        let properties = vec![
            ("kind".to_string(), "\"X\"".to_string()),
            ("statusCode".to_string(), "5".to_string()),
        ];
        assert_eq!(
            conditions(&properties),
            " && error?.kind === \"X\" && error?.statusCode === 5"
        );
    }
}
