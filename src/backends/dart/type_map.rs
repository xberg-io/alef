use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use std::borrow::Cow;

/// TypeMapper for Dart bindings.
///
/// Dart has a unified `int` type for all integer widths (both signed and
/// unsigned), `double` for all floating-point, and `Uint8List` for raw byte
/// arrays (from `dart:typed_data`). Optional types use the Dart nullable
/// suffix `?`. The `error_wrapper` returns `"Result"` as a placeholder for
/// the sealed-class result type that lands in Phase 2B.
pub struct DartMapper;

impl TypeMapper for DartMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        match prim {
            PrimitiveType::Bool => Cow::Borrowed("bool"),
            PrimitiveType::U8
            | PrimitiveType::I8
            | PrimitiveType::U16
            | PrimitiveType::I16
            | PrimitiveType::U32
            | PrimitiveType::I32
            | PrimitiveType::U64
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => Cow::Borrowed("int"),
            PrimitiveType::F32 | PrimitiveType::F64 => Cow::Borrowed("double"),
        }
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("Uint8List")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Duration")
    }

    fn optional(&self, inner: &str) -> String {
        format!("{inner}?")
    }

    fn vec(&self, inner: &str) -> String {
        format!("List<{inner}>")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Map<{key}, {value}>")
    }

    /// Override the default Vec dispatch so `Vec<T>` with a primitive `T`
    /// maps to the matching `dart:typed_data` typed list — matches FRB's
    /// automatic Rust → Dart mapping (`Vec<f64>` → `Float64List`,
    /// `Vec<u8>` → `Uint8List`, …). User trait implementations and e2e
    /// fixture stubs must surface these typed names to satisfy the
    /// FRB-generated `create_*_dart_impl` factory signatures.
    fn map_type(&self, ty: &TypeRef) -> String {
        if let TypeRef::Vec(inner) = ty {
            if let Some(typed) = dart_typed_list_for(inner) {
                return typed.to_string();
            }
        }
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
        "Result"
    }
}

/// Map `Vec<primitive>` to the matching `dart:typed_data` typed list, mirroring
/// the FRB widening alef applies in `gen_rust_crate`: every Rust integer is
/// widened to `i64` (→ `Int64List`) and every float to `f64` (→ `Float64List`).
/// `Vec<u8>` is special-cased to `Uint8List` because alef preserves byte arrays
/// for FRB's binary fast path. Non-primitive elements fall back to `List<T>`.
fn dart_typed_list_for(inner: &TypeRef) -> Option<&'static str> {
    if let TypeRef::Primitive(p) = inner {
        match p {
            PrimitiveType::U8 => Some("Uint8List"),
            PrimitiveType::I8
            | PrimitiveType::U16
            | PrimitiveType::I16
            | PrimitiveType::U32
            | PrimitiveType::I32
            | PrimitiveType::U64
            | PrimitiveType::I64
            | PrimitiveType::Usize
            | PrimitiveType::Isize => Some("Int64List"),
            PrimitiveType::F32 | PrimitiveType::F64 => Some("Float64List"),
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::TypeRef;

    #[test]
    fn test_primitive_bool() {
        assert_eq!(DartMapper.primitive(&PrimitiveType::Bool), "bool");
    }

    #[test]
    fn test_primitive_integers_all_map_to_int() {
        assert_eq!(DartMapper.primitive(&PrimitiveType::U8), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::I8), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::U16), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::I16), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::U32), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::I32), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::U64), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::I64), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::Usize), "int");
        assert_eq!(DartMapper.primitive(&PrimitiveType::Isize), "int");
    }

    #[test]
    fn test_primitive_floats_map_to_double() {
        assert_eq!(DartMapper.primitive(&PrimitiveType::F32), "double");
        assert_eq!(DartMapper.primitive(&PrimitiveType::F64), "double");
    }

    #[test]
    fn test_string() {
        assert_eq!(DartMapper.string(), "String");
    }

    #[test]
    fn test_bytes() {
        assert_eq!(DartMapper.bytes(), "Uint8List");
    }

    #[test]
    fn test_path_maps_to_string() {
        assert_eq!(DartMapper.path(), "String");
    }

    #[test]
    fn test_json_maps_to_string() {
        assert_eq!(DartMapper.map_type(&TypeRef::Json), "String");
    }

    #[test]
    fn test_unit() {
        assert_eq!(DartMapper.unit(), "void");
    }

    #[test]
    fn test_duration() {
        assert_eq!(DartMapper.map_type(&TypeRef::Duration), "Duration");
    }

    #[test]
    fn test_optional() {
        assert_eq!(DartMapper.optional("String"), "String?");
    }

    #[test]
    fn test_optional_via_map_type() {
        assert_eq!(
            DartMapper.map_type(&TypeRef::Optional(Box::new(TypeRef::String))),
            "String?"
        );
    }

    #[test]
    fn test_vec() {
        assert_eq!(DartMapper.vec("int"), "List<int>");
    }

    #[test]
    fn test_map() {
        assert_eq!(DartMapper.map("String", "int"), "Map<String, int>");
    }
}
