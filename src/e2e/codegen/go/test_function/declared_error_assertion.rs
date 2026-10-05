//! The Go assertion for a fixture's declared `error` value.
//!
//! A value naming a variant is compared through the generated error's `ErrorType` field when it
//! has one; otherwise it falls back to the message-or-type-name check. Split out of
//! `test_function.rs`, which is over the 1000-line cap and may not grow.
//!
//! The Go error generator exports one `*Error` per error type, with a field for every
//! whitelisted introspection method (`gen_go_error_struct`). When the type has an `error_type`
//! method the binding records the variant's name in `ErrorType`, so identity is read off the
//! typed error instead of searching the message for the variant name. An error type without that
//! field keeps the message-or-type-name check. ~keep

use std::fmt::Write as FmtWrite;

use crate::codegen::naming::go_error_type_name;
use crate::core::ir::{ErrorDef, TypeRef};
use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, declared_variant, skip_line};
use crate::e2e::escape::go_string_literal;
use crate::e2e::fixture::Fixture;

/// Whether the generated Go error struct for `error` carries an `ErrorType` string field.
///
/// The field exists exactly when `error.methods` lists an `error_type` method, and its Go type is
/// `string` only for a string return (`typeref_to_go_type` maps everything else it does not know
/// to `string` as well, but a non-string return is not a variant name). ~keep
fn carries_error_type_field(error: &ErrorDef) -> bool {
    !error.binding_excluded
        && error
            .methods
            .iter()
            .any(|method| method.name == "error_type" && matches!(method.return_type, TypeRef::String))
}

/// Render the typed `ErrorType` comparison and return `true`, or return `false` and write
/// nothing when the fixture declares no variant this backend can substantiate or the error type
/// has no `ErrorType` field -- the caller then renders the message-or-type-name check.
///
/// `go_package` is the Go package name the binding is generated under, which
/// [`go_error_type_name`] needs to strip the stutter prefix exactly as the generator did. ~keep
fn emit_typed(out: &mut String, fixture: &Fixture, errors: &[ErrorDef], import_alias: &str, go_package: &str) -> bool {
    let DeclaredErrorAssertion::Assert(_) = classify("go", fixture, errors) else {
        return false;
    };
    let Some((error, variant)) = declared_variant(fixture, errors) else {
        return false;
    };
    if !carries_error_type_field(error) {
        return false;
    }
    let error_type = go_error_type_name(&error.name, go_package);
    let expected = go_string_literal(&variant.name);
    let _ = writeln!(out, "\tif err != nil {{");
    let _ = writeln!(out, "\t\tvar typedError *{import_alias}.{error_type}");
    let _ = writeln!(out, "\t\tif !errors.As(err, &typedError) {{");
    let _ = writeln!(
        out,
        "\t\t\tt.Errorf(\"expected a *{import_alias}.{error_type} for {}, got %T: %v\", err, err)",
        variant.name
    );
    let _ = writeln!(out, "\t\t}} else if typedError.ErrorType != {expected} {{");
    let _ = writeln!(
        out,
        "\t\t\tt.Errorf(\"expected ErrorType %s, got %q\", {expected}, typedError.ErrorType)"
    );
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}}");
    true
}

/// Emit the message-or-type-name check for a fixture's declared `error` assertion value — or,
/// when the declared value names a real error variant Go cannot substantiate, the registered
/// skip instead of an assertion that can never pass.
///
/// No-op when nothing is declared, leaving the `expects_error` branch's output unchanged from
/// before this check existed.
///
/// ~keep Mirrors the Rust/Python backends' disjunction (see
/// `crate::e2e::codegen::declared_error_value`): fixture authors name either a message
/// substring (config-validation fixtures) or a type-name prefix (API-error fixtures) in
/// the assertion's value, never both conventions at once. Checking `err.Error()` OR
/// `fmt.Sprintf("%T", err)` lets this single code path serve both. Whether Go can satisfy the
/// second convention for a given variant is decided once by `declared_error_variant::classify`
/// — Go dispatches to a variant-named error only when `#[alef(error_code = N)]` is declared.
pub(super) fn emit_declared_error_value_assertion(
    out: &mut String,
    fixture: &Fixture,
    errors: &[crate::core::ir::ErrorDef],
) {
    match classify("go", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => {}
        DeclaredErrorAssertion::Assert(declared) => {
            let expected = go_string_literal(declared);
            let _ = writeln!(out, "\tif err != nil {{");
            let _ = writeln!(
                out,
                "\t\tif !strings.Contains(err.Error(), {expected}) && !strings.Contains(fmt.Sprintf(\"%T\", err), \
                 {expected}) {{"
            );
            let _ = writeln!(
                out,
                "\t\t\tt.Errorf(\"expected error to match %s, got message=%q type=%T\", {expected}, err.Error(), \
                 err)"
            );
            let _ = writeln!(out, "\t\t}}");
            let _ = writeln!(out, "\t}}");
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            let _ = writeln!(out, "{}", skip_line("\t", "//", variant, &fixture.id, "go"));
        }
    }
}

