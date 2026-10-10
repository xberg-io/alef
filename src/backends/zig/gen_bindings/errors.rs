use crate::core::ir::ErrorDef;

/// Case a Rust error-variant identifier into a Zig error-set tag.
///
/// This is deliberately *not* `public_host_identifier(Zig, PublicIdentifierKind::Type, ...)`.
/// That path assumes (per issue #455) the input is already a PascalCase Rust IR *type* name and
/// is now identity for Zig -- correct for `pub const {typ.name} = struct`, where `typ.name` is a
/// real Rust struct/enum identifier. An error-variant identifier is not that: `ErrorVariant.name`
/// is `v.ident.to_string()` (`src/extract/extractor/types.rs`), i.e. the bare Rust variant
/// spelling with no host-language casing applied yet, and Zig error tags follow their own
/// PascalCase convention independent of what casing the source enum used. Before #455 this call
/// went through the same catch-all arm when it still applied `to_pascal_case()` unconditionally;
/// #455 turned that arm into an identity mapping for real type names and, as a side effect, this
/// caller stopped being cased at all (see issue #467). Routing through `cased_public_type_name`
/// keeps this caller's pre-#455 behavior -- PascalCase an arbitrary input -- without reopening
/// #455 for the type-name callers that must stay identity. ~keep
fn zig_error_variant_component(name: &str) -> String {
    crate::codegen::naming::cased_public_type_name(crate::core::config::Language::Zig, name)
}

pub(crate) fn emit_error_set(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_doc_block.jinja",
            crate::alef_context! {
                error_doc_lines => error.doc.lines().collect::<Vec<_>>(),
            },
        ));
    }
    out.push_str(&crate::backends::zig::template_env::render(
        "error_set_header.jinja",
        crate::alef_context! {
            error_name => &error.name,
        },
    ));
    for variant in &error.variants {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            crate::alef_context! {
                variant_name => zig_error_variant_component(&variant.name),
            },
        ));
    }
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "OutOfMemory")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            crate::alef_context! {
                variant_name => "OutOfMemory",
            },
        ));
    }
    // The generated helpers return `error.UnknownFfiError` whenever the FFI layer reports a
    // failure that no declared `#[alef(error_code = N)]` substantiates (the Zig mirror of
    // `ALEF_FFI_UNKNOWN_ERROR`). Zig coerces that only into a set that declares the member, and
    // the helpers are instantiated with every declared set, so it must be injected here. ~keep
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "UnknownFfiError")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            crate::alef_context! {
                variant_name => "UnknownFfiError",
            },
        ));
    }
    // `_error_with_message` returns `error.Cancelled` for the FFI cancellation code, so every
    // declared set must admit it for the same reason it admits `UnknownFfiError`. ~keep
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "Cancelled")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            crate::alef_context! {
                variant_name => "Cancelled",
            },
        ));
    }
    out.push_str("};\n");
}

/// The Zig error-set expression a function's own `error_type` resolves to when it names no
/// declared error set (and at least one is declared -- an empty crate falls back to `anyerror`
/// instead, see `resolve_zig_error_type`). Every declared set already carries both members
/// (`emit_error_set` injects them unconditionally), so this is exactly what an
/// untyped-but-fallible function already gets from `wrapper_return_type`, and
/// `_error_with_message` on it falls through every `if (E == ...)` arm to the same
/// `UnknownFfiError` a real mismatch deserves. ~keep
const UNMATCHED_ERROR_SET: &str = "error{OutOfMemory,UnknownFfiError,Cancelled}";

