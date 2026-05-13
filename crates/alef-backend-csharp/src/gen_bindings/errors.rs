//! C# exception class generation.

use alef_core::ir::TypeRef;
use std::collections::HashSet;

/// Generate a generic `{ClassName} : Exception` class used as the fallback error type.
pub(super) fn gen_exception_class(namespace: &str, class_name: &str) -> String {
    use crate::template_env::render;
    use minijinja::Value;

    render(
        "exception_class.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
        })),
    )
}

/// Compute the set of types that are returned as opaque handles (matching `*mut T` pattern).
/// A type is considered opaque-handle-returned if any public function returns TypeRef::Named(T).
pub(super) fn compute_handle_returned_types(api: &alef_core::ir::ApiSurface) -> HashSet<String> {
    let mut handle_types = HashSet::new();

    // Scan functions for Named return types (will be emitted as *mut T in FFI).
    for func in &api.functions {
        if let alef_core::ir::TypeRef::Named(name) = &func.return_type {
            handle_types.insert(name.clone());
        }
    }

    // Scan methods for Named return types.
    for typ in &api.types {
        for method in &typ.methods {
            if let alef_core::ir::TypeRef::Named(name) = &method.return_type {
                handle_types.insert(name.clone());
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
) {
    use super::{returns_bool_via_int, returns_json_object, returns_string};
    use crate::template_env::render;
    use crate::type_map::csharp_type;
    use heck::ToPascalCase;

    if *return_type == TypeRef::Unit {
        return;
    }

    if returns_string(return_type) {
        // IntPtr → string, then free the native buffer.
        out.push_str(&render("return_string_utf8.jinja", minijinja::context! { indent }));
        out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
    } else if returns_bool_via_int(return_type) {
        // C int → bool
        out.push_str(&render("return_bool_from_int.jinja", minijinja::context! { indent }));
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = type_name.to_pascal_case();
        if true_opaque_types.contains(type_name) || handle_returned_types.contains(type_name) {
            // Truly opaque handle (is_opaque = true) OR returned from a public function (as *mut T).
            // Both are wrapped in the C# handle class.
            out.push_str(&render(
                "return_opaque_ctor.jinja",
                minijinja::context! { indent, pascal },
            ));
        } else if !enum_names.contains(&pascal) {
            // Data struct with to_json: call to_json, deserialise, then free both.
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
            // Enum returned as JSON string IntPtr.
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
        // Optional<String> — the FFI returns a raw C string (not JSON-encoded).
        if let TypeRef::Optional(inner) = return_type {
            if returns_string(inner) {
                out.push_str(&render("return_ptr_as_string.jinja", minijinja::context! { indent }));
                out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
                return;
            }
            // Optional<Named<T>> where T is an opaque-handle return: emit constructor wrapper.
            // (The IntPtr.Zero null-check has already been emitted by the caller via
            // `emit_named_param_setup` / the wrapper template's null-sentinel handling.)
            if let TypeRef::Named(type_name) = inner.as_ref() {
                let pascal = type_name.to_pascal_case();
                if true_opaque_types.contains(type_name) || handle_returned_types.contains(type_name) {
                    out.push_str(&render(
                        "return_opaque_ctor.jinja",
                        minijinja::context! { indent, pascal },
                    ));
                    return;
                }
            }
        }
        // IntPtr → JSON string → deserialized object, then free the native buffer.
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
        // Numeric primitives — direct return.
        out.push_str(&render("return_native_result.jinja", minijinja::context! { indent }));
    }
}

/// Emit the final `return returnValue;` with configurable indentation.
pub(super) fn emit_return_statement_indented(out: &mut String, return_type: &TypeRef, indent: &str) {
    if *return_type != TypeRef::Unit {
        out.push_str(&crate::template_env::render(
            "return_value.jinja",
            minijinja::context! { indent },
        ));
    }
}
