//! Whether a fixture's `error.<field>` assertion names one of the crate's whitelisted error
//! introspection methods (`ErrorDef.methods` — e.g. `status_code`, `is_transient`,
//! `error_type`), and which declared error type answers it.
//!
//! ~keep This is a DIFFERENT reachability fact from `FieldResolver::accessor_for_error` /
//! `[e2e.error_field_aliases]`. That mechanism only ever answers "what does the *Rust* error
//! type call this field" and is documented as mapping to the Rust struct alone (see
//! `error_path_assertions`'s module doc). It cannot generalize to a non-`rust` backend, because
//! most bindings never re-expose the Rust error struct's own fields — they collapse a failure to
//! a numeric FFI-taxonomy code plus a message (`{prefix}_last_error_code` /
//! `{prefix}_last_error_context`, or `.to_string()` for the native-extension backends' generic
//! catch-all).
//!
//! `ErrorDef.methods` is the one place in the IR where a SPECIFIC field is proven reachable on a
//! non-Rust generated error type. Python reads it from the `{Error}Info` companion pyclass that
//! `gen_pyo3_error_converter` fills from the live Rust error value. The C, Go, Java, Elixir, PHP,
//! Node, WASM, Zig, Ruby, C# and Swift backends read `status_code` off the failed call the same
//! way, and each lists the methods it can read in its own call to
//! `error_field_checks::reachable_errors`.
//!
//! This module answers ONLY "does the IR say the field is whitelisted", not "does this specific
//! backend's error-conversion path actually populate it with live data" — that second judgment is
//! a per-backend fact the caller is responsible for.

use crate::core::ir::{ErrorDef, MethodDef};
use crate::e2e::fixture::Assertion;

/// The first declared error type (in source order) whose `methods` list contains a whitelisted
/// introspection method named `sub_field`, paired with that method's definition.
///
/// First-match-wins across multiple `ErrorDef`s in the same crate, mirroring the permissiveness
/// of `[e2e.error_field_aliases]` (a single flat map, not keyed per error type): a fixture names a
/// field, not the Rust error enum that carries it.
pub(crate) fn introspection_method<'a>(
    errors: &'a [ErrorDef],
    sub_field: &str,
) -> Option<(&'a ErrorDef, &'a MethodDef)> {
    crate::codegen::binding_projection::visible_errors(errors)
        .find_map(|error| error.methods.iter().find(|m| m.name == sub_field).map(|m| (error, m)))
}

/// Whether `assertion` is an `equals` check against an `error.<field>` path that resolves to a
/// whitelisted introspection method — the one assertion shape a caller actually
/// renders through this reachability fact.
///
/// ~keep Centralizing the "equals + error.<field> + resolves" test here, rather than letting the
/// funnel (`error_path_assertions::render_with_errors`) and the renderer
/// (`python::test_function::error_assertions::emit_error_assertion`) each re-derive it, is the
/// point: the funnel must suppress its skip marker for EXACTLY the assertions the renderer
/// actually renders, or a fixture's assertion goes silently missing (rendered by neither) — the
/// exact defect `error_path_assertions`'s own module doc describes as indistinguishable from a
/// fixture that never declared an assertion at all. A future backend that renders a different
/// assertion shape (e.g. `greater_than`) extends this function, not a private copy of it.
pub(crate) fn resolvable_equals_error_field<'a>(
    assertion: &Assertion,
    errors: &'a [ErrorDef],
) -> Option<(&'a ErrorDef, &'a MethodDef)> {
    if assertion.assertion_type != "equals" {
        return None;
    }
    let sub_field = assertion.field.as_deref()?.strip_prefix("error.")?;
    introspection_method(errors, sub_field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ErrorVariant, ReceiverKind, TypeRef};

    fn method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: Vec::new(),
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U16),
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn error_with_methods(name: &str, methods: Vec<&str>) -> ErrorDef {
        ErrorDef {
            exception_base: None,
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant::default()],
            doc: String::new(),
            methods: methods.into_iter().map(method).collect(),
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn equals_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!(429)),
            ..Assertion::default()
        }
    }

    #[test]
    fn introspection_method_finds_a_whitelisted_method() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code", "is_transient"])];
        let (found_error, found_method) = introspection_method(&errors, "status_code").expect("must resolve");
        assert_eq!(found_error.name, "SampleError");
        assert_eq!(found_method.name, "status_code");
    }

    #[test]
    fn introspection_method_returns_none_for_an_unwhitelisted_field() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        assert!(introspection_method(&errors, "retry_after").is_none());
    }

    #[test]
    fn introspection_method_returns_none_when_no_errors_declared() {
        assert!(introspection_method(&[], "status_code").is_none());
    }

    #[test]
    fn introspection_method_first_match_wins_across_multiple_error_types() {
        let errors = vec![
            error_with_methods("FirstError", vec!["status_code"]),
            error_with_methods("SecondError", vec!["status_code"]),
        ];
        let (found_error, _) = introspection_method(&errors, "status_code").expect("must resolve");
        assert_eq!(
            found_error.name, "FirstError",
            "must prefer the first declared error type"
        );
    }

    #[test]
    fn resolvable_equals_error_field_matches_equals_on_a_whitelisted_field() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        let assertion = equals_assertion("error.status_code");
        let (found_error, found_method) = resolvable_equals_error_field(&assertion, &errors).expect("must resolve");
        assert_eq!(found_error.name, "SampleError");
        assert_eq!(found_method.name, "status_code");
    }

    #[test]
    fn resolvable_equals_error_field_rejects_a_non_equals_assertion_type() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        let mut assertion = equals_assertion("error.status_code");
        assertion.assertion_type = "greater_than".to_string();
        assert!(
            resolvable_equals_error_field(&assertion, &errors).is_none(),
            "only `equals` is rendered through this path today"
        );
    }

    #[test]
    fn resolvable_equals_error_field_rejects_a_field_outside_the_error_namespace() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        let assertion = equals_assertion("status_code");
        assert!(
            resolvable_equals_error_field(&assertion, &errors).is_none(),
            "a bare `status_code` field (no `error.` prefix) targets the Ok value, not the error"
        );
    }

    #[test]
    fn resolvable_equals_error_field_rejects_an_unwhitelisted_field() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        let assertion = equals_assertion("error.retry_after");
        assert!(resolvable_equals_error_field(&assertion, &errors).is_none());
    }

    #[test]
    fn resolvable_equals_error_field_rejects_a_fieldless_assertion() {
        let errors = vec![error_with_methods("SampleError", vec!["status_code"])];
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: None,
            value: Some(serde_json::json!(429)),
            ..Assertion::default()
        };
        assert!(resolvable_equals_error_field(&assertion, &errors).is_none());
    }
}
