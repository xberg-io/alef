use std::borrow::Cow;

use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{PrimitiveType, ReceiverKind, TypeRef};
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

    fn named<'a>(&self, _name: &'a str) -> Cow<'a, str> {
        Cow::Borrowed("AlefHandle")
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

    fn named<'a>(&self, _name: &'a str) -> Cow<'a, str> {
        Cow::Borrowed("AlefHandle")
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
        TypeRef::Named(_) => "AlefHandle".to_string(),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Bytes | TypeRef::Unit | TypeRef::Duration => {
            "*const std::ffi::c_char".to_string()
        }
    }
}

/// True when `Optional<inner>` crosses the C ABI as a raw scalar in return position (a bare
/// primitive/`u64`) rather than a pointer (`*mut c_char` / `AlefHandle`). This is the exact
/// decision [`c_return_optional`] below makes; every C-ABI-consuming backend that has to declare
/// a matching return type (C#'s `[DllImport]`, Go's cgo signature, ...) must ask this function
/// instead of re-deriving its own copy of the branching — see `two-generators-disagree` in the
/// repo's skill set for why that duplication is this codebase's dominant defect shape.
///
/// The nested-`Optional` case is deliberately narrower than the single-level case:
/// `Option<Option<Duration>>` does NOT flatten to a scalar (only `Option<Duration>` does) —
/// [`c_return_optional`]'s nested-optional arm only recurses into `Primitive`, never `Duration`.
/// `should_agree_with_c_return_optional_on_every_shape` below pins this asymmetry so a future
/// edit to either side that drifts is caught immediately rather than silently. ~keep
pub fn optional_return_crosses_as_scalar(inner: &TypeRef) -> bool {
    match inner {
        TypeRef::Primitive(_) | TypeRef::Duration => true,
        TypeRef::Optional(inner2) => matches!(inner2.as_ref(), TypeRef::Primitive(_)),
        _ => false,
    }
}

/// True when an FFI getter -- a struct field accessor, or a bare `Optional<T>` function/method
/// return -- cannot distinguish `None` from a legitimate zero-valued `Some` using its return
/// value alone: the C ABI has no null representation for the leaf type, so both collapse to the
/// same sentinel in `null_return_value`. Pointer-shaped returns (String/Path/Json/Bytes/Vec/Map/
/// Char) and handle-shaped returns (Named types, where handle `0` is reserved and
/// `insert_handle` never allocates it) already carry a real null and are excluded. Recurses
/// through a nested `Option<Option<T>>` (a struct field's "not touched" pattern, or a
/// function/method returning `Option<Option<T>>` directly) because the outer getter still emits
/// one sentinel for both `None` and `Some(None)`.
///
/// This is the single source of truth every consumer of the presence-companion convention
/// (`{prefix}_{type}_has_{field}` for fields, `{fn}_has_result` for functions/methods, and any
/// host-language backend that has to decide whether to call the companion before trusting a
/// scalar it received) must ask, instead of re-deriving the "which leaves are ambiguous"
/// judgment independently -- see `two-generators-disagree` in the repo's skill set. Deliberately
/// public (unlike the field/return codegen that consumes it) because Go's cgo wrapper needs the
/// same answer to decide whether to call the companion before wrapping a raw scalar in a
/// pointer. `ty` is the type with the outermost `Option` already stripped: pass `&field.ty` for
/// an optional field, or the inner of a `TypeRef::Optional` for a bare function/method return
/// type. ~keep
pub fn optional_leaf_needs_presence_signal(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_) | TypeRef::Duration => true,
        TypeRef::Optional(inner) => optional_leaf_needs_presence_signal(inner),
        _ => false,
    }
}

