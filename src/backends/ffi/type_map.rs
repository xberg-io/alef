use std::borrow::Cow;

use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, TypeRef};
use ahash::{AHashMap, AHashSet};

/// TypeMapper for C FFI bindings — parameter position (input, `*const`).
///
/// Holds the `core_import` path used to qualify Named types (e.g. `"my_crate"`).
/// Maps Rust types to the C FFI parameter types:
/// - Strings and paths become `*const std::ffi::c_char`
/// - Primitives use their direct C-compatible Rust types
/// - Optional types use nullable pointers or sentinel values
/// - Vec/Map become `*const std::ffi::c_char` (JSON-encoded)
pub struct FfiParamMapper<'a> {
    pub core_import: &'a str,
}

impl TypeMapper for FfiParamMapper<'_> {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        c_primitive(prim)
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("*const std::ffi::c_char")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("*const u8")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("*const std::ffi::c_char")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("*const std::ffi::c_char")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("u64")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(format!("*const {}::{name}", self.core_import))
    }

    fn vec(&self, _inner: &str) -> String {
        "*const std::ffi::c_char".to_string()
    }

    fn map(&self, _key: &str, _value: &str) -> String {
        "*const std::ffi::c_char".to_string()
    }

    /// Override map_type to handle Optional's complex C FFI sentinel/pointer semantics.
    ///
    /// Optional params use nullable pointers or integer sentinels depending on the inner type.
    /// The default map_type cannot capture this because it loses access to the inner TypeRef.
    fn map_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(prim) => self.primitive(prim).into_owned(),
            TypeRef::String | TypeRef::Char => self.string().into_owned(),
            TypeRef::Bytes => self.bytes().into_owned(),
            TypeRef::Path => self.path().into_owned(),
            TypeRef::Json => self.json().into_owned(),
            TypeRef::Unit => self.unit().into_owned(),
            TypeRef::Duration => self.duration().into_owned(),
            TypeRef::Named(name) => self.named(name).into_owned(),
            TypeRef::Vec(_) => self.vec("").to_string(),
            TypeRef::Map(_, _) => self.map("", "").to_string(),
            TypeRef::Optional(inner) => c_param_optional(inner, self.core_import),
        }
    }

    fn error_wrapper(&self) -> &str {
        "i32"
    }
}

/// TypeMapper for C FFI bindings — return position (output, `*mut`).
///
/// Holds the `core_import` path used to qualify Named types.
/// Maps Rust types to the C FFI return types (mutable pointers for heap-allocated values).
pub struct FfiReturnMapper<'a> {
    pub core_import: &'a str,
}

impl TypeMapper for FfiReturnMapper<'_> {
    fn primitive(&self, prim: &PrimitiveType) -> Cow<'static, str> {
        c_primitive(prim)
    }

    fn string(&self) -> Cow<'static, str> {
        Cow::Borrowed("*mut std::ffi::c_char")
    }

    fn bytes(&self) -> Cow<'static, str> {
        Cow::Borrowed("*mut u8")
    }

    fn path(&self) -> Cow<'static, str> {
        Cow::Borrowed("*mut std::ffi::c_char")
    }

    fn json(&self) -> Cow<'static, str> {
        Cow::Borrowed("*mut std::ffi::c_char")
    }

    fn unit(&self) -> Cow<'static, str> {
        Cow::Borrowed("()")
    }

    fn duration(&self) -> Cow<'static, str> {
        Cow::Borrowed("u64")
    }

    fn named<'a>(&self, name: &'a str) -> Cow<'a, str> {
        Cow::Owned(format!("*mut {}::{name}", self.core_import))
    }

    fn vec(&self, _inner: &str) -> String {
        "*mut std::ffi::c_char".to_string()
    }

    fn map(&self, _key: &str, _value: &str) -> String {
        "*mut std::ffi::c_char".to_string()
    }

    /// Override map_type to handle Optional's complex C FFI nullable-pointer semantics.
    fn map_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(prim) => self.primitive(prim).into_owned(),
            TypeRef::String | TypeRef::Char => self.string().into_owned(),
            TypeRef::Bytes => self.bytes().into_owned(),
            TypeRef::Path => self.path().into_owned(),
            TypeRef::Json => self.json().into_owned(),
            TypeRef::Unit => self.unit().into_owned(),
            TypeRef::Duration => self.duration().into_owned(),
            TypeRef::Named(name) => self.named(name).into_owned(),
            TypeRef::Vec(_) => self.vec("").to_string(),
            TypeRef::Map(_, _) => self.map("", "").to_string(),
            TypeRef::Optional(inner) => c_return_optional(inner, self.core_import),
        }
    }

    fn error_wrapper(&self) -> &str {
        "i32"
    }
}

