//! Dart error assertions for fixtures that expect the call to fail.
//!
//! The Dart bridge throws the generated error class of the declared error, one subclass per
//! variant, named `{Error}_{Variant}` by flutter_rust_bridge. A fixture that names a variant
//! asserts that class, and a fixture that asserts a field of the error reads it from the
//! caught error through the error's generated introspection method.

use crate::core::ir::ErrorDef;
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, declared_variant, skip_line};
use crate::e2e::codegen::error_field_reachability::resolvable_equals_error_field;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::values::escape_dart;

/// The variable that holds the thrown error when a fixture asserts a field of it.
pub(super) const CAUGHT_ERROR: &str = "caughtError";

/// The errors the Dart binding mirrors, with only the introspection methods it bridges.
///
/// An error excluded from the binding never reaches Dart as a class, so a fixture cannot assert
/// its variant or fields. This list is the single input to the matcher, the field checks and
/// the skip markers, so the three always agree on which assertions are rendered. ~keep
pub(super) fn live_errors(errors: &[ErrorDef]) -> Vec<ErrorDef> {
    errors
        .iter()
        .filter(|error| !error.binding_excluded)
        .cloned()
        .map(|mut error| {
            error.methods.retain(|method| !method.sanitized);
            error
        })
        .collect()
}

/// Build the `throwsA(...)` matcher expression for an `error`-asserting test.
///
/// A declared value that names a variant of a mirrored error asserts that variant's class. Any
/// other declared value is message text: the thrown value must contain it in its text or its
/// type name. With no declared value the matcher is `throwsA(anything)`, the plain "the call
/// must fail" check. A variant of an error the binding does not mirror cannot be asserted, so
/// the skip line lands in `out` above the statement that uses the matcher.
pub(super) fn error_matcher(out: &mut String, indent: &str, fixture: &Fixture, errors: &[ErrorDef]) -> String {
    let live = live_errors(errors);
    match classify("dart", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => "throwsA(anything)".to_string(),
        DeclaredErrorAssertion::Assert(declared) => match declared_variant(fixture, &live) {
            Some((error, variant)) => format!("throwsA(isA<{}_{}>())", error.name, variant.name),
            None if declared_variant(fixture, errors).is_some() => {
                let _ = writeln!(out, "{}", skip_line(indent, "//", declared, &fixture.id, "dart"));
                "throwsA(anything)".to_string()
            }
            None => {
                let escaped = escape_dart(declared);
                format!(
                    "throwsA(predicate((e) => e.toString().contains('{escaped}') || e.runtimeType.toString().contains('{escaped}')))"
                )
            }
        },
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{}", skip_line(indent, "//", variant, &fixture.id, "dart"));
            "throwsA(anything)".to_string()
        }
    }
}

/// A Dart literal for a scalar fixture value, or `None` for a value Dart cannot compare with
/// `equals` without a conversion.
fn scalar_literal(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) => Some(format!("'{}'", escape_dart(s))),
        _ => None,
    }
}

/// The statements that check each `error.<field>` equals assertion against the caught error.
///
/// Every assertion this renders is one `error_path_assertions` leaves unmarked for the same
/// `errors` list, so no assertion is rendered twice or dropped. A value the check cannot
/// express gets the shared skip marker in `out`. ~keep
pub(super) fn field_checks(out: &mut String, indent: &str, fixture: &Fixture, errors: &[ErrorDef]) -> Vec<String> {
    let live = live_errors(errors);
    let mut checks = Vec::new();
    for assertion in &fixture.assertions {
        let Some((error, method)) = resolvable_equals_error_field(assertion, &live) else {
            continue;
        };
        let Some(dart_method) = crate::backends::dart::gen_rust_crate::error_method_dart_name(error, &method.name)
        else {
            continue;
        };
        let literal = assertion.value.as_ref().and_then(scalar_literal);
        match literal {
            Some(literal) => checks.push(format!(
                "{indent}expect(await ({CAUGHT_ERROR} as {}).{dart_method}(), equals({literal}));",
                error.name
            )),
            None => {
                let field = assertion.field.as_deref().unwrap_or("<none>");
                let line = format!(
                    "{indent}// skipped: assertion type 'equals' has no accessor for error field {field} in this backend"
                );
                crate::e2e::codegen::fail_on_unsupported_assertion_type_markers(&line, "dart", &fixture.id);
                let _ = writeln!(out, "{line}");
            }
        }
    }
    checks
}