/// Render the declared-value assertion for `fixture`: the typed `ErrorType` comparison when the
/// generated error carries one, the message-or-type-name check or registered skip otherwise.
pub(super) fn emit(out: &mut String, fixture: &Fixture, errors: &[ErrorDef], import_alias: &str, go_package: &str) {
    if !emit_typed(out, fixture, errors, import_alias, go_package) {
        emit_declared_error_value_assertion(out, fixture, errors);
    }
}

#[cfg(test)]
mod tests {
    use super::emit_typed as emit;
    use crate::core::ir::{ErrorDef, ErrorVariant, MethodDef, TypeRef};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_declaring(value: &str) -> Fixture {
        Fixture {
            id: "budget_enforced".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    fn method(name: &str, return_type: TypeRef) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            return_type,
            ..MethodDef::default()
        }
    }

    fn error_def(name: &str, error_code: Option<u32>, methods: Vec<MethodDef>) -> Vec<ErrorDef> {
        vec![ErrorDef {
            name: name.to_string(),
            rust_path: format!("samplelib::{name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: "BudgetExceeded".to_string(),
                error_code,
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }]
    }

    fn render(fixture: &Fixture, errors: &[ErrorDef]) -> Option<String> {
        let mut out = String::new();
        emit(&mut out, fixture, errors, "pkg", "samplelib").then_some(out)
    }

    #[test]
    fn a_named_variant_is_compared_through_the_pointer_typed_error_field() {
        let errors = error_def("SamplelibError", Some(100), vec![method("error_type", TypeRef::String)]);

        let out = render(&fixture_declaring("BudgetExceeded"), &errors).expect("typed assertion");

        assert!(out.contains("var typedError *pkg.Error\n"), "{out}");
        assert!(out.contains("errors.As(err, &typedError)"), "{out}");
        assert!(out.contains("typedError.ErrorType != `BudgetExceeded`"), "{out}");
        assert!(
            !out.contains("strings."),
            "identity must not fall back to a message search: {out}"
        );
    }

    #[test]
    fn an_error_type_without_the_field_keeps_the_message_check() {
        let errors = error_def(
            "SamplelibError",
            Some(100),
            vec![method("status_code", TypeRef::String)],
        );

        assert_eq!(render(&fixture_declaring("BudgetExceeded"), &errors), None);
    }

    #[test]
    fn a_non_string_error_type_method_is_not_a_variant_name() {
        let errors = error_def(
            "SamplelibError",
            Some(100),
            vec![method(
                "error_type",
                TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            )],
        );

        assert_eq!(render(&fixture_declaring("BudgetExceeded"), &errors), None);
    }

    /// A message-style value names no variant, so there is nothing to compare an `ErrorType`
    /// against and the caller's message-or-type-name check stays in force.
    #[test]
    fn a_message_style_value_is_left_to_the_message_check() {
        let errors = error_def("SamplelibError", Some(100), vec![method("error_type", TypeRef::String)]);

        assert_eq!(render(&fixture_declaring("budget was exceeded"), &errors), None);
    }

    /// An uncoded variant is a registered skip, not an assertion, and must stay that way.
    #[test]
    fn an_uncoded_variant_is_not_asserted() {
        let errors = error_def("SamplelibError", None, vec![method("error_type", TypeRef::String)]);

        assert_eq!(render(&fixture_declaring("BudgetExceeded"), &errors), None);
    }

    /// The stutter-stripped name must match the generator: `SamplelibError` in package
    /// `samplelib` is exported as `Error`, while an unrelated name is kept verbatim.
    #[test]
    fn the_error_type_name_follows_the_generators_prefix_stripping() {
        let methods = || vec![method("error_type", TypeRef::String)];
        let stripped = error_def("SamplelibError", Some(100), methods());
        let verbatim = error_def("ConversionError", Some(100), methods());

        let stripped_out = render(&fixture_declaring("BudgetExceeded"), &stripped).expect("typed assertion");
        let verbatim_out = render(&fixture_declaring("BudgetExceeded"), &verbatim).expect("typed assertion");

        assert!(stripped_out.contains("var typedError *pkg.Error\n"), "{stripped_out}");
        assert!(
            verbatim_out.contains("var typedError *pkg.ConversionError\n"),
            "{verbatim_out}"
        );
    }
}
