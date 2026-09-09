//! C# exception class generation.

use crate::core::ir::{ErrorDef, ErrorVariant, TypeRef};
use std::collections::HashSet;

/// The literal prefix of `variant`'s `#[error("...")]` template that `FromLastError`'s generated
/// `message.StartsWith(...)` dispatch matches against, or `None` when the template has no
/// literal text before its first `{` placeholder (or no template at all) — such a variant cannot
/// be told apart from any other by message prefix, so it gets no dispatch line and stays on the
/// generic fallback exception.
///
/// The single source of truth for "can C# dispatch this variant to its own exception class
/// today": both [`compute_variant_dispatch`] (which builds the dispatch lines this predicate
/// describes) and `e2e::codegen::declared_error_variant::substantiates_variant_identity` call
/// this exact function, so the e2e assertion generator can never claim a variant is
/// substantiable that the binding generator did not actually wire a dispatch line for. ~keep
pub(crate) fn variant_dispatch_prefix(variant: &ErrorVariant) -> Option<String> {
    let template = variant.message_template.as_deref()?;
    let prefix_end = template.find('{').unwrap_or(template.len());
    let prefix = template[..prefix_end].trim_end().to_string();
    if prefix.is_empty() { None } else { Some(prefix) }
}

/// Compute `FromLastError`'s per-variant message-prefix dispatch lines across every declared
/// error type (not just the crate's first `ErrorDef` — a prior version of this logic lived
/// inline in `gen_wrapper_class` and only ever read `errors[0]`, silently leaving every variant
/// of a second or later error type undispatched) plus the exception class that receives the FFI
/// "unknown" fallback (`ApiSurface::FFI_ERROR_CODE_UNKNOWN`, code 2).
///
/// Returns `(has_base_error, base_exception_class, dispatch_lines)`. Longer prefixes are tried
/// first so a variant whose prefix is itself a prefix of another variant's message never
/// shadows the more specific match.
///
/// Every variant dispatches by message prefix here, `InvalidInput` included: the FFI layer's
/// numeric code 1 is the infrastructure `ALEF_FFI_CONVERSION_ERROR`, not a slot reserved for a
/// user variant that happens to share that name, and `ApiSurface::validate_error_taxonomy`
/// forbids user `error_code`s below 100 — so no legitimate user variant can ever earn code 1. A
/// prior version special-cased `code == 1` straight to `InvalidInputException`, which mislabeled
/// every real conversion failure as that variant whenever an error enum happened to declare
/// one. ~keep
pub(super) fn compute_variant_dispatch(errors: &[ErrorDef]) -> (bool, String, Vec<String>) {
    if errors.is_empty() {
        return (false, String::new(), Vec::new());
    }
    let base_exception_class = format!("{}Exception", errors[0].name);

    let mut seen: HashSet<String> = HashSet::new();
    let mut variants_with_prefix: Vec<(String, String)> = Vec::new();
    for error in errors {
        for variant in &error.variants {
            let Some(prefix) = variant_dispatch_prefix(variant) else {
                continue;
            };
            let class_name = format!("{}Exception", variant.name);
            if seen.insert(class_name.clone()) {
                variants_with_prefix.push((class_name, prefix));
            }
        }
    }
    variants_with_prefix.sort_by_key(|item| std::cmp::Reverse(item.1.len()));

    let dispatch_lines = variants_with_prefix
        .into_iter()
        .map(|(class, prefix)| {
            let escaped_prefix = prefix.replace('\\', "\\\\").replace('"', "\\\"");
            format!("        if (message.StartsWith(\"{escaped_prefix}\")) return WithNativeCode(new {class}(message), code);")
        })
        .collect();

    (true, base_exception_class, dispatch_lines)
}

/// Generate a generic `{ClassName} : Exception` class used as the fallback error type. Also
/// carries the `FromLastError` factory every fallible throw site in the binding calls, so a
/// variant's per-class identity is dispatched uniformly regardless of which template renders
/// the throw.
pub(super) fn gen_exception_class(namespace: &str, class_name: &str, errors: &[ErrorDef]) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let (has_base_error, base_exception_class, variant_dispatch_lines) = compute_variant_dispatch(errors);

    render(
        "exception_class.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
            "has_base_error": has_base_error,
            "base_exception_class": base_exception_class,
            "variant_dispatch_lines": variant_dispatch_lines,
        })),
    )
}

