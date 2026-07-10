use std::borrow::Cow;

use crate::codegen::naming::csharp_type_name;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};

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
        Cow::Borrowed("string")
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
        Cow::Owned(csharp_type_name(name))
    }

    fn error_wrapper(&self) -> &str {
        "Task"
    }
}

/// Maps a TypeRef to its C# type representation for FFI parameters/returns.
///
/// Uses `string` for Json types to match P/Invoke marshalling requirements.
/// Delegates to [`CsharpMapper`] for exhaustive TypeRef handling.
pub fn csharp_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(CsharpMapper.map_type(ty))
}

/// Maps a TypeRef to its C# type representation for DTO fields.
///
/// Uses `JsonElement` for Json types to properly deserialize embedded objects
/// via System.Text.Json. This avoids the "Cannot get the value of a token type
/// 'StartObject' as a string" error when Rust embeds a JSON object.
pub fn csharp_type_for_dto_field(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Json => Cow::Borrowed("JsonElement"),
        TypeRef::Map(k, v) if matches!(v.as_ref(), TypeRef::Json) => {
            let key_type = csharp_type(k);
            Cow::Owned(format!("Dictionary<{}, JsonElement>", key_type))
        }
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Json) => Cow::Borrowed("JsonElement?"),
        _ => csharp_type(ty),
    }
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
        assert_eq!(CsharpMapper.map_type(&TypeRef::Json), "string");
    }

    #[test]
    fn test_json_for_dto_field() {
        assert_eq!(csharp_type_for_dto_field(&TypeRef::Json), "JsonElement");
    }

    #[test]
    fn test_map_json_for_dto_field() {
        let map_type = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json));
        assert_eq!(csharp_type_for_dto_field(&map_type), "Dictionary<string, JsonElement>");
    }

    #[test]
    fn test_optional_json_for_dto_field() {
        let opt_type = TypeRef::Optional(Box::new(TypeRef::Json));
        assert_eq!(csharp_type_for_dto_field(&opt_type), "JsonElement?");
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
            "GraphQLRouteConfig"
        );
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Named("GraphQlRouteConfig".to_string())),
            "GraphQLRouteConfig"
        );
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Named("HttpStatus".to_string())),
            "HttpStatus"
        );
    }

    #[test]
    fn test_csharp_type_delegate() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
        assert_eq!(csharp_type(&ty).as_ref(), CsharpMapper.map_type(&ty));
    }
}
