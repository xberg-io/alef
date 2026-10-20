use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::TypeRef;
use ahash::AHashSet;

use super::json_values::elixir_struct_field_typespec;

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
            _ => format!(
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
            if let TypeRef::Named(n) = inner.as_ref()
                && opaque_types.contains(n)
            {
                return format!("Option<ResourceArc<{n}>>");
            }
            mapper.map_type(ty)
        }
        _ => mapper.map_type(ty),
    }
}

/// Map a return TypeRef to an Elixir typespec for `@spec` return annotations.
///
/// Public return values use decoded DTO types, including inside collections. ~keep
pub(in crate::backends::rustler::gen_bindings) fn elixir_return_typespec(
    ty: &TypeRef,
    has_error: bool,
    opaque_types: &AHashSet<String>,
    app_module: &str,
    public_types: &AHashSet<String>,
) -> String {
    let base = elixir_struct_field_typespec(ty, app_module, opaque_types, &AHashSet::new(), public_types);
    if has_error {
        format!("{{:ok, {}}} | {{:error, atom, String.t()}}", base)
    } else {
        base
    }
}
