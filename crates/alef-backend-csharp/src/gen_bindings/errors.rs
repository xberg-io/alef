//! C# exception class generation.

use super::csharp_file_header;
use alef_core::ir::TypeRef;
use std::collections::HashSet;

/// Generate a generic `{ClassName} : Exception` class used as the fallback error type.
pub(super) fn gen_exception_class(namespace: &str, class_name: &str) -> String {
    let mut out = csharp_file_header();
    out.push_str("using System;\n\n");

    out.push_str(&format!("namespace {};\n\n", namespace));

    out.push_str(&format!("public class {} : Exception\n", class_name));
    out.push_str("{\n");
    out.push_str("    public int Code { get; }\n\n");
    out.push_str(&format!(
        "    public {}(int code, string message) : base(message)\n",
        class_name
    ));
    out.push_str("    {\n");
    out.push_str("        Code = code;\n");
    out.push_str("    }\n\n");
    // String-only constructor for derived classes that don't carry a numeric code.
    out.push_str(&format!("    public {}(string message) : base(message)\n", class_name));
    out.push_str("    {\n");
    out.push_str("        Code = 0;\n");
    out.push_str("    }\n\n");
    out.push_str(&format!(
        "    public {}(string message, Exception innerException) : base(message, innerException)\n",
        class_name
    ));
    out.push_str("    {\n");
    out.push_str("        Code = 0;\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
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
    use super::{returns_bool_via_int, returns_json_object, returns_string};
    use crate::type_map::csharp_type;
    use heck::ToPascalCase;

    if *return_type == TypeRef::Unit {
        // void — nothing to return
        return;
    }

    if returns_string(return_type) {
        // IntPtr → string, then free the native buffer.
        out.push_str("        var returnValue = Marshal.PtrToStringUTF8(nativeResult) ?? string.Empty;\n");
        out.push_str("        NativeMethods.FreeString(nativeResult);\n");
    } else if returns_bool_via_int(return_type) {
        // C int → bool
        out.push_str("        var returnValue = nativeResult != 0;\n");
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = type_name.to_pascal_case();
        if true_opaque_types.contains(type_name) {
            // Truly opaque handle: wrap the IntPtr in the C# handle class.
            out.push_str(&format!("        var returnValue = new {pascal}(nativeResult);\n"));
        } else if !enum_names.contains(&pascal) {
            // Data struct with to_json: call to_json, deserialise, then free both.
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!(
                "        var jsonPtr = NativeMethods.{to_json_method}(nativeResult);\n"
            ));
            out.push_str("        var json = Marshal.PtrToStringUTF8(jsonPtr);\n");
            out.push_str("        NativeMethods.FreeString(jsonPtr);\n");
            out.push_str(&format!("        NativeMethods.{free_method}(nativeResult);\n"));
            out.push_str(&format!(
                "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        } else {
            // Enum returned as JSON string IntPtr.
            let cs_ty = csharp_type(return_type);
            out.push_str("        var json = Marshal.PtrToStringUTF8(nativeResult);\n");
            out.push_str("        NativeMethods.FreeString(nativeResult);\n");
            out.push_str(&format!(
                "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        }
    } else if returns_json_object(return_type) {
        // Optional<String> — the FFI returns a raw C string (not JSON-encoded).
        if let TypeRef::Optional(inner) = return_type {
            if returns_string(inner) {
                out.push_str("        var returnValue = Marshal.PtrToStringUTF8(nativeResult);\n");
                out.push_str("        NativeMethods.FreeString(nativeResult);\n");
                return;
            }
        }
        // IntPtr → JSON string → deserialized object, then free the native buffer.
        let cs_ty = csharp_type(return_type);
        out.push_str("        var json = Marshal.PtrToStringUTF8(nativeResult);\n");
        out.push_str("        NativeMethods.FreeString(nativeResult);\n");
        out.push_str(&format!(
            "        var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
            cs_ty
        ));
    } else {
        // Numeric primitives — direct return.
        out.push_str("        var returnValue = nativeResult;\n");
    }
}

/// Emit the final `return returnValue;` statement after cleanup.
pub(super) fn emit_return_statement(out: &mut String, return_type: &TypeRef) {
    if *return_type != TypeRef::Unit {
        out.push_str("        return returnValue;\n");
    }
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
    use crate::type_map::csharp_type;
    use heck::ToPascalCase;

    if *return_type == TypeRef::Unit {
        return;
    }

    if returns_string(return_type) {
        out.push_str(&format!(
            "{indent}var returnValue = Marshal.PtrToStringUTF8(nativeResult) ?? string.Empty;\n"
        ));
        out.push_str(&format!("{indent}NativeMethods.FreeString(nativeResult);\n"));
    } else if returns_bool_via_int(return_type) {
        out.push_str(&format!("{indent}var returnValue = nativeResult != 0;\n"));
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = type_name.to_pascal_case();
        if true_opaque_types.contains(type_name) {
            // Truly opaque handle: wrap the IntPtr in the C# handle class.
            out.push_str(&format!("{indent}var returnValue = new {pascal}(nativeResult);\n"));
        } else if !enum_names.contains(&pascal) {
            // Data struct with to_json: call to_json, deserialise, then free both.
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!(
                "{indent}var jsonPtr = NativeMethods.{to_json_method}(nativeResult);\n"
            ));
            out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(jsonPtr);\n"));
            out.push_str(&format!("{indent}NativeMethods.FreeString(jsonPtr);\n"));
            out.push_str(&format!("{indent}NativeMethods.{free_method}(nativeResult);\n"));
            out.push_str(&format!(
                "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        } else {
            // Enum returned as JSON string IntPtr.
            let cs_ty = csharp_type(return_type);
            out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(nativeResult);\n"));
            out.push_str(&format!("{indent}NativeMethods.FreeString(nativeResult);\n"));
            out.push_str(&format!(
                "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
                cs_ty
            ));
        }
    } else if returns_json_object(return_type) {
        // Optional<String> — the FFI returns a raw C string (not JSON-encoded).
        if let TypeRef::Optional(inner) = return_type {
            if returns_string(inner) {
                out.push_str(&format!(
                    "{indent}var returnValue = Marshal.PtrToStringUTF8(nativeResult);\n"
                ));
                out.push_str(&format!("{indent}NativeMethods.FreeString(nativeResult);\n"));
                return;
            }
        }
        let cs_ty = csharp_type(return_type);
        out.push_str(&format!("{indent}var json = Marshal.PtrToStringUTF8(nativeResult);\n"));
        out.push_str(&format!("{indent}NativeMethods.FreeString(nativeResult);\n"));
        out.push_str(&format!(
            "{indent}var returnValue = JsonSerializer.Deserialize<{}>(json ?? \"null\", JsonOptions)!;\n",
            cs_ty
        ));
    } else {
        out.push_str(&format!("{indent}var returnValue = nativeResult;\n"));
    }
}

/// Emit the final `return returnValue;` with configurable indentation.
pub(super) fn emit_return_statement_indented(out: &mut String, return_type: &TypeRef, indent: &str) {
    if *return_type != TypeRef::Unit {
        out.push_str(&format!("{indent}return returnValue;\n"));
    }
}
