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
        // ExtractionResult and other DTOs round-trip via System.Text.Json. The Rust core
        // emits nested JSON for any field typed `JsonValue`, so the C# DTO must declare
        // it as `JsonElement` to accept the wire format. Declaring it as `string` causes
        // a deserialization failure ("Cannot get the value of a token type 'StartObject'
        // as a string") when the Rust side embeds an object rather than a stringified one.
        Cow::Borrowed("JsonElement")
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

    fn map_type(&self, ty: &TypeRef) -> String {
        // Special case: Map<String, Json> should become Dictionary<string, JsonElement>
        // to handle numeric and array values in the JSON dict during deserialization.
        if let TypeRef::Map(k, v) = ty {
            if matches!(v.as_ref(), TypeRef::Json) {
                let key_str = match k.as_ref() {
                    TypeRef::Primitive(p) => self.primitive(p).into_owned(),
                    TypeRef::String | TypeRef::Char => self.string().into_owned(),
                    TypeRef::Bytes => self.bytes().into_owned(),
                    TypeRef::Path => self.path().into_owned(),
                    TypeRef::Json => self.json().into_owned(),
                    TypeRef::Unit => self.unit().into_owned(),
                    TypeRef::Optional(inner) => self.optional(&self.map_type(inner)),
                    TypeRef::Vec(inner) => self.vec(&self.map_type(inner)),
                    TypeRef::Named(name) => self.named(name).into_owned(),
                    TypeRef::Duration => self.duration().into_owned(),
                    // Nested Map in key is unlikely but handle recursively
                    TypeRef::Map(kk, vv) => {
                        let kk_str = self.map_type(kk);
                        let vv_str = self.map_type(vv);
                        format!("Dictionary<{kk_str}, {vv_str}>")
                    }
                };
                return format!("Dictionary<{key_str}, JsonElement>");
            }
        }
        // Fall back to default implementation for all other types
        match ty {
            TypeRef::Primitive(p) => self.primitive(p).into_owned(),
            TypeRef::String | TypeRef::Char => self.string().into_owned(),
            TypeRef::Bytes => self.bytes().into_owned(),
            TypeRef::Path => self.path().into_owned(),
            TypeRef::Json => self.json().into_owned(),
            TypeRef::Unit => self.unit().into_owned(),
            TypeRef::Optional(inner) => self.optional(&self.map_type(inner)),
            TypeRef::Vec(inner) => self.vec(&self.map_type(inner)),
            TypeRef::Map(k, v) => self.map(&self.map_type(k), &self.map_type(v)),
            TypeRef::Named(name) => self.named(name).into_owned(),
            TypeRef::Duration => self.duration().into_owned(),
        }
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
        assert_eq!(CsharpMapper.map_type(&TypeRef::Json), "JsonElement");
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
    fn test_map_with_json_value() {
        // Map<String, Json> should map to Dictionary<string, JsonElement>
        // to properly handle numeric and array values during deserialization
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::Json))),
            "Dictionary<string, JsonElement>"
        );
    }

    #[test]
    fn test_named() {
        assert_eq!(CsharpMapper.map_type(&TypeRef::Named("MyType".to_string())), "MyType");
        // IR type names in PascalCase are preserved with initialism uppercasing.
        assert_eq!(
            CsharpMapper.map_type(&TypeRef::Named("GraphQLRouteConfig".to_string())),
            "GraphQLRouteConfig"
        );
        // heck-corrupted input is restored to canonical initialism form.
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
        // Ensure the free function produces the same output as the mapper
        let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)));
        assert_eq!(csharp_type(&ty).as_ref(), CsharpMapper.map_type(&ty));
    }
}
