use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

use super::json_values::elixir_typespec;

/// Generate a type-appropriate unsupported body for Rustler.
pub(in crate::backends::rustler::gen_bindings) fn gen_rustler_unimplemented_body(
    return_type: &TypeRef,
    fn_name: &str,
    has_error: bool,
) -> String {
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        format!("Err(String::from(\"{err_msg}\"))")
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => format!("String::from(\"[unimplemented: {fn_name}]\")"),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                crate::core::ir::PrimitiveType::Bool => "false".to_string(),
                crate::core::ir::PrimitiveType::F32 | crate::core::ir::PrimitiveType::F64 => "0.0".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => format!(
                "compile_error!(\"alef cannot generate Rustler binding for {fn_name}; \
                 configure elixir.exclude_functions or make the return type fallible\")"
            ),
        }
    }
}

/// Map a return type, wrapping opaque Named types in ResourceArc.
/// Handles both bare opaque returns (T) and optional opaque returns (Option<T>).
pub(in crate::backends::rustler::gen_bindings) fn map_return_type(
    ty: &TypeRef,
    mapper: &crate::backends::rustler::type_map::RustlerMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    match ty {
        TypeRef::Named(n) if opaque_types.contains(n) => format!("ResourceArc<{n}>"),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                if opaque_types.contains(n) {
                    return format!("Option<ResourceArc<{n}>>");
                }
            }
            mapper.map_type(ty)
        }
        _ => mapper.map_type(ty),
    }
}

/// Map a return TypeRef to an Elixir typespec for `@spec` return annotations.
///
/// For `Named` types that are in `default_types` (i.e. they are passed *into* NIFs as
/// JSON strings), the **input** typespec is `String.t() | nil`. But when such a type
/// appears as a **return** type the NIF returns the fully-deserialised struct/map.
///
/// Errors are returned as `{:error, atom, String.t()}` where the atom is the error kind
/// and the string is the human-readable message.
pub(in crate::backends::rustler::gen_bindings) fn elixir_return_typespec(
    ty: &TypeRef,
    has_error: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    let base = match ty {
        TypeRef::Named(name) if default_types.contains(name) => "map()".to_string(),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if default_types.contains(name) => "map() | nil".to_string(),
            _ => elixir_typespec(ty, opaque_types, default_types),
        },
        _ => elixir_typespec(ty, opaque_types, default_types),
    };
    if has_error {
        format!("{{:ok, {}}} | {{:error, atom, String.t()}}", base)
    } else {
        base
    }
}
