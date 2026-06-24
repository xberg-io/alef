/// Returns true when the IR type is `Vec<u8>` (binary data → `ByteArray`).
fn is_vec_u8(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8))
    )
}

fn is_binary_return_type(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Bytes) || is_vec_u8(ty)
}

fn is_optional_binary_return_type(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Optional(inner) if is_binary_return_type(inner))
}

fn is_binary_param_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)),
        TypeRef::Optional(inner) => is_binary_param_type(inner),
        _ => false,
    }
}

/// Returns true when the bridge return type is a JSON String that must be
/// deserialised into a richer Kotlin type in the wrapper body.
fn needs_json_deserialize(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named(_) => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Named(_)),
        TypeRef::Map(_, _) => true,
        TypeRef::Vec(inner) => {
            // Vec<u8> → ByteArray (pass-through); other Vec → JSON String → needs deserialize.
            !matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8))
        }
        _ => false,
    }
}

/// Returns true when the bridge return type for an opaque is a JSON String that must be
/// deserialised (as opposed to being a raw handle).
///
/// Opaque types that are known handles do NOT need JSON deserialization — they return Long.
/// Data types (structs, enums, maps, etc.) return JSON String and need deserialization.
fn needs_json_deserialize_for_method(
    ty: &TypeRef,
    opaque_type_names: &std::collections::HashSet<&str>,
) -> bool {
    // If it's an opaque type name, it's a handle that returns Long (not JSON)
    if let TypeRef::Named(n) = ty {
        if opaque_type_names.contains(n.as_str()) {
            return false;
        }
    }
    // For Optional<OpaqueType>, also exclude it
    if let TypeRef::Optional(inner) = ty {
        if let TypeRef::Named(n) = inner.as_ref() {
            if opaque_type_names.contains(n.as_str()) {
                return false;
            }
        }
    }
    // Otherwise use the standard logic (data types need deserialization)
    needs_json_deserialize(ty)
}
/// Map an IR `TypeRef` to a JNI-compatible Kotlin type string for `external fun` return types
/// on instance methods (where opaque handle semantics do not apply to the return).
///
/// JNI external funs must use primitive-width types and `String` for text.
/// Complex types (structs, enums) are passed as JSON-encoded `String` values.
/// `Vec<u8>` maps to `ByteArray` so binary responses (images, speech audio) avoid
/// base64 overhead through Jackson.
fn jni_return_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "Unit",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean",
                PrimitiveType::I8 => "Byte",
                PrimitiveType::I16 => "Short",
                PrimitiveType::I32 => "Int",
                PrimitiveType::I64 => "Long",
                PrimitiveType::U8 => "Byte",
                PrimitiveType::U16 => "Short",
                PrimitiveType::U32 => "Int",
                PrimitiveType::U64 => "Long",
                PrimitiveType::F32 => "Float",
                PrimitiveType::F64 => "Double",
                PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            }
        }
        TypeRef::String => "String",
        TypeRef::Bytes => "ByteArray",
        // Optional return → nullable String (JSON-encoded or null)
        TypeRef::Optional(inner) if is_binary_return_type(inner) => "ByteArray?",
        TypeRef::Optional(_) => "String?",
        // Named types (structs, enums, errors) → JSON-encoded String
        TypeRef::Named(_) => "String",
        // Vec<u8> (binary data) → ByteArray; other collections → JSON-encoded String
        TypeRef::Vec(inner) => {
            if matches!(inner.as_ref(), TypeRef::Primitive(crate::core::ir::PrimitiveType::U8)) {
                "ByteArray"
            } else {
                "String"
            }
        }
        TypeRef::Map(_, _) => "String",
        // Opaque handle → Long
        _ => "Long",
    }
}

/// Map an IR `TypeRef` to a JNI-compatible Kotlin type string for top-level function
/// return types, where opaque named types become `Long` (raw handle) instead of `String`.
fn jni_return_type_for_function(ty: &TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> &'static str {
    if let TypeRef::Named(n) = ty {
        if opaque_type_names.contains(n.as_str()) {
            return "Long";
        }
    }
    jni_return_type(ty)
}

/// Map an IR `TypeRef` to a JNI-compatible Kotlin type string for instance method
/// return types, where opaque named types become `Long` (raw handle) instead of `String`.
#[allow(dead_code)]
fn jni_return_type_for_method(ty: &TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> &'static str {
    // Unwrap Optional to check the inner type
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    if let TypeRef::Named(n) = base {
        if opaque_type_names.contains(n.as_str()) {
            // Opaque handle return: the JNI shim returns a primitive `jlong` (0 = None), so the
            // external fun MUST be a non-null primitive `Long` — both for required and optional
            // returns. Optionality is encoded via the `0L` sentinel and unwrapped in the client
            // wrapper (`if (h == 0L) null else Wrapper(h)`). Declaring `Long?` here produces a
            // boxed `java.lang.Long` JNI signature that mismatches the primitive `jlong` the Rust
            // shim returns → object-vs-primitive UB → JVM access violation (tslp issue #146).
            return "Long";
        }
    }
    jni_return_type(ty)
}

/// Build the `external fun native<Method>(...)` parameter list for a function.
///
/// Opaque named types are passed as `Long` (raw handle pointer).
/// Complex non-opaque types (named structs, vec, map, optional-named) are serialized
/// to JSON `String` by the caller. Primitive types map directly to JNI primitives.
fn jni_params_for_function(
    f: &crate::core::ir::FunctionDef,
    opaque_type_names: &std::collections::HashSet<&str>,
) -> String {
    f.params
        .iter()
        .map(|p| {
            let jni_ty = jni_param_type_for_function(&p.ty, opaque_type_names);
            let name = to_lower_camel(&p.name);
            format!("{name}: {jni_ty}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// JNI param type for top-level function params.
///
/// Opaque named types → `Long`; everything else falls through to `jni_param_type`.
fn jni_param_type_for_function(ty: &TypeRef, opaque_type_names: &std::collections::HashSet<&str>) -> &'static str {
    // Unwrap Optional to check the inner type.
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    if let TypeRef::Named(n) = base {
        if opaque_type_names.contains(n.as_str()) {
            return "Long";
        }
    }
    jni_param_type(ty)
}

fn jni_param_type(ty: &TypeRef) -> &'static str {
    if is_binary_param_type(ty) {
        // Binary data (Vec<u8> / Bytes) is base64-encoded to String by the Kotlin
        // wrapper before calling the JNI bridge, which then decodes it back to bytes.
        // This matches the JNI implementation which receives a JString (not jbyteArray).
        return "String";
    }
    match ty {
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean",
                PrimitiveType::I8 => "Byte",
                PrimitiveType::I16 => "Short",
                PrimitiveType::I32 => "Int",
                PrimitiveType::I64 => "Long",
                PrimitiveType::U8 => "Byte",
                PrimitiveType::U16 => "Short",
                PrimitiveType::U32 => "Int",
                PrimitiveType::U64 => "Long",
                PrimitiveType::F32 => "Float",
                PrimitiveType::F64 => "Double",
                PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            }
        }
        TypeRef::String => "String",
        // All complex types (named, optional, vec, map) are passed as JSON String.
        _ => "String",
    }
}
