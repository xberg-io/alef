use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::PrimitiveType;
use std::borrow::Cow;

/// TypeMapper for Gleam bindings.
///
/// Maps Rust types to idiomatic Gleam types:
/// - Most integers map to Gleam's `Int` (arbitrary precision)
/// - Floats map to Gleam's `Float`
/// - Booleans map to `Bool`
/// - Strings, paths, JSON become `String`
/// - Bytes become `BitArray`
/// - Optionals use Gleam's `Option(T)` type
/// - Collections use `List(T)` for vectors and `Dict(K, V)` for maps
pub struct GleamMapper;

impl TypeMapper for GleamMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        use alef_core::ir::PrimitiveType;
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("Bool"),
            PrimitiveType::F32 | PrimitiveType::F64 => Cow::Borrowed("Float"),
            _ => Cow::Borrowed("Int"), // All integer types map to Int
        }
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("BitArray")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("Nil")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Int")
    }

    fn optional(&self, inner: &str) -> String {
        format!("Option({inner})")
    }

    fn vec(&self, inner: &str) -> String {
        format!("List({inner})")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Dict({key}, {value})")
    }

    fn error_wrapper(&self) -> &str {
        "Result"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::TypeRef;

    #[test]
    fn test_primitive_bool() {
        assert_eq!(GleamMapper.primitive(&PrimitiveType::Bool), "Bool");
    }

    #[test]
    fn test_primitive_int() {
        assert_eq!(GleamMapper.primitive(&PrimitiveType::U32), "Int");
        assert_eq!(GleamMapper.primitive(&PrimitiveType::I64), "Int");
    }

    #[test]
    fn test_primitive_float() {
        assert_eq!(GleamMapper.primitive(&PrimitiveType::F32), "Float");
        assert_eq!(GleamMapper.primitive(&PrimitiveType::F64), "Float");
    }

    #[test]
    fn test_string() {
        assert_eq!(GleamMapper.string(), "String");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(GleamMapper.bytes(), "BitArray");
    }

    #[test]
    fn test_optional() {
        assert_eq!(GleamMapper.optional("Int"), "Option(Int)");
    }

    #[test]
    fn test_vec() {
        assert_eq!(GleamMapper.vec("String"), "List(String)");
    }

    #[test]
    fn test_map_type_json() {
        assert_eq!(GleamMapper.map_type(&TypeRef::Json), "String");
    }

    #[test]
    fn test_optional_string() {
        assert_eq!(
            GleamMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "Option(String)"
        );
    }

    #[test]
    fn test_vec_int() {
        assert_eq!(
            GleamMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32)))),
            "List(Int)"
        );
    }
}
