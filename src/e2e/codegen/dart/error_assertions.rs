use super::values::escape_dart;
use crate::codegen::naming::public_field_name;
use crate::core::config::Language;
use crate::core::ir::{ErrorDef, ErrorVariant, FieldDef};
use crate::e2e::codegen::error_field_reachability::resolvable_equals_error_field;
use crate::e2e::fixture::{Assertion, Fixture};
use std::fmt::Write as FmtWrite;

pub(super) fn typed_matcher(fixture: &Fixture, errors: &[ErrorDef]) -> Option<String> {
    let (error, variant) = super::super::declared_error_variant::declared_variant(fixture, errors)?;
    if error.binding_excluded {
        return None;
    }
    let type_name = crate::backends::dart::naming::dart_error_variant_class(&error.name, &variant.name);
    let enum_value = crate::backends::dart::naming::dart_error_is_simple(error)
        .then(|| crate::backends::dart::naming::dart_simple_error_value(&error.name, &variant.name));
    let fields = fixture
        .assertions
        .iter()
        .filter_map(|assertion| {
            let (index, field) = resolved_field(assertion, variant)?;
            let property = if field.name.is_empty() || field.name.starts_with('_') {
                format!("field{index}")
            } else {
                public_field_name(Language::Dart, &field.name, None)
            };
            Some(crate::alef_context! {
                property => property,
                name => super::values::escape_dart(&field.name),
                value => super::assertions::dart_format_value(assertion.value.as_ref()?),
            })
        })
        .collect::<Vec<_>>();
    Some(
        crate::e2e::template_env::render(
            "dart/error_matcher.dart.jinja",
            crate::alef_context! {
                type_name => type_name, fields => fields, enum_value => enum_value,
            },
        )
        .trim_end()
        .to_string(),
    )
}

fn resolved_field<'a>(assertion: &Assertion, variant: &'a ErrorVariant) -> Option<(usize, &'a FieldDef)> {
    if assertion.assertion_type != "equals"
        || assertion.value.as_ref()?.is_array()
        || assertion.value.as_ref()?.is_object()
    {
        return None;
    }
    let name = assertion.field.as_deref()?.strip_prefix("error.")?;
    variant
        .fields
        .iter()
        .filter(|field| !field.binding_excluded)
        .enumerate()
        .find(|(index, field)| {
            field.name == name
                || ((field.name.is_empty() || field.name.starts_with('_')) && name == format!("field{index}"))
        })
}

pub(super) fn pending_assertions(fixture: &Fixture, errors: &[ErrorDef]) -> Fixture {
    let mut pending = fixture.clone();
    if let Some((error, variant)) = super::super::declared_error_variant::declared_variant(fixture, errors)
        && !error.binding_excluded
    {
        pending
            .assertions
            .retain(|assertion| resolved_field(assertion, variant).is_none());
    }
    pending
}

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

/// A Dart literal for a scalar fixture value, or `None` for a value Dart cannot compare with
/// `equals` without a conversion.
fn scalar_literal(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => Some("null".into()),
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
    let pending = pending_assertions(fixture, errors);
    for assertion in &pending.assertions {
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
