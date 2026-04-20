use std::borrow::Cow;

use alef_core::ir::{PrimitiveType, TypeRef};

/// Maps a TypeRef to its Go type representation.
/// Used for non-optional types in general contexts.
pub fn go_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Primitive(prim) => go_primitive(prim),
        TypeRef::String | TypeRef::Char => Cow::Borrowed("string"),
        TypeRef::Bytes => Cow::Borrowed("[]byte"),
        TypeRef::Optional(inner) => Cow::Owned(format!("*{}", go_type(inner))),
        TypeRef::Vec(inner) => Cow::Owned(format!("[]{}", go_type(inner))),
        TypeRef::Map(k, v) => Cow::Owned(format!("map[{}]{}", go_type(k), go_type(v))),
        TypeRef::Named(name) => Cow::Owned(name.clone()),
        TypeRef::Path => Cow::Borrowed("string"),
        TypeRef::Json => Cow::Borrowed("json.RawMessage"),
        TypeRef::Unit => Cow::Borrowed(""), // void
        TypeRef::Duration => Cow::Borrowed("uint64"),
    }
}

/// Maps a TypeRef to its optional Go type representation (pointer for option).
pub fn go_optional_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Optional(_) => go_type(ty),
        _ => Cow::Owned(format!("*{}", go_type(ty))),
    }
}

/// Maps a primitive type to its Go equivalent.
fn go_primitive(prim: &PrimitiveType) -> Cow<'static, str> {
    match prim {
        PrimitiveType::Bool => Cow::Borrowed("bool"),
        PrimitiveType::U8 => Cow::Borrowed("uint8"),
        PrimitiveType::U16 => Cow::Borrowed("uint16"),
        PrimitiveType::U32 => Cow::Borrowed("uint32"),
        PrimitiveType::U64 => Cow::Borrowed("uint64"),
        PrimitiveType::I8 => Cow::Borrowed("int8"),
        PrimitiveType::I16 => Cow::Borrowed("int16"),
        PrimitiveType::I32 => Cow::Borrowed("int32"),
        PrimitiveType::I64 => Cow::Borrowed("int64"),
        PrimitiveType::F32 => Cow::Borrowed("float32"),
        PrimitiveType::F64 => Cow::Borrowed("float64"),
        PrimitiveType::Usize => Cow::Borrowed("uint"),
        PrimitiveType::Isize => Cow::Borrowed("int"),
    }
}
