use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::PrimitiveType;
use std::borrow::Cow;

/// TypeMapper for Zig bindings.
///
/// Maps Rust types to idiomatic Zig types:
/// - Integers map to fixed-width Zig int types (u32→u32, u64→u64, etc.)
/// - Strings/paths/JSON become sentinel-terminated byte pointers ([:0]const u8)
/// - Optionals use Zig's `?T` syntax
/// - Collections use `[]const T` for arrays and `std.StringHashMap(T)` for maps
pub struct ZigMapper;

impl TypeMapper for ZigMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        use alef_core::ir::PrimitiveType;
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("bool"),
            PrimitiveType::U8 => Cow::Borrowed("u8"),
            PrimitiveType::U16 => Cow::Borrowed("u16"),
            PrimitiveType::U32 => Cow::Borrowed("u32"),
            PrimitiveType::U64 => Cow::Borrowed("u64"),
            PrimitiveType::Usize => Cow::Borrowed("u64"),
            PrimitiveType::I8 => Cow::Borrowed("i8"),
            PrimitiveType::I16 => Cow::Borrowed("i16"),
            PrimitiveType::I32 => Cow::Borrowed("i32"),
            PrimitiveType::I64 => Cow::Borrowed("i64"),
            PrimitiveType::Isize => Cow::Borrowed("i64"),
            PrimitiveType::F32 => Cow::Borrowed("f32"),
            PrimitiveType::F64 => Cow::Borrowed("f64"),
        }
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("[:0]const u8")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("[]const u8")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("[:0]const u8")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("[:0]const u8")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("i64")
    }

    fn optional(&self, inner: &str) -> String {
        format!("?{inner}")
    }

    fn vec(&self, inner: &str) -> String {
        format!("[]const {inner}")
    }

    fn map(&self, _key: &str, value: &str) -> String {
        format!("std.StringHashMap({value})")
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
    fn test_primitive_u32() {
        assert_eq!(ZigMapper.primitive(&PrimitiveType::U32), "u32");
    }

    #[test]
    fn test_string() {
        assert_eq!(ZigMapper.string(), "[:0]const u8");
    }

    #[test]
    fn test_optional() {
        assert_eq!(ZigMapper.optional("u32"), "?u32");
    }

    #[test]
    fn test_vec() {
        assert_eq!(ZigMapper.vec("u8"), "[]const u8");
    }

    #[test]
    fn test_map_type_json() {
        assert_eq!(ZigMapper.map_type(&TypeRef::Json), "[:0]const u8");
    }

    #[test]
    fn test_optional_json() {
        assert_eq!(
            ZigMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::Json))),
            "?[:0]const u8"
        );
    }
}
