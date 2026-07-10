use std::borrow::Cow;

use crate::codegen::naming::go_type_name;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};

/// TypeMapper for Go bindings.
///
/// Maps Rust types to idiomatic Go types:
/// - Integers use Go's explicit-width types (uint8, int32, etc.)
/// - usize/isize map to uint/int (platform-native width)
/// - `Optional<T>` becomes `*T` (nullable pointer)
/// - `Vec<T>` becomes `[]T`
/// - `Map<K,V>` becomes `map[K]V`
/// - JSON becomes json.RawMessage
/// - Unit becomes "" (void in Go — no type in return position)
/// - Duration becomes uint64 (milliseconds)
pub struct GoMapper;

impl TypeMapper for GoMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "uint8",
            PrimitiveType::U16 => "uint16",
            PrimitiveType::U32 => "uint32",
            PrimitiveType::U64 => "uint64",
            PrimitiveType::I8 => "int8",
            PrimitiveType::I16 => "int16",
            PrimitiveType::I32 => "int32",
            PrimitiveType::I64 => "int64",
            PrimitiveType::F32 => "float32",
            PrimitiveType::F64 => "float64",
            PrimitiveType::Usize => "uint",
            PrimitiveType::Isize => "int",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("[]byte")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("json.RawMessage")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("uint64")
    }

    fn optional(&self, inner: &str) -> String {
        format!("*{inner}")
    }

    fn vec(&self, inner: &str) -> String {
        format!("[]{inner}")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("map[{key}]{value}")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(go_type_name(name))
    }

    fn error_wrapper(&self) -> &str {
        "error"
    }
}

/// Maps a TypeRef to its Go type representation.
/// Used for non-optional types in general contexts.
///
/// Delegates to [`GoMapper`] for exhaustive TypeRef handling.
pub fn go_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(GoMapper.map_type(ty))
}

/// Maps a TypeRef to its optional Go type representation (pointer for option).
///
/// If the type is already `Optional`, delegates to `go_type` (which produces `*T`).
/// Slices (`Vec<T>`, `Bytes`) and maps are already reference types in Go — they
/// are not wrapped in a pointer because `*[]T` and `*map[K]V` are unidiomatic
/// and unnecessary.
/// String types (String, Char, Path) are wrapped in pointer: `*string`.
/// All other non-reference types are wrapped in a pointer: `*T`.
pub fn go_optional_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Optional(_) | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes => go_type(ty),
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Named(_)
        | TypeRef::Primitive(_)
        | TypeRef::Duration
        | TypeRef::Unit => Cow::Owned(format!("*{}", GoMapper.map_type(ty))),
    }
}

/// Returns the Go zero-value expression for a return-type, used in `return <zero>, fmt.Errorf(...)`
/// early exits.
///
/// Must stay in sync with the return-signature logic in `gen_bindings::methods` and
/// `gen_bindings::functions`: scalar primitives and Duration stay as value types and
/// need an explicit zero literal (`0`, `false`); scalar types (String, Char, Path, Json)
/// also stay as value types and use empty string `""`; everything else (Named, Vec, Map,
/// Bytes, Optional) is emitted as a pointer or reference type whose zero is `nil`.
pub fn go_zero_value(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => "false".to_string(),
        TypeRef::Primitive(_) | TypeRef::Duration => "0".to_string(),
        TypeRef::String | TypeRef::Char | TypeRef::Path => "\"\"".to_string(),
        TypeRef::Json => "nil".to_string(),
        TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _)
        | TypeRef::Optional(_)
        | TypeRef::Named(_)
        | TypeRef::Unit => "nil".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitives() {
        let m = GoMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "bool");
        assert_eq!(m.primitive(&PrimitiveType::U8), "uint8");
        assert_eq!(m.primitive(&PrimitiveType::U16), "uint16");
        assert_eq!(m.primitive(&PrimitiveType::U32), "uint32");
        assert_eq!(m.primitive(&PrimitiveType::U64), "uint64");
        assert_eq!(m.primitive(&PrimitiveType::I8), "int8");
        assert_eq!(m.primitive(&PrimitiveType::I16), "int16");
        assert_eq!(m.primitive(&PrimitiveType::I32), "int32");
        assert_eq!(m.primitive(&PrimitiveType::I64), "int64");
        assert_eq!(m.primitive(&PrimitiveType::F32), "float32");
        assert_eq!(m.primitive(&PrimitiveType::F64), "float64");
        assert_eq!(m.primitive(&PrimitiveType::Usize), "uint");
        assert_eq!(m.primitive(&PrimitiveType::Isize), "int");
    }

    #[test]
    fn test_string_and_char() {
        assert_eq!(GoMapper.map_type(&TypeRef::String), "string");
        assert_eq!(GoMapper.map_type(&TypeRef::Char), "string");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(GoMapper.map_type(&TypeRef::Bytes), "[]byte");
    }

    #[test]
    fn test_path() {
        assert_eq!(GoMapper.map_type(&TypeRef::Path), "string");
    }

    #[test]
    fn test_json() {
        assert_eq!(GoMapper.map_type(&TypeRef::Json), "json.RawMessage");
    }

    #[test]
    fn test_unit() {
        assert_eq!(GoMapper.map_type(&TypeRef::Unit), "");
    }

    #[test]
    fn test_duration() {
        assert_eq!(GoMapper.map_type(&TypeRef::Duration), "uint64");
    }

    #[test]
    fn test_optional() {
        assert_eq!(
            GoMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "*string"
        );
    }

    #[test]
    fn test_vec() {
        assert_eq!(GoMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))), "[]string");
    }

    #[test]
    fn test_map() {
        assert_eq!(
            GoMapper.map_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32))
            )),
            "map[string]int32"
        );
    }

    #[test]
    fn test_go_type_delegate() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
        assert_eq!(go_type(&ty).as_ref(), GoMapper.map_type(&ty));
    }

    #[test]
    fn test_go_optional_type_already_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), go_type(&ty));
    }

    #[test]
    fn test_go_optional_type_non_optional() {
        assert_eq!(go_optional_type(&TypeRef::String), "*string");
    }

    #[test]
    fn test_go_optional_type_vec_not_pointer() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), "[]string");
    }

    #[test]
    fn test_go_optional_type_bytes_not_pointer() {
        assert_eq!(go_optional_type(&TypeRef::Bytes), "[]byte");
    }

    #[test]
    fn test_go_optional_type_map_not_pointer() {
        let ty = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(go_optional_type(&ty), "map[string]string");
    }
}
