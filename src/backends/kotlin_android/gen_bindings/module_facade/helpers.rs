use crate::core::ir::{PrimitiveType, TypeRef};

/// Return the inner `TypeRef` if `ty` is `Optional(inner)`, otherwise return `ty`.
pub(super) fn unwrap_optional(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

/// Return a nullable Kotlin type string for an optional facade parameter.
pub(super) fn kotlin_nullable_type_for_optional(ty: &TypeRef) -> String {
    let base = match ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    let non_null = match base {
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "Boolean",
            PrimitiveType::I8 | PrimitiveType::U8 => "Byte",
            PrimitiveType::I16 | PrimitiveType::U16 => "Short",
            PrimitiveType::I32 | PrimitiveType::U32 => "Int",
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
        },
        TypeRef::String => "String",
        TypeRef::Bytes => "ByteArray",
        TypeRef::Vec(inner) => {
            // Vec<u8> (binary data) → ByteArray; other Vec → String (will be JSON-encoded)
            if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) {
                "ByteArray"
            } else {
                "String"
            }
        }
        TypeRef::Named(n) => return format!("{n}?"),
        _ => "String",
    };
    format!("{non_null}?")
}

/// Return the Kotlin literal zero-value for a JNI primitive type.
pub(super) fn jni_zero_literal(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String => "\"\"",
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "false",
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            PrimitiveType::I64 | PrimitiveType::U64 | PrimitiveType::Usize | PrimitiveType::Isize => "0L",
            _ => "0",
        },
        _ => "\"\"",
    }
}