/// True when a function or method's `return_type` gets a `{fn}_has_result` presence companion.
/// Combines [`optional_leaf_needs_presence_signal`] (is the leaf ambiguous) with the one other
/// condition that rules a companion out: an owned receiver (`receiver == Some(ReceiverKind::Owned)`).
/// The companion has to call the underlying method a second time to observe presence, and an
/// owned receiver's first call already removes the handle from the registry -- a second call
/// would not just risk re-running a side effect, it would fail outright. Pass `receiver: None`
/// for a free function or a static method.
///
/// This is THE eligibility predicate `gen_method_result_presence_wrapper` /
/// `gen_free_function_result_presence_wrapper` use to decide whether to emit the companion, and
/// every host-language backend that has to decide whether to declare/call it (Go's cgo wrapper,
/// ...) must ask the same question through this function -- not re-derive "is this eligible"
/// independently, or it will reference a companion the FFI crate never exported. ~keep
pub fn result_presence_companion_exists(return_type: &TypeRef, receiver: Option<&ReceiverKind>) -> bool {
    let TypeRef::Optional(inner) = return_type else {
        return false;
    };
    if !optional_leaf_needs_presence_signal(inner) {
        return false;
    }
    receiver != Some(&ReceiverKind::Owned)
}

/// C FFI Optional return type — nullable-pointer logic.
fn c_return_optional(inner: &TypeRef, core_import: &str) -> String {
    match inner {
        TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
        TypeRef::Primitive(_) => c_return_type(inner, core_import).into_owned(),
        TypeRef::Optional(inner2) => match inner2.as_ref() {
            TypeRef::Primitive(PrimitiveType::Bool) => "i32".to_string(),
            TypeRef::Primitive(_) => c_return_type(inner2, core_import).into_owned(),
            // `Option<Option<Named>>` returns a handle, exactly like the single-level
            // `Option<Named>` one line below. This arm used to fall through to `*mut c_char`
            // while the emitted body (`gen_owned_value_to_c` -> `insert_handle`) and the absent
            // branch (`null_return_value`, which answers `0` for this shape) both produced a
            // handle -- a declaration the function's own body could not satisfy. ~keep
            TypeRef::Named(_) => "AlefHandle".to_string(),
            _ => "*mut std::ffi::c_char".to_string(),
        },
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => "*mut std::ffi::c_char".to_string(),
        TypeRef::Named(_) => "AlefHandle".to_string(),
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

/// The Rust spelling of a `Named` parameter that crosses the C ABI as a bare scalar
/// discriminant rather than as a handle-registry key. cbindgen renders it `int32_t`.
pub const SCALAR_NAMED_C_PARAM_TYPE: &str = "i32";

/// The Rust spelling of a `Named` parameter that crosses the C ABI as an `AlefHandle`
/// (`uint64_t`) registry key.
pub const HANDLE_NAMED_C_PARAM_TYPE: &str = "AlefHandle";

/// The set of IR type names whose `Named` parameter positions cross the C ABI as
/// [`SCALAR_NAMED_C_PARAM_TYPE`] instead of [`HANDLE_NAMED_C_PARAM_TYPE`].
///
/// This is the single construction of that set. Every backend that has to mirror an emitted C
/// signature — the C FFI crate itself, and the C-ABI consumers such as the C# `[DllImport]`
/// emitters — must build it from here rather than deriving an independent notion of
/// "is this an enum", so a host binding can never declare a width the C header contradicts. ~keep
pub fn scalar_c_abi_named_types(api: &crate::core::ir::ApiSurface) -> AHashSet<String> {
    api.enums
        .iter()
        .filter(|enum_def| enum_def.is_copy)
        .map(|enum_def| enum_def.name.clone())
        .chain(
            api.types
                .iter()
                .filter(|type_def| !type_def.is_trait && type_def.is_copy)
                .map(|type_def| type_def.name.clone()),
        )
        .collect()
}

/// Named types that cross the C ABI as [`SCALAR_NAMED_C_PARAM_TYPE`] in *parameter* position.
///
/// Strictly wider than [`scalar_c_abi_named_types`], and deliberately so: that set answers
/// "is this type `Copy`", but a parameter's C width is decided by whether the FFI crate can
/// lower it to a discriminant, and it emits a `<enum>_from_i32` helper for **any** enum whose
/// variants are all fieldless -- `Copy` or not. A consumer's `Method` is the worked example: not
/// `Copy`, so the `Copy` set omits it, yet the C header declares
/// `<crate>_route_builder_new(int32_t method, …)` and the C# wrapper casts `(int)method`, while
/// the C# `[DllImport]` derived from the `Copy` set declared `ulong`. Three renderings of one
/// parameter, two of them agreeing and the declaration between them disagreeing -- a CS1503 at
/// best and a silent ABI violation at worst.
///
/// Every backend that declares a foreign signature against the C header must read parameter
/// widths from here, not from [`scalar_c_abi_named_types`]. ~keep
pub fn scalar_c_abi_param_named_types(api: &crate::core::ir::ApiSurface) -> AHashSet<String> {
    let mut named = scalar_c_abi_named_types(api);
    named.extend(
        api.enums
            .iter()
            .filter(|enum_def| {
                enum_def
                    .variants
                    .iter()
                    .all(|variant| variant.fields.is_empty() && !variant.is_tuple)
            })
            .map(|enum_def| enum_def.name.clone()),
    );
    named
}

/// True when the IR type called `name` has parameter positions that cross the C ABI as
/// [`SCALAR_NAMED_C_PARAM_TYPE`]. Convenience over [`scalar_c_abi_named_types`] for callers that
/// resolve one name at a time; it builds the set so there is still only one such construction.
pub fn is_scalar_c_abi_named_type(api: &crate::core::ir::ApiSurface, name: &str) -> bool {
    scalar_c_abi_named_types(api).contains(name)
}

/// True when `ty` occupies a `Named` (or `Optional<Named>`) position that crosses the C ABI as
/// [`SCALAR_NAMED_C_PARAM_TYPE`]. Pair with [`scalar_c_abi_named_types`].
pub fn crosses_c_abi_as_scalar(ty: &TypeRef, scalar_named_types: &AHashSet<String>) -> bool {
    let named = match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    };
    named.is_some_and(|name| scalar_named_types.contains(name.as_str()))
}

/// Maps a TypeRef to the C FFI parameter type, using full rust_path from path_map for Named
/// types and emitting [`SCALAR_NAMED_C_PARAM_TYPE`] for scalar-discriminant types.
pub fn c_param_type_with_paths_and_enums(
    ty: &TypeRef,
    core_import: &str,
    _path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    _is_mut: bool,
) -> Cow<'static, str> {
    let is_named_position = matches!(ty, TypeRef::Named(_))
        || matches!(ty, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)));
    if !is_named_position {
        return c_param_type(ty, core_import);
    }
    if crosses_c_abi_as_scalar(ty, enum_names) {
        Cow::Borrowed(SCALAR_NAMED_C_PARAM_TYPE)
    } else {
        Cow::Borrowed(HANDLE_NAMED_C_PARAM_TYPE)
    }
}