/// Maps a TypeRef to the C FFI parameter type (input position).
///
/// Delegates to [`FfiParamMapper`] for exhaustive TypeRef handling.
pub fn c_param_type(ty: &TypeRef, core_import: &str) -> Cow<'static, str> {
    Cow::Owned(FfiParamMapper { core_import }.map_type(ty))
}

/// Maps a TypeRef to the C FFI return type (output position).
///
/// Delegates to [`FfiReturnMapper`] for exhaustive TypeRef handling.
pub fn c_return_type(ty: &TypeRef, core_import: &str) -> Cow<'static, str> {
    Cow::Owned(FfiReturnMapper { core_import }.map_type(ty))
}

/// Maps a primitive type to its C FFI equivalent.
fn c_primitive(prim: &PrimitiveType) -> Cow<'static, str> {
    Cow::Borrowed(match prim {
        PrimitiveType::Bool => "i32",
        PrimitiveType::U8 => "u8",
        PrimitiveType::U16 => "u16",
        PrimitiveType::U32 => "u32",
        PrimitiveType::U64 => "u64",
        PrimitiveType::I8 => "i8",
        PrimitiveType::I16 => "i16",
        PrimitiveType::I32 => "i32",
        PrimitiveType::I64 => "i64",
        PrimitiveType::F32 => "f32",
        PrimitiveType::F64 => "f64",
        PrimitiveType::Usize => "usize",
        PrimitiveType::Isize => "isize",
    })
}

/// C FFI Optional parameter type — sentinel/nullable-pointer logic.
fn c_param_optional(inner: &TypeRef, core_import: &str) -> String {
    match inner {
        TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
        TypeRef::Primitive(_) => c_param_type(inner, core_import).into_owned(),
        TypeRef::Optional(inner2) => match inner2.as_ref() {
            TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
            TypeRef::Primitive(_) => c_param_type(inner2, core_import).into_owned(),
            _ => "*const std::ffi::c_char".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "*const std::ffi::c_char".to_string(),
        TypeRef::Named(_) => format!("*const {}", c_param_type(inner, core_import)),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes | TypeRef::Unit | TypeRef::Duration => {
            "*const std::ffi::c_char".to_string()
        }
    }
}

/// C FFI Optional return type — nullable-pointer logic.
fn c_return_optional(inner: &TypeRef, core_import: &str) -> String {
    match inner {
        TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
        TypeRef::Primitive(_) => c_return_type(inner, core_import).into_owned(),
        TypeRef::Optional(inner2) => match inner2.as_ref() {
            TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
            TypeRef::Primitive(_) => c_return_type(inner2, core_import).into_owned(),
            _ => "*mut std::ffi::c_char".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "*mut std::ffi::c_char".to_string(),
        TypeRef::Named(name) => format!("*mut {core_import}::{name}"),
        TypeRef::Duration => "u64".to_string(),
        TypeRef::Bytes => "*mut u8".to_string(),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Unit => "*mut std::ffi::c_char".to_string(),
    }
}

/// Returns `true` if the return type is void in C.
pub fn is_void_return(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Unit)
}

/// Returns `true` if the return type passes through without conversion in C FFI.
/// For these types, the call expression can be used directly as the tail expression
/// without binding to an intermediate `let result = ...;`.
pub fn is_passthrough_return(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Primitive(p) if !matches!(p, crate::core::ir::PrimitiveType::Bool)
    )
}

