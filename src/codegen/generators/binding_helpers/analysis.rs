use crate::core::ir::{ParamDef, TypeRef};
use ahash::AHashSet;

/// Check if params contain any non-opaque Named types that need let bindings.
/// This includes direct Named types, `Vec<Named>` types, `Vec<String>` params
/// with is_ref=true (which need a `Vec<&str>` intermediate to pass as `&[&str]`),
/// and sanitized `Vec<String>` params (which are JSON-deserialized to tuples).
pub fn has_named_params(params: &[ParamDef], opaque_types: &AHashSet<String>) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => true,
        TypeRef::Vec(inner) => {
            matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name.as_str()))
                || (matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref)
                || (matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some())
        }
        _ => false,
    })
}

/// Check if a param type is safe for non-opaque delegation (no complex conversions needed).
/// Vec and Map params can cause type mismatches (e.g. `Vec<String>` vs `&[&str]`).
///
/// `Json` is delegatable: the binding takes a JSON string and `gen_call_args` emits
/// `serde_json::from_str(...)` to bridge it into the core `serde_json::Value` parameter.
/// This lets fluent-builder methods like `with_extension(self, key: String, value: Value) -> Self`
/// be auto-generated instead of being rejected as non-delegatable.
pub fn is_simple_non_opaque_param(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Primitive(_)
        | TypeRef::String
        | TypeRef::Char
        | TypeRef::Bytes
        | TypeRef::Path
        | TypeRef::Unit
        | TypeRef::Duration
        | TypeRef::Json => true,
        TypeRef::Optional(inner) => is_simple_non_opaque_param(inner),
        _ => false,
    }
}