/// Compute the set of types that are returned as opaque handles (matching `*mut T` pattern).
/// A type is considered opaque-handle-returned if any public function or method returns the
/// type directly or wrapped in Optional/Vec — those all surface across FFI as `*mut T`.
/// Includes types with NO serde support (truly opaque handles only); serde-capable types
/// are routed through JSON marshalling, even when the FFI layer omits the `*_to_json`
/// helper for the type itself (the consumer constructs the handle by calling the
/// corresponding `*_from_json` on the engine result).
pub(super) fn compute_handle_returned_types(api: &crate::core::ir::ApiSurface) -> HashSet<String> {
    fn inner_named(ty: &crate::core::ir::TypeRef) -> Option<&str> {
        match ty {
            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
            crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }

    let mut type_def_map = std::collections::HashMap::new();
    for typ in &api.types {
        type_def_map.insert(typ.name.clone(), typ);
    }

    let mut handle_types = HashSet::new();

    for func in &api.functions {
        if let Some(name) = inner_named(&func.return_type)
            && let Some(type_def) = type_def_map.get(name)
            && !type_def.has_serde
        {
            handle_types.insert(name.to_string());
        }
    }

    for typ in &api.types {
        for method in &typ.methods {
            if let Some(name) = inner_named(&method.return_type)
                && let Some(type_def) = type_def_map.get(name)
                && !type_def.has_serde
            {
                handle_types.insert(name.to_string());
            }
        }
    }

    handle_types
}

/// Emit the final `return returnValue;` statement after cleanup.
pub(super) fn emit_return_statement(out: &mut String, return_type: &TypeRef) {
    emit_return_statement_indented(out, return_type, "        ");
}

/// Emit the return-value marshalling code with configurable indentation.
///
/// Like `emit_return_marshalling` this stores the value in `returnValue` without emitting
/// the final `return` statement.  Callers must call `emit_return_statement_indented` after.
pub(super) fn emit_return_marshalling_indented(
    out: &mut String,
    return_type: &TypeRef,
    indent: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    enum_data_variant_names: &HashSet<String>,
) {
    use super::{returns_bool_via_int, returns_json_object, returns_string};
    use crate::backends::csharp::template_env::render;
    use crate::backends::csharp::type_map::csharp_type;
    use crate::codegen::naming::csharp_type_name;

    if *return_type == TypeRef::Unit {
        return;
    }

    if returns_string(return_type) {
        out.push_str(&render("return_string_utf8.jinja", minijinja::context! { indent }));
        out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
    } else if returns_bool_via_int(return_type) {
        out.push_str(&render("return_bool_from_int.jinja", minijinja::context! { indent }));
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = csharp_type_name(type_name);
        if true_opaque_types.contains(type_name)
            || true_opaque_types.contains(&pascal)
            || handle_returned_types.contains(type_name)
            || handle_returned_types.contains(&pascal)
        {
            out.push_str(&render(
                "return_opaque_ctor.jinja",
                minijinja::context! { indent, pascal },
            ));
        } else if !enum_names.contains(&pascal) || enum_data_variant_names.contains(&pascal) {
            // A data struct, or *any* enum (fieldless or data-carrying), is boxed by
            // `insert_handle` exactly like any other `Named` return (`gen_owned_value_to_c` in
            // the FFI crate has no enum-ness branch, and no fieldless-vs-data-carrying branch
            // either, for owned conversion) — so `nativeResult` here is the `AlefHandle` scalar,
            // not a pointer, and must be exchanged for the JSON string via the `{Pascal}ToJson`
            // companion before it can be freed and deserialised. Passing `nativeResult` straight
            // to `Marshal.PtrToStringUTF8` (the `else` branch below) is the CS1503
            // `ulong`-to-`nint` defect this condition exists to avoid. `enum_data_variant_names`
            // (see `enum_names_with_data_variants` in `marshalling.rs`) now covers every enum
            // name, not just data-carrying ones, so this branch is always taken for a genuine
            // enum return and the `else` below is unreachable for enums — kept only as a
            // structural fallback, not because a live case still needs it. ~keep
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&render(
                "native_to_json_ptr.jinja",
                minijinja::context! { indent, to_json_method },
            ));
            out.push_str(&render(
                "json_from_ptr.jinja",
                minijinja::context! { indent, ptr_var => "jsonPtr" },
            ));
            out.push_str(&render(
                "free_string_ptr.jinja",
                minijinja::context! { indent, ptr_var => "jsonPtr" },
            ));
            out.push_str(&render(
                "free_native_handle.jinja",
                minijinja::context! { indent, free_method },
            ));
            out.push_str(&render(
                "deserialize_json.jinja",
                minijinja::context! { indent, cs_type => cs_ty },
            ));
        } else {
            let cs_ty = csharp_type(return_type);
            out.push_str(&render(
                "json_from_ptr.jinja",
                minijinja::context! { indent, ptr_var => "nativeResult" },
            ));
            out.push_str(&render(
                "free_string_ptr.jinja",
                minijinja::context! { indent, ptr_var => "nativeResult" },
            ));
            out.push_str(&render(
                "deserialize_json.jinja",
                minijinja::context! { indent, cs_type => cs_ty },
            ));
        }
    } else if returns_json_object(return_type) {
        if let TypeRef::Optional(inner) = return_type {
            if returns_string(inner) {
                out.push_str(&render("return_ptr_as_string.jinja", minijinja::context! { indent }));
                out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
                return;
            }
            if let TypeRef::Named(type_name) = inner.as_ref() {
                let pascal = csharp_type_name(type_name);
                if true_opaque_types.contains(type_name)
                    || true_opaque_types.contains(&pascal)
                    || handle_returned_types.contains(type_name)
                    || handle_returned_types.contains(&pascal)
                {
                    out.push_str(&render(
                        "return_opaque_ctor.jinja",
                        minijinja::context! { indent, pascal },
                    ));
                    return;
                }
                let to_json_method = format!("{pascal}ToJson");
                let free_method = format!("{pascal}Free");
                let cs_ty = csharp_type(return_type);
                out.push_str(&render(
                    "native_to_json_ptr.jinja",
                    minijinja::context! { indent, to_json_method },
                ));
                out.push_str(&render(
                    "json_from_ptr.jinja",
                    minijinja::context! { indent, ptr_var => "jsonPtr" },
                ));
                out.push_str(&render(
                    "free_string_ptr.jinja",
                    minijinja::context! { indent, ptr_var => "jsonPtr" },
                ));
                out.push_str(&render(
                    "free_native_handle.jinja",
                    minijinja::context! { indent, free_method },
                ));
                out.push_str(&render(
                    "deserialize_json.jinja",
                    minijinja::context! { indent, cs_type => cs_ty },
                ));
                return;
            }
        }
        let cs_ty = csharp_type(return_type);
        out.push_str(&render(
            "json_from_ptr.jinja",
            minijinja::context! { indent, ptr_var => "nativeResult" },
        ));
        out.push_str(&render(
            "free_string_ptr.jinja",
            minijinja::context! { indent, ptr_var => "nativeResult" },
        ));
        out.push_str(&render(
            "deserialize_json.jinja",
            minijinja::context! { indent, cs_type => cs_ty },
        ));
    } else {
        out.push_str(&render("return_native_result.jinja", minijinja::context! { indent }));
    }
}