/// Maps a TypeRef to the C FFI parameter type, using full rust_path from path_map for Named
/// types and emitting `i32` for enum types.
///
/// When `is_mut` is true and the type is a non-enum Named type, the pointer is `*mut` instead
/// of `*const` — required when the core function takes `&mut T` rather than `&T` or `T`.
/// Optional Named types are always `*const` (null = None), regardless of `is_mut`.
pub fn c_param_type_with_paths_and_enums(
    ty: &TypeRef,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    is_mut: bool,
) -> Cow<'static, str> {
    match ty {
        TypeRef::Named(name) => {
            if enum_names.contains(name.as_str()) {
                Cow::Borrowed("i32")
            } else {
                let full_path = path_map
                    .get(name.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("{core_import}::{name}"));
                let ptr_kind = if is_mut { "*mut" } else { "*const" };
                Cow::Owned(format!("{ptr_kind} {full_path}"))
            }
        }
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(name) = inner.as_ref() {
                if enum_names.contains(name.as_str()) {
                    Cow::Borrowed("i32")
                } else {
                    let inner_type = path_map
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or_else(|| format!("{core_import}::{name}"));
                    Cow::Owned(format!("*const {inner_type}"))
                }
            } else {
                c_param_type(ty, core_import)
            }
        }
        _ => c_param_type(ty, core_import),
    }
}

