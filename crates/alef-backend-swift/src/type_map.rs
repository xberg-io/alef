use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::PrimitiveType;
use std::borrow::Cow;

/// TypeMapper for Swift bindings.
///
/// Maps Rust/alef IR types to their idiomatic Swift equivalents.
/// Unsigned integers retain distinct types (Swift has native unsigned support),
/// paths map to `URL`, bytes to `Data`, and durations to `Duration` (Swift 5.7+).
pub struct SwiftMapper;

impl TypeMapper for SwiftMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("Bool"),
            PrimitiveType::U8 => Cow::Borrowed("UInt8"),
            PrimitiveType::I8 => Cow::Borrowed("Int8"),
            PrimitiveType::U16 => Cow::Borrowed("UInt16"),
            PrimitiveType::I16 => Cow::Borrowed("Int16"),
            PrimitiveType::U32 => Cow::Borrowed("UInt32"),
            PrimitiveType::I32 => Cow::Borrowed("Int32"),
            PrimitiveType::U64 => Cow::Borrowed("UInt64"),
            PrimitiveType::I64 => Cow::Borrowed("Int64"),
            PrimitiveType::Usize => Cow::Borrowed("UInt"),
            PrimitiveType::Isize => Cow::Borrowed("Int"),
            PrimitiveType::F32 => Cow::Borrowed("Float"),
            PrimitiveType::F64 => Cow::Borrowed("Double"),
        }
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("Data")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("URL")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("Void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Duration")
    }

    fn optional(&self, inner: &str) -> String {
        format!("{inner}?")
    }

    fn vec(&self, inner: &str) -> String {
        format!("[{inner}]")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("[{key}: {value}]")
    }

    fn error_wrapper(&self) -> &str {
        // Swift has a native `Result<Success, Failure>`; Stage 2B emits the
        // fully-parameterised form and replaces this placeholder.
        "Result"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::TypeRef;

    #[test]
    fn test_primitive_bool() {
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::Bool), "Bool");
    }

    #[test]
    fn test_primitive_unsigned() {
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::U8), "UInt8");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::U16), "UInt16");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::U32), "UInt32");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::U64), "UInt64");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::Usize), "UInt");
    }

    #[test]
    fn test_primitive_signed() {
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::I8), "Int8");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::I16), "Int16");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::I32), "Int32");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::I64), "Int64");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::Isize), "Int");
    }

    #[test]
    fn test_primitive_float() {
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::F32), "Float");
        assert_eq!(SwiftMapper.primitive(&PrimitiveType::F64), "Double");
    }

    #[test]
    fn test_string() {
        assert_eq!(SwiftMapper.string(), "String");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(SwiftMapper.bytes(), "Data");
    }

    #[test]
    fn test_path() {
        assert_eq!(SwiftMapper.path(), "URL");
    }

    #[test]
    fn test_json() {
        assert_eq!(SwiftMapper.map_type(&TypeRef::Json), "String");
    }

    #[test]
    fn test_unit() {
        assert_eq!(SwiftMapper.map_type(&TypeRef::Unit), "Void");
    }

    #[test]
    fn test_duration() {
        assert_eq!(SwiftMapper.map_type(&TypeRef::Duration), "Duration");
    }

    #[test]
    fn test_optional() {
        assert_eq!(SwiftMapper.optional("String"), "String?");
    }

    #[test]
    fn test_optional_map_type() {
        assert_eq!(
            SwiftMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "String?"
        );
    }

    #[test]
    fn test_vec() {
        assert_eq!(SwiftMapper.vec("Int32"), "[Int32]");
    }

    #[test]
    fn test_vec_map_type() {
        assert_eq!(
            SwiftMapper.map_type(&TypeRef::Vec(Box::new(TypeRef::String))),
            "[String]"
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(SwiftMapper.map("String", "Int32"), "[String: Int32]");
    }

    #[test]
    fn test_map_map_type() {
        assert_eq!(
            SwiftMapper.map_type(&TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String))),
            "[String: String]"
        );
    }
}
