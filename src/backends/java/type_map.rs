use std::borrow::Cow;

use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};

/// TypeMapper for Java bindings — unboxed (primitive) types.
///
/// Maps Rust types to Java types with primitive numerics (boolean, byte, int, long, etc.).
/// Optional<T> unwraps to the inner boxed type (Java unboxed convention at FFI boundary).
/// Vec<T> becomes List<T>; Map<K,V> becomes Map<K,V> with boxed generics.
pub struct JavaMapper;

impl TypeMapper for JavaMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "boolean",
            PrimitiveType::U8 | PrimitiveType::I8 => "byte",
            PrimitiveType::U16 | PrimitiveType::I16 => "short",
            PrimitiveType::U32 | PrimitiveType::I32 => "int",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "long",
            PrimitiveType::F32 => "float",
            PrimitiveType::F64 => "double",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("byte[]")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("java.nio.file.Path")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("Object")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Long")
    }

    /// Java unwraps Optional<T> to the inner boxed type at the FFI boundary.
    fn optional(&self, inner: &str) -> String {
        inner.to_string()
    }

    fn vec(&self, inner: &str) -> String {
        format!("List<{inner}>")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Map<{key}, {value}>")
    }

    /// Override map_type to use boxed types inside Vec/Map generics.
    ///
    /// Java requires boxed types as generic type parameters (List<Integer> not List<int>).
    /// The default map_type would pass primitive names as the inner string to vec/map,
    /// producing invalid Java like List<int>. Override to use JavaBoxedMapper for inner types.
    fn map_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => self.primitive(p).into_owned(),
            TypeRef::String | TypeRef::Char => self.string().into_owned(),
            TypeRef::Bytes => self.bytes().into_owned(),
            TypeRef::Path => self.path().into_owned(),
            TypeRef::Json => self.json().into_owned(),
            TypeRef::Unit => self.unit().into_owned(),
            TypeRef::Duration => self.duration().into_owned(),
            TypeRef::Named(name) => self.named(name).into_owned(),
            TypeRef::Optional(inner) => JavaBoxedMapper.map_type(inner),
            TypeRef::Vec(inner) => format!("List<{}>", JavaBoxedMapper.map_type(inner)),
            TypeRef::Map(k, v) => {
                format!("Map<{}, {}>", JavaBoxedMapper.map_type(k), JavaBoxedMapper.map_type(v))
            }
        }
    }

    fn error_wrapper(&self) -> &str {
        "CompletableFuture"
    }
}

/// TypeMapper for Java bindings — boxed (reference) types.
///
/// Maps Rust types to Java boxed types suitable for use as generic type parameters
/// (Boolean, Integer, Long, etc. rather than boolean, int, long).
/// Used for Optional<T>, Vec<T>, and Map<K,V> inner types.
pub struct JavaBoxedMapper;

impl TypeMapper for JavaBoxedMapper {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        Cow::Borrowed(match prim {
            PrimitiveType::Bool => "Boolean",
            PrimitiveType::U8 | PrimitiveType::I8 => "Byte",
            PrimitiveType::U16 | PrimitiveType::I16 => "Short",
            PrimitiveType::U32 | PrimitiveType::I32 => "Integer",
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => "Long",
            PrimitiveType::F32 => "Float",
            PrimitiveType::F64 => "Double",
        })
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("String")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("byte[]")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("java.nio.file.Path")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("Object")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("Void")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("Long")
    }

    /// Optional<T> in boxed context unwraps to the inner boxed type.
    fn optional(&self, inner: &str) -> String {
        inner.to_string()
    }

    fn vec(&self, inner: &str) -> String {
        format!("List<{inner}>")
    }

    fn map(&self, key: &str, value: &str) -> String {
        format!("Map<{key}, {value}>")
    }

    fn error_wrapper(&self) -> &str {
        "CompletableFuture"
    }
}

/// Maps a TypeRef to its Java type representation.
///
/// Delegates to [`JavaMapper`] for exhaustive TypeRef handling.
/// Optional<T> unwraps to the inner boxed type (FFI boundary convention).
pub fn java_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(JavaMapper.map_type(ty))
}

/// Maps a TypeRef to its Java boxed type (for Optional/null-safe contexts).
///
/// Delegates to [`JavaBoxedMapper`] for exhaustive TypeRef handling.
pub fn java_boxed_type(ty: &TypeRef) -> Cow<'static, str> {
    Cow::Owned(JavaBoxedMapper.map_type(ty))
}

/// Maps a TypeRef to its Java type representation for function return types.
/// Unlike `java_type`, this preserves Optional<T> as Optional<T> instead of unwrapping.
pub fn java_return_type(ty: &TypeRef) -> Cow<'static, str> {
    match ty {
        TypeRef::Optional(inner) => Cow::Owned(format!("Optional<{}>", java_boxed_type(inner))),
        other => java_type(other),
    }
}