/// Like `c_return_type` but uses full rust_path from path_map for Named types.
pub fn c_return_type_with_paths(
    ty: &TypeRef,
    core_import: &str,
    _path_map: &AHashMap<String, String>,
) -> Cow<'static, str> {
    match ty {
        TypeRef::Named(_) => Cow::Borrowed("AlefHandle"),
        TypeRef::Optional(inner) => {
            if matches!(inner.as_ref(), TypeRef::Named(_)) {
                Cow::Borrowed("AlefHandle")
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
        assert_eq!(c_param_type(&TypeRef::Named("MyType".to_string()), CORE), "AlefHandle");
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
        assert_eq!(c_return_type(&TypeRef::Named("MyType".to_string()), CORE), "AlefHandle");
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
            "AlefHandle"
        );
    }

    /// A C function returns one value, so `Option<Option<Named>>` collapses to the same handle
    /// the single-level `Option<Named>` returns. The declaration used to say `*mut c_char` while
    /// the emitted body handed back `insert_handle(..)` and the absent branch handed back the
    /// handle-shaped `0` -- three answers to one question, only one of which cbindgen writes into
    /// the header. Asserting the declaration alone would not have caught it, so this pins it
    /// against the single-level spelling that the body and the null value already agree with.
    /// ~keep
    #[test]
    fn should_return_a_handle_when_a_nested_option_returns_a_named_type() {
        let single = TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string())));
        let nested = TypeRef::Optional(Box::new(single.clone()));

        assert_eq!(c_return_type(&nested, CORE), "AlefHandle");
        assert_eq!(c_return_type(&nested, CORE), c_return_type(&single, CORE));
        assert_ne!(c_return_type(&nested, CORE), "*mut std::ffi::c_char");
    }

    /// `optional_return_crosses_as_scalar` is the fact every C-ABI-consuming backend asks instead
    /// of re-deriving its own copy of `c_return_optional`'s scalar-vs-pointer branching. Walk a
    /// representative shape for every `TypeRef` variant (including the nested-`Optional` shapes,
    /// where `Duration` and `Primitive` deliberately disagree) and assert the predicate agrees
    /// with what `c_return_optional` actually emits: a scalar answer must never come back as a
    /// pointer spelling (`*mut ...` / `AlefHandle`), and a pointer answer must never come back as
    /// a bare Rust primitive spelling.
    #[test]
    fn should_agree_with_c_return_optional_on_every_shape() {
        let shapes: Vec<TypeRef> = vec![
            TypeRef::Primitive(PrimitiveType::Bool),
            TypeRef::Primitive(PrimitiveType::U8),
            TypeRef::Primitive(PrimitiveType::U64),
            TypeRef::Primitive(PrimitiveType::F64),
            TypeRef::Duration,
            TypeRef::String,
            TypeRef::Char,
            TypeRef::Path,
            TypeRef::Json,
            TypeRef::Bytes,
            TypeRef::Named("Foo".to_string()),
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8))),
            TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Primitive(PrimitiveType::U8)),
            ),
            TypeRef::Unit,
            // Nested Optional: Primitive flattens to scalar, Duration does not — the asymmetry
            // this test exists to pin.
            TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
            TypeRef::Optional(Box::new(TypeRef::Duration)),
            TypeRef::Optional(Box::new(TypeRef::Named("Foo".to_string()))),
        ];

        for inner in shapes {
            let c_type = c_return_optional(&inner, CORE);
            let is_pointer_spelling = c_type == "AlefHandle" || c_type.starts_with("*mut ");
            assert_eq!(
                optional_return_crosses_as_scalar(&inner),
                !is_pointer_spelling,
                "optional_return_crosses_as_scalar({inner:?}) disagrees with c_return_optional's \
                 actual output `{c_type}`"
            );
        }

        // The nested-Optional asymmetry itself, spelled out directly rather than only inferred
        // from the loop above.
        assert!(optional_return_crosses_as_scalar(&TypeRef::Optional(Box::new(
            TypeRef::Primitive(PrimitiveType::U32)
        ))));
        assert!(!optional_return_crosses_as_scalar(&TypeRef::Optional(Box::new(
            TypeRef::Duration
        ))));
    }

    #[test]
    fn test_return_named_with_paths_is_a_generational_handle() {
        let path_map = ahash::AHashMap::from_iter([("Request".to_string(), "my_crate::nested::Request".to_string())]);
        assert_eq!(
            c_return_type_with_paths(&TypeRef::Named("Request".to_string()), CORE, &path_map),
            "AlefHandle"
        );
        assert_eq!(
            c_return_type_with_paths(
                &TypeRef::Optional(Box::new(TypeRef::Named("Request".to_string()))),
                CORE,
                &path_map,
            ),
            "AlefHandle"
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
            "AlefHandle"
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
            "AlefHandle"
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
            "AlefHandle"
        );
    }
}