/// Like `c_return_type` but uses full rust_path from path_map for Named types.
pub fn c_return_type_with_paths(
    ty: &TypeRef,
    core_import: &str,
    path_map: &AHashMap<String, String>,
) -> Cow<'static, str> {
    match ty {
        TypeRef::Named(name) => {
            let full_path = path_map
                .get(name.as_str())
                .map(|s| s.as_str())
                .unwrap_or_else(|| name.as_str());
            Cow::Owned(format!("*mut {full_path}"))
        }
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(name) = inner.as_ref() {
                let inner_type = path_map
                    .get(name.as_str())
                    .map(|s| s.as_str())
                    .unwrap_or_else(|| name.as_str());
                Cow::Owned(format!("*mut {inner_type}"))
            } else {
                c_return_type(ty, core_import)
            }
        }
        _ => c_return_type(ty, core_import),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORE: &str = "my_crate";

    #[test]
    fn test_param_primitive_bool_becomes_i32() {
        assert_eq!(c_param_type(&TypeRef::Primitive(PrimitiveType::Bool), CORE), "i32");
    }

    #[test]
    fn test_param_primitive_u32() {
        assert_eq!(c_param_type(&TypeRef::Primitive(PrimitiveType::U32), CORE), "u32");
    }

    #[test]
    fn test_param_string() {
        assert_eq!(c_param_type(&TypeRef::String, CORE), "*const std::ffi::c_char");
    }

    #[test]
    fn test_param_char() {
        assert_eq!(c_param_type(&TypeRef::Char, CORE), "*const std::ffi::c_char");
    }

    #[test]
    fn test_param_bytes() {
        assert_eq!(c_param_type(&TypeRef::Bytes, CORE), "*const u8");
    }

    #[test]
    fn test_param_path() {
        assert_eq!(c_param_type(&TypeRef::Path, CORE), "*const std::ffi::c_char");
    }

    #[test]
    fn test_param_json() {
        assert_eq!(c_param_type(&TypeRef::Json, CORE), "*const std::ffi::c_char");
    }

    #[test]
    fn test_param_unit() {
        assert_eq!(c_param_type(&TypeRef::Unit, CORE), "");
    }

    #[test]
    fn test_param_duration() {
        assert_eq!(c_param_type(&TypeRef::Duration, CORE), "u64");
    }

    #[test]
    fn test_param_named() {
        assert_eq!(
            c_param_type(&TypeRef::Named("MyType".to_string()), CORE),
            "*const my_crate::MyType"
        );
    }

    #[test]
    fn test_param_vec() {
        assert_eq!(
            c_param_type(&TypeRef::Vec(Box::new(TypeRef::String)), CORE),
            "*const std::ffi::c_char"
        );
    }

    #[test]
    fn test_param_optional_bool_is_i32_sentinel() {
        assert_eq!(
            c_param_type(
                &TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                CORE
            ),
            "i32"
        );
    }

    #[test]
    fn test_param_optional_string_is_nullable_ptr() {
        assert_eq!(
            c_param_type(&TypeRef::Optional(Box::new(TypeRef::String)), CORE),
            "*const std::ffi::c_char"
        );
    }

    #[test]
    fn test_return_primitive_bool_becomes_i32() {
        assert_eq!(c_return_type(&TypeRef::Primitive(PrimitiveType::Bool), CORE), "i32");
    }

    #[test]
    fn test_return_string() {
        assert_eq!(c_return_type(&TypeRef::String, CORE), "*mut std::ffi::c_char");
    }

    #[test]
    fn test_return_bytes() {
        assert_eq!(c_return_type(&TypeRef::Bytes, CORE), "*mut u8");
    }

    #[test]
    fn test_return_unit() {
        assert_eq!(c_return_type(&TypeRef::Unit, CORE), "()");
    }

    #[test]
    fn test_return_duration() {
        assert_eq!(c_return_type(&TypeRef::Duration, CORE), "u64");
    }

    #[test]
    fn test_return_named() {
        assert_eq!(
            c_return_type(&TypeRef::Named("MyType".to_string()), CORE),
            "*mut my_crate::MyType"
        );
    }

    #[test]
    fn test_return_optional_bool_is_i32_sentinel() {
        assert_eq!(
            c_return_type(
                &TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
                CORE
            ),
            "i32"
        );
    }

    #[test]
    fn test_return_optional_string_is_nullable_mut_ptr() {
        assert_eq!(
            c_return_type(&TypeRef::Optional(Box::new(TypeRef::String)), CORE),
            "*mut std::ffi::c_char"
        );
    }

    #[test]
    fn test_return_optional_named() {
        assert_eq!(
            c_return_type(&TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string()))), CORE),
            "*mut my_crate::Foo"
        );
    }

    #[test]
    fn test_param_enum_by_value_as_i32() {
        let mut enum_names = ahash::AHashSet::new();
        enum_names.insert("Method".to_string());
        let path_map = ahash::AHashMap::new();
        assert_eq!(
            c_param_type_with_paths_and_enums(
                &TypeRef::Named("Method".to_string()),
                CORE,
                &path_map,
                &enum_names,
                false
            ),
            "i32"
        );
    }

    #[test]
    fn test_param_optional_enum_by_value_as_i32() {
        let mut enum_names = ahash::AHashSet::new();
        enum_names.insert("Status".to_string());
        let path_map = ahash::AHashMap::new();
        assert_eq!(
            c_param_type_with_paths_and_enums(
                &TypeRef::Optional(Box::new(TypeRef::Named("Status".to_string()))),
                CORE,
                &path_map,
                &enum_names,
                false
            ),
            "i32"
        );
    }

    #[test]
    fn test_param_named_type_as_pointer_when_not_enum() {
        let enum_names = ahash::AHashSet::new();
        let path_map = ahash::AHashMap::new();
        assert_eq!(
            c_param_type_with_paths_and_enums(
                &TypeRef::Named("RouteBuilder".to_string()),
                CORE,
                &path_map,
                &enum_names,
                false
            ),
            "*const my_crate::RouteBuilder"
        );
    }

    #[test]
    fn test_param_named_type_is_mut_emits_mut_pointer() {
        let enum_names = ahash::AHashSet::new();
        let path_map = ahash::AHashMap::new();
        assert_eq!(
            c_param_type_with_paths_and_enums(
                &TypeRef::Named("ExtractionResult".to_string()),
                CORE,
                &path_map,
                &enum_names,
                true
            ),
            "*mut my_crate::ExtractionResult"
        );
    }

    #[test]
    fn test_param_optional_named_is_mut_stays_const_pointer() {
        let enum_names = ahash::AHashSet::new();
        let path_map = ahash::AHashMap::new();
        assert_eq!(
            c_param_type_with_paths_and_enums(
                &TypeRef::Optional(Box::new(TypeRef::Named("OperationOutput".to_string()))),
                CORE,
                &path_map,
                &enum_names,
                true
            ),
            "*const my_crate::OperationOutput"
        );
    }
}
