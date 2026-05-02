//! Primitive type mapping helpers used by vtable code generation.

use alef_core::ir::{PrimitiveType, TypeRef};

/// Map a `PrimitiveType` to its C-compatible Rust type name.
pub(crate) fn prim_to_c(p: &PrimitiveType) -> &'static str {
    match p {
        PrimitiveType::Bool => "i32", // C bool is int
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    }
}

/// Return the Rust default-value expression for a `TypeRef`.
pub(crate) fn default_for_type(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Unit => "()",
        TypeRef::String | TypeRef::Char | TypeRef::Path => "String::new()",
        TypeRef::Bytes => "Vec::new()",
        TypeRef::Primitive(p) => match p {
            PrimitiveType::Bool => "false",
            PrimitiveType::F32 | PrimitiveType::F64 => "0.0",
            _ => "0",
        },
        TypeRef::Optional(_) => "None",
        TypeRef::Vec(_) => "Vec::new()",
        TypeRef::Map(_, _) => "std::collections::HashMap::new()",
        TypeRef::Json => "serde_json::Value::Null",
        TypeRef::Duration => "std::time::Duration::ZERO",
        TypeRef::Named(_) => "Default::default()",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prim_to_c_bool_is_i32() {
        assert_eq!(prim_to_c(&PrimitiveType::Bool), "i32");
    }

    #[test]
    fn prim_to_c_numeric_types() {
        assert_eq!(prim_to_c(&PrimitiveType::U8), "u8");
        assert_eq!(prim_to_c(&PrimitiveType::I64), "i64");
        assert_eq!(prim_to_c(&PrimitiveType::F32), "f32");
        assert_eq!(prim_to_c(&PrimitiveType::Usize), "usize");
    }

    #[test]
    fn default_for_type_primitives() {
        assert_eq!(default_for_type(&TypeRef::Unit), "()");
        assert_eq!(default_for_type(&TypeRef::String), "String::new()");
        assert_eq!(default_for_type(&TypeRef::Bytes), "Vec::new()");
        assert_eq!(default_for_type(&TypeRef::Primitive(PrimitiveType::Bool)), "false");
        assert_eq!(default_for_type(&TypeRef::Primitive(PrimitiveType::F64)), "0.0");
        assert_eq!(default_for_type(&TypeRef::Primitive(PrimitiveType::I32)), "0");
    }

    #[test]
    fn default_for_type_complex() {
        assert_eq!(default_for_type(&TypeRef::Optional(Box::new(TypeRef::String))), "None");
        assert_eq!(default_for_type(&TypeRef::Vec(Box::new(TypeRef::String))), "Vec::new()");
        assert_eq!(default_for_type(&TypeRef::Duration), "std::time::Duration::ZERO");
    }
}