/// Map a Rust error_type (e.g. `"anyhow::Error"`, `"SampleCrateError"`) to a
/// Zig error-set expression. If the path's last segment matches a declared
/// error set by name, use it.
///
/// A mismatch (a foreign type like `anyhow::Error`, or an extraction-resolved
/// name none of the crate's declared sets carry) used to fall back to "the
/// first declared error set" on the theory that a crate typically has one
/// primary error type. That guess is silently wrong for any crate with more
/// than one declared error set: nothing here can tell "the crate's one true
/// error type, spelled unusually" apart from "an unrelated second error type
/// this function has nothing to do with," so guessing attributes the
/// function to a real, wrong, unrelated declared error instead of an honest
/// unknown. The safe answer is [`UNMATCHED_ERROR_SET`], not a specific named
/// identity. ~keep
pub(crate) fn resolve_zig_error_type(error_type: &str, declared: &[String]) -> String {
    let last = error_type.rsplit("::").next().unwrap_or(error_type);
    if declared.iter().any(|d| d == last) {
        return last.to_string();
    }
    if declared.is_empty() {
        return "anyerror".to_string();
    }
    UNMATCHED_ERROR_SET.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ErrorVariant;

    fn variant(name: &str, template: Option<&str>) -> ErrorVariant {
        ErrorVariant {
            error_code: None,
            name: name.to_string(),
            message_template: template.map(str::to_string),
            ..ErrorVariant::default()
        }
    }

    /// Issue #467 regression: `zig_error_variant_component` used to route through
    /// `public_host_identifier(Zig, Type, ...)`, which #455 turned into an identity mapping for
    /// every catch-all-arm language including Zig. `ErrorVariant.name` is the raw
    /// `v.ident.to_string()` extracted from source (`src/extract/extractor/types.rs`), so a
    /// caller can hand this an uncased name; it must come back PascalCased, matching Zig's
    /// idiomatic error-tag convention. ~keep
    #[test]
    fn zig_error_variant_component_pascal_cases_an_uncased_snake_case_name() {
        assert_eq!(zig_error_variant_component("connection_failed"), "ConnectionFailed");
    }

    /// Companion to the test above: an input that is already a single PascalCase word must come
    /// back unchanged, not merely "still PascalCase" -- an idempotence check that would pass
    /// trivially if the function scrambled the name into some other PascalCase spelling.
    #[test]
    fn zig_error_variant_component_leaves_an_already_pascal_case_name_unchanged() {
        assert_eq!(zig_error_variant_component("ConnectionFailed"), "ConnectionFailed");
    }

    // `emit_error_set` stopped emitting the `_from_ffi_msg_*` prefix-matcher in
    // a6f094df5 ("feat(go-zig): map typed errors by native code"); dispatch now
    // goes exclusively through the numeric FFI taxonomy code in
    // `helpers::emit_helpers`/`_error_with_message` (see
    // `error_with_message_dispatches_to_each_declared_error` in helpers.rs). ~keep
    #[test]
    fn emit_error_set_emits_only_the_error_set() {
        let error = ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants: vec![variant("Boom", Some("Boom happened: {0}"))],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_error_set(&error, &mut out);

        assert!(
            out.contains("pub const MyError = error{"),
            "expected error set header:\n{out}"
        );
        assert!(out.contains("Boom,"), "expected Boom variant:\n{out}");
        assert!(
            out.contains("OutOfMemory,"),
            "expected implicit OutOfMemory variant:\n{out}"
        );
        assert!(
            out.trim_end().ends_with("};"),
            "expected closing brace of error set:\n{out}"
        );
        assert!(
            !out.contains("_from_ffi_msg_"),
            "message-prefix dispatch was replaced by numeric taxonomy-code dispatch \
             in helpers::_error_with_message and must not be emitted here:\n{out}"
        );
        assert!(
            out.contains("UnknownFfiError,"),
            "expected implicit UnknownFfiError variant:\n{out}"
        );
    }

    /// `_error_with_message` returns `error.Cancelled` for the FFI cancellation code, so every
    /// declared set must admit it, exactly once even if the Rust enum already has a `Cancelled`
    /// variant of its own. ~keep
    #[test]
    fn emit_error_set_declares_cancelled_exactly_once() {
        let make = |variants| ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants,
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut implicit = String::new();
        emit_error_set(&make(vec![variant("Boom", None)]), &mut implicit);
        assert_eq!(implicit.matches("Cancelled,").count(), 1, "{implicit}");

        let mut explicit = String::new();
        emit_error_set(
            &make(vec![variant("Cancelled", None), variant("Boom", None)]),
            &mut explicit,
        );
        assert_eq!(explicit.matches("Cancelled,").count(), 1, "{explicit}");

        assert!(UNMATCHED_ERROR_SET.contains("Cancelled"));
    }

    /// `helpers::emit_helpers` emits `return error.UnknownFfiError;` inside a function whose
    /// return type is a caller-supplied `comptime E`. Zig only coerces that literal into `E`
    /// when `E` declares the member, and `E` is instantiated with every generated error set —
    /// so a set missing the member is a compile error in the emitted binding, not a runtime
    /// nicety. Injected exactly once even if the Rust enum already spells it. ~keep
    #[test]
    fn emit_error_set_never_duplicates_an_explicit_unknown_ffi_error_variant() {
        let error = ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants: vec![variant("UnknownFfiError", None), variant("Boom", None)],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_error_set(&error, &mut out);

        assert_eq!(
            out.matches("UnknownFfiError,").count(),
            1,
            "the unknown-error member must be declared exactly once:\n{out}"
        );
    }

    /// A qualified Rust path resolves by its last path segment against the declared error sets.
    /// Asserted against the SECOND declared set specifically: matching only ever against a
    /// single declared set can't distinguish real name-matching from the position-based
    /// fallback this module used to have (see the regression test below). ~keep
    #[test]
    fn resolve_zig_error_type_matches_a_qualified_path_against_the_second_declared_set() {
        let declared = vec!["FirstError".to_string(), "SampleCrateError".to_string()];
        assert_eq!(
            resolve_zig_error_type("crate::error::SampleCrateError", &declared),
            "SampleCrateError"
        );
    }

    /// Regression: `resolve_zig_error_type` used to fall back to "the first declared error set"
    /// for any `error_type` that matched no declared name (a foreign `anyhow::Error`, or an
    /// extraction-resolved name none of the crate's declared sets carry) -- silently attributing
    /// the function to a real, wrong, unrelated declared error. Traced from a consumer whose
    /// functions were typed with a completely unrelated crate error enum because their true
    /// error type failed to match any declared name. The fix must never resolve a mismatch to
    /// either real declared identifier. ~keep
    #[test]
    fn resolve_zig_error_type_never_falls_back_to_an_unrelated_declared_set() {
        let declared = vec!["FirstError".to_string(), "SampleCrateError".to_string()];
        let resolved = resolve_zig_error_type("anyhow::Error", &declared);
        assert_ne!(
            resolved, "FirstError",
            "must not masquerade as the unrelated first-declared set"
        );
        assert_ne!(
            resolved, "SampleCrateError",
            "must not masquerade as the unrelated second-declared set"
        );
        assert_eq!(resolved, UNMATCHED_ERROR_SET);
    }

    #[test]
    fn resolve_zig_error_type_yields_anyerror_without_any_declared_set() {
        assert_eq!(resolve_zig_error_type("SampleCrateError", &[]), "anyerror");
    }

    /// Integration-level fixtures for the same regression, exercised through
    /// `functions::wrapper_return_type` -- the actual function-signature generator every free
    /// function's return type goes through -- rather than the raw resolver alone. The fixture
    /// crate declares two unrelated error enums (`FirstError`, `SecondError`); each test below
    /// states explicitly which one the function under test actually returns. ~keep
    mod wrapper_return_type_integration {
        use crate::backends::zig::gen_bindings::functions::wrapper_return_type;
        use crate::core::ir::{FunctionDef, TypeRef};

        fn declared_two_errors() -> Vec<String> {
            vec!["FirstError".to_string(), "SecondError".to_string()]
        }

        fn fallible_function(name: &str, error_type: &str) -> FunctionDef {
            FunctionDef {
                name: name.to_string(),
                return_type: TypeRef::Unit,
                error_type: Some(error_type.to_string()),
                ..FunctionDef::default()
            }
        }

        /// The regression itself: a function whose Rust return type is `Result<_, SecondError>`
        /// -- the SECOND declared error, not the crate's first -- must resolve to `SecondError`,
        /// never to `FirstError` merely because `FirstError` was declared first. ~keep
        #[test]
        fn a_function_returning_the_second_declared_error_resolves_to_it() {
            let f = fallible_function("uses_second_error", "SecondError");
            let return_ty = wrapper_return_type(&f, &declared_two_errors(), &Default::default(), &Default::default());

            assert!(
                return_ty.starts_with("SecondError!"),
                "a function returning `Result<_, SecondError>` must be typed `SecondError!...`, got: {return_ty}"
            );
            assert!(
                !return_ty.contains("FirstError"),
                "must never carry the unrelated first-declared error: {return_ty}"
            );
        }

        /// Negative control: a function that genuinely returns the crate's first-declared error
        /// must still resolve to it. Without this, the regression test above would pass trivially
        /// if the fix broke ordinary matching instead of just the mismatch fallback. ~keep
        #[test]
        fn a_function_returning_the_first_declared_error_still_resolves_to_it() {
            let f = fallible_function("uses_first_error", "FirstError");
            let return_ty = wrapper_return_type(&f, &declared_two_errors(), &Default::default(), &Default::default());

            assert!(
                return_ty.starts_with("FirstError!"),
                "a function returning `Result<_, FirstError>` must be typed `FirstError!...`, got: {return_ty}"
            );
        }

        /// A function whose own error type names neither declared error must never be attributed
        /// to one of them -- the exact shape of the reported defect (three functions returning a
        /// crate's real, unrelated error enum instead of their own). ~keep
        #[test]
        fn a_function_with_an_unmatched_error_type_is_never_mislabeled_as_a_declared_set() {
            let f = fallible_function("uses_foreign_error", "anyhow::Error");
            let return_ty = wrapper_return_type(&f, &declared_two_errors(), &Default::default(), &Default::default());

            assert!(
                !return_ty.starts_with("FirstError!") && !return_ty.starts_with("SecondError!"),
                "an unmatched error type must never be attributed to an unrelated declared error, got: {return_ty}"
            );
            assert!(
                return_ty.contains("UnknownFfiError"),
                "the safe fallback must still admit UnknownFfiError so `_error_with_message` compiles: {return_ty}"
            );
        }
    }
}
