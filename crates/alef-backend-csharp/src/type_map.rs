use std::borrow::Cow;

use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{PrimitiveType, TypeRef};
use heck::ToPascalCase;

/// TypeMapper for C# bindings.
///
/// Maps Rust types to idiomatic C# types:
/// - Integers map to the corresponding C# primitive (bool, byte, ushort, etc.)
/// - Optional<T> becomes T? (nullable)
/// - Vec<T> becomes List<T>
/// - Map<K,V> becomes Dictionary<K,V>
/// - Duration maps to ulong? (milliseconds, nullable sentinel)
pub struct CsharpMapper;

impl TypeMapper for CsharpMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "bool",
            PrimitiveType::U8 => "byte",
            PrimitiveType::U16 => "ushort",
            PrimitiveType::U32 => "uint",
            PrimitiveType::U64 => "ulong",
            PrimitiveType::I8 => "sbyte",
            PrimitiveType::I16 => "short",
            PrimitiveType::I32 => "int",
            PrimitiveType::I64 => "long",
            PrimitiveType::F32 => "float",
            PrimitiveType::F64 => "double",
            PrimitiveType::Usize => "ulong",
            PrimitiveType::Isize => "long",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("byte[]")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("string")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("object")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("ulong?")
    }

    fn optional(&self, inner: &str) -> String {
        format!("{inner}?")
    }

    fn vec(&self, inner: &str) -> String {
        format!("List<{inner}>")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Dictionary<{key}, {value}>")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(name.to_pascal_case())
    }

    fn error_wrapper(&self) -> &str {
        "Task"
    }
}

/// Maps a TypeRef to its C# type representation.
///
/// Delegates to [`CsharpMapper`] for exhaustive TypeRef handling.
pub fn csharp_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(CsharpMapper.map_type(ty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitives() {
        let m = CsharpMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "bool");
        assert_eq!(m.primitive(&PrimitiveType::U8), "byte");
        assert_eq!(m.primitive(&PrimitiveType::U16), "ushort");
        assert_eq!(m.primitive(&PrimitiveType::U32), "uint");
        assert_eq!(m.primitive(&PrimitiveType::U64), "ulong");
        assert_eq!(m.primitive(&PrimitiveType::I8), "sbyte");
        assert_eq!(m.primitive(&PrimitiveType::I16), "short");
        assert_eq!(m.primitive(&PrimitiveType::I32), "int");
        assert_eq!(m.primitive(&PrimitiveType::I64), "long");
        assert_eq!(m.primitive(&PrimitiveType::F32), "float");
        assert_eq!(m.primitive(&PrimitiveType::F64), "double");
        assert_eq!(m.primitive(&PrimitiveType::Usize), "ulong");
        assert_eq!(m.primitive(&PrimitiveType::Isize), "long");
    }

    #[test]
    fn test_string_and_char() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::String), "string");
        assert_eq!(CsharpMapper.map_type(&TypeRef::Char), "string");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Bytes), "byte[]");
    }

    #[test]
    fn test_path() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Path), "string");
    }

    #[test]
    fn test_json() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Json), "object");
    }

    #[test]
    fn test_unit() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Unit), "void");
    }

    #[test]
    fn test_duration() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Duration), "ulong?");
    }

    #[test]
    fn test_optional() {
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "string?"
        );
    }

    #[test]
    fn test_optional_duration() {
        // Duration is already ulong? — Optional<Duration> becomes ulong??
        // This is consistent with what the old free function produced.
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::Duration))),
            "ulong??"
        );
    }

    #[test]
    fn test_vec() {
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))),
            "List<string>"
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::I32))
            )),
            "Dictionary<string, int>"
        );
    }

    #[test]
    fn test_named() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Named("MyType".to_string())), "MyType");
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Named("GraphQLRouteConfig".to_string())),
            "GraphQlRouteConfig"
        );
    }

    #[test]
    fn test_csharp_type_delegate() {
        // Ensure the free function produces the same output as the mapper
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
        assert_eq!(csharp_type(&ty).as_ref(), CsharpMapper.map_type(&ty));
    }
}