/// Emit the final `return returnValue;` with configurable indentation.
pub(super) fn emit_return_statement_indented(out: &mut String, return_type: &TypeRef, indent: &str) {
    if *return_type != TypeRef::Unit {
        out.push_str(&crate::backends::csharp::template_env::render(
            "return_value.jinja",
            minijinja::context! { indent },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(name: &str, message_template: Option<&str>) -> ErrorVariant {
        ErrorVariant {
            name: name.to_string(),
            message_template: message_template.map(str::to_string),
            is_unit: true,
            ..ErrorVariant::default()
        }
    }

    fn error(name: &str, variants: Vec<ErrorVariant>) -> ErrorDef {
        ErrorDef {
            name: name.to_string(),
            rust_path: format!("lib::{name}"),
            original_rust_path: String::new(),
            variants,
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    #[test]
    fn variant_dispatch_prefix_takes_the_literal_text_before_the_first_placeholder() {
        assert_eq!(
            variant_dispatch_prefix(&variant("Authentication", Some("Authentication failed: {reason}"))),
            Some("Authentication failed:".to_string())
        );
    }

    #[test]
    fn variant_dispatch_prefix_is_none_without_a_template() {
        assert_eq!(variant_dispatch_prefix(&variant("Authentication", None)), None);
    }

    #[test]
    fn variant_dispatch_prefix_is_none_when_the_template_opens_on_a_placeholder() {
        assert_eq!(variant_dispatch_prefix(&variant("Wrapped", Some("{0}"))), None);
    }

    /// The regression this fix closes: a prior version of this logic only ever read
    /// `errors[0]`, so a second error type's variants never got a dispatch line no matter what
    /// their message template said. `compute_variant_dispatch` must dispatch variants from
    /// EVERY error type, not only the first.
    #[test]
    fn compute_variant_dispatch_covers_every_error_type_not_only_the_first() {
        let errors = vec![
            error(
                "ApiError",
                vec![variant("Authentication", Some("Authentication failed: {reason}"))],
            ),
            error(
                "StorageError",
                vec![variant("Corrupt", Some("Corrupt archive: {path}"))],
            ),
        ];
        let (has_base_error, base_exception_class, dispatch_lines) = compute_variant_dispatch(&errors);
        assert!(has_base_error);
        assert_eq!(base_exception_class, "ApiErrorException");
        // Longer prefixes sort first, so `Authentication failed:` (longer) precedes
        // `Corrupt archive:` (shorter) regardless of declaration order. Asserting the exact two
        // lines, in exact order, is what would catch a regression back to `errors[0]`-only (the
        // second error type's line would silently disappear).
        assert_eq!(
            dispatch_lines,
            vec![
                "        if (message.StartsWith(\"Authentication failed:\")) return WithNativeCode(new AuthenticationException(message), code);"
                    .to_string(),
                "        if (message.StartsWith(\"Corrupt archive:\")) return WithNativeCode(new CorruptException(message), code);".to_string(),
            ]
        );
    }

    #[test]
    fn compute_variant_dispatch_skips_variants_with_no_dispatchable_prefix() {
        let errors = vec![error(
            "ApiError",
            vec![
                variant("Authentication", Some("Authentication failed: {reason}")),
                variant("Wrapped", Some("{0}")),
                variant("Unknown", None),
            ],
        )];
        let (_, _, dispatch_lines) = compute_variant_dispatch(&errors);
        assert_eq!(
            dispatch_lines,
            vec![
                "        if (message.StartsWith(\"Authentication failed:\")) return WithNativeCode(new AuthenticationException(message), code);"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn compute_variant_dispatch_with_no_errors_disables_dispatch() {
        assert_eq!(compute_variant_dispatch(&[]), (false, String::new(), Vec::new()));
    }

    /// A real fix makes the WRONG variant fail: `FromLastError` must contain a dispatch line
    /// for `Authentication` and a SEPARATE one for `BadRequest`, each returning its own class —
    /// not a single shared branch both variant names would satisfy. Asserting the exact
    /// generated file (not a substring) is what would catch a regression that collapsed both
    /// variants onto one class or dropped a line.
    #[test]
    fn gen_exception_class_renders_a_distinct_dispatch_line_per_variant() {
        let errors = vec![error(
            "ApiError",
            vec![
                variant("Authentication", Some("Authentication failed: {reason}")),
                variant("BadRequest", Some("Bad request: {reason}")),
            ],
        )];
        let rendered = gen_exception_class("Sample.Client", "SampleClientException", &errors);
        let lines: Vec<&str> = rendered.lines().collect();
        // Exact line-by-line comparison (not `.contains`): a regression that collapsed both
        // variants onto one class, dropped a line, or reordered them would change this Vec.
        let expected: Vec<&str> = vec![
            "// This file is auto-generated by alef. DO NOT EDIT.",
            "#nullable enable",
            "",
            "using System;",
            "",
            "namespace Sample.Client;",
            "",
            "public class SampleClientException : Exception",
            "{",
            "    public int Code { get; private set; }",
            "",
            "    public SampleClientException(int code, string message) : base(message)",
            "    {",
            "        Code = code;",
            "    }",
            "",
            "    public SampleClientException(string message) : base(message)",
            "    {",
            "        Code = 0;",
            "    }",
            "",
            "    public SampleClientException(string message, Exception innerException) : base(message, innerException)",
            "    {",
            "        Code = 0;",
            "    }",
            "",
            "    private static SampleClientException WithNativeCode(SampleClientException exception, int code)",
            "    {",
            "        exception.Code = code;",
            "        return exception;",
            "    }",
            "",
            "    /// <summary>",
            "    /// Builds the concrete exception for the FFI's current thread-local last-error state,",
            "    /// dispatching to the specific per-variant exception class when the message's prefix",
            "    /// identifies a known variant. Every throw site across the generated binding funnels",
            "    /// through here so a variant's identity is never lost to a bypassed dispatch.",
            "    /// </summary>",
            "    internal static Exception FromLastError(string fallbackMessage)",
            "    {",
            "        var code = NativeMethods.LastErrorCode();",
            "        var ctxPtr = NativeMethods.LastErrorContext();",
            "        var message = global::System.Runtime.InteropServices.Marshal.PtrToStringUTF8(ctxPtr) ?? fallbackMessage;",
            "        if (message.StartsWith(\"Authentication failed:\")) return WithNativeCode(new AuthenticationException(message), code);",
            "        if (message.StartsWith(\"Bad request:\")) return WithNativeCode(new BadRequestException(message), code);",
            "        if (code == 2) return WithNativeCode(new ApiErrorException(message), code);",
            "        return new SampleClientException(code, message);",
            "    }",
            "}",
        ];
        assert_eq!(lines, expected, "got:\n{rendered}");
    }
}