/// Maps a primitive type to its Java FFI equivalent (Panama FFM ValueLayout).
///
/// All integer primitives are promoted to JAVA_LONG to avoid `ClassCastException`
/// on JetBrains Runtime (JBR) Windows 64-bit Panama linker. JBR's linker cannot handle
/// sub-64-bit integer layouts (`JAVA_BYTE`, `JAVA_SHORT`, `JAVA_INT`) in `FunctionDescriptor`
/// and throws: `ClassCastException: ValueLayouts$OfIntImpl cannot be cast to ValueLayout$OfLong`.
///
/// `MethodHandle.invoke()` adapts the wider value back to the narrower Java type at the
/// call site via `asType()`, which is allowed for numeric primitive narrowing.
pub fn java_ffi_type(prim: &PrimitiveType) -> &'static str {
    match prim {
        PrimitiveType::Bool => "ValueLayout.JAVA_LONG",
        PrimitiveType::U8 | PrimitiveType::I8 => "ValueLayout.JAVA_LONG",
        PrimitiveType::U16 | PrimitiveType::I16 => "ValueLayout.JAVA_LONG",
        PrimitiveType::U32 | PrimitiveType::I32 => "ValueLayout.JAVA_LONG",
        PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
            "ValueLayout.JAVA_LONG"
        }
        PrimitiveType::F32 => "ValueLayout.JAVA_FLOAT",
        PrimitiveType::F64 => "ValueLayout.JAVA_DOUBLE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_mapper_primitives() {
        let m = JavaMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "boolean");
        assert_eq!(m.primitive(&PrimitiveType::U8), "byte");
        assert_eq!(m.primitive(&PrimitiveType::I8), "byte");
        assert_eq!(m.primitive(&PrimitiveType::U16), "short");
        assert_eq!(m.primitive(&PrimitiveType::I16), "short");
        assert_eq!(m.primitive(&PrimitiveType::U32), "int");
        assert_eq!(m.primitive(&PrimitiveType::I32), "int");
        assert_eq!(m.primitive(&PrimitiveType::U64), "long");
        assert_eq!(m.primitive(&PrimitiveType::I64), "long");
        assert_eq!(m.primitive(&PrimitiveType::Usize), "long");
        assert_eq!(m.primitive(&PrimitiveType::Isize), "long");
        assert_eq!(m.primitive(&PrimitiveType::F32), "float");
        assert_eq!(m.primitive(&PrimitiveType::F64), "double");
    }

    #[test]
    fn java_boxed_mapper_primitives() {
        let m = JavaBoxedMapper;
        assert_eq!(m.primitive(&PrimitiveType::Bool), "Boolean");
        assert_eq!(m.primitive(&PrimitiveType::U8), "Byte");
        assert_eq!(m.primitive(&PrimitiveType::U32), "Integer");
        assert_eq!(m.primitive(&PrimitiveType::U64), "Long");
        assert_eq!(m.primitive(&PrimitiveType::F32), "Float");
        assert_eq!(m.primitive(&PrimitiveType::F64), "Double");
    }

    #[test]
    fn java_type_string() {
        assert_eq!(java_type(&TypeRef::String), "String");
        assert_eq!(java_type(&TypeRef::Char), "String");
    }

    #[test]
    fn java_type_bytes() {
        assert_eq!(java_type(&TypeRef::Bytes), "byte[]");
    }

    #[test]
    fn java_type_path() {
        assert_eq!(java_type(&TypeRef::Path), "java.nio.file.Path");
    }

    #[test]
    fn java_type_json() {
        assert_eq!(java_type(&TypeRef::Json), "Object");
    }

    #[test]
    fn java_type_unit() {
        assert_eq!(java_type(&TypeRef::Unit), "void");
    }

    #[test]
    fn java_type_duration() {
        assert_eq!(java_type(&TypeRef::Duration), "Long");
    }

    #[test]
    fn java_type_vec_uses_boxed_inner() {
        assert_eq!(
            java_type(&TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::I32)))),
            "List<Integer>"
        );
    }

    #[test]
    fn java_type_map_uses_boxed_keys_and_values() {
        assert_eq!(
            java_type(&TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::U32))
            )),
            "Map<String, Integer>"
        );
    }

    #[test]
    fn java_type_unwraps_optional() {
        assert_eq!(java_type(&TypeRef::Optional(Box::new(TypeRef::String))), "String");
    }

    #[test]
    fn java_boxed_type_unit_is_void_class() {
        assert_eq!(java_boxed_type(&TypeRef::Unit), "Void");
    }

    #[test]
    fn java_return_type_wraps_optional_string() {
        let ty = TypeRef::Optional(Box::new(TypeRef::String));
        assert_eq!(java_return_type(&ty), "Optional<String>");
    }

    #[test]
    fn java_return_type_wraps_optional_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("EmbeddingPreset".to_string())));
        assert_eq!(java_return_type(&ty), "Optional<EmbeddingPreset>");
    }

    #[test]
    fn java_return_type_wraps_optional_vec() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
        assert_eq!(java_return_type(&ty), "Optional<List<String>>");
    }

    #[test]
    fn java_return_type_preserves_non_optional() {
        assert_eq!(java_return_type(&TypeRef::String), "String");
    }

    #[test]
    fn java_return_type_preserves_vec() {
        let ty = TypeRef::Vec(Box::new(TypeRef::String));
        assert_eq!(java_return_type(&ty), "List<String>");
    }

    #[test]
    fn java_ffi_type_promotes_bool_to_java_long() {
        assert_eq!(java_ffi_type(&PrimitiveType::Bool), "ValueLayout.JAVA_LONG");
    }

    #[test]
    fn java_ffi_type_promotes_all_integers_to_java_long() {
        assert_eq!(java_ffi_type(&PrimitiveType::U8), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::I8), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::U16), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::I16), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::U32), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::I32), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::U64), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::I64), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::Usize), "ValueLayout.JAVA_LONG");
        assert_eq!(java_ffi_type(&PrimitiveType::Isize), "ValueLayout.JAVA_LONG");
    }

    #[test]
    fn java_ffi_type_float_types_unchanged() {
        assert_eq!(java_ffi_type(&PrimitiveType::F32), "ValueLayout.JAVA_FLOAT");
        assert_eq!(java_ffi_type(&PrimitiveType::F64), "ValueLayout.JAVA_DOUBLE");
    }
}
