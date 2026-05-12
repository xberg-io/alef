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

/// Emit the return-value marshalling code shared by both function and method wrappers.
///
/// This function emits the code to convert the raw P/Invoke `result` into the managed return
/// type and store it in a local variable `returnValue`.  It intentionally does **not** emit
/// the `return` statement so that callers can interpose cleanup (param handle teardown) between
/// the value computation and the return.
///
/// `enum_names`: the set of C# type names that are enums (not opaque handles).
/// `true_opaque_types`: types with `is_opaque = true` — wrapped in `new CsType(result)`.
///
/// Callers must invoke `emit_return_statement` after their cleanup to complete the method body.
pub(super) fn emit_return_marshalling(
    out: &mut String,
    return_type: &TypeRef,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
) {
    emit_return_marshalling_indented(out, return_type, "        ", enum_names, true_opaque_types);
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
        if true_opaque_types.contains(type_name) {
            // Truly opaque handle: wrap the IntPtr in the C# handle class.
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
