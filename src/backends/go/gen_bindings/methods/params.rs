use super::super::types::{cgo_type_for_primitive, primitive_max_sentinel};
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::go_type;
use crate::codegen::c_consumer;
use crate::codegen::naming::go_param_name;
use crate::core::ir::{ParamDef, TypeRef};

/// The JSON text `serde_json` writes for an EMPTY value of `param.ty`, for collection types whose
/// Go counterpart has a nil form that marshals to `null` instead.
///
/// ~keep Go's `json.Marshal` writes `null` for a nil slice or map and `[]`/`{}` only for a
/// non-nil empty one, so the same logical argument crosses the ABI two different ways depending
/// on how the caller happened to construct it — and `null` is not a value the Rust side accepts:
/// `ffi/templates/param_non_optional_json_conversion.jinja` runs `serde_json::from_str::<Vec<T>>`
/// on it and takes the `set_last_error(2, ..)` branch. `param_named_type.jinja` already performs
/// exactly this substitution for a nil DTO pointer; the Vec/Map path was the one that did not.
///
/// An optional collection also returns `None`: Go slices and maps are nil-capable reference
/// values, and their nil `null` is the intentional `Option::None` wire value rather than empty.
///
/// `TypeRef::Json` deliberately returns `None`: there the Rust side is a `serde_json::Value`, for
/// which `null` is a legitimate inhabitant rather than a nil artefact, and rewriting it to `[]`
/// would change the argument's meaning.
fn empty_collection_wire_literal(param: &ParamDef) -> Option<&'static str> {
    if param.optional {
        return None;
    }
    match &param.ty {
        TypeRef::Vec(_) => Some("[]"),
        TypeRef::Map(_, _) => Some("{}"),
        _ => None,
    }
}

/// Generate parameter conversion code from Go to C.
/// `err_return_prefix` is the leading `"<zero>, "` (or `""` for value-less returns) prepended to
/// every `return ... fmt.Errorf(...)` early exit. Callers compute it from the enclosing function's
/// return type — `"nil, "` for pointer/slice/channel returns, `"0, "` / `"false, "` / `"\"\", "`
/// for plain primitive/string returns, and `""` when the function only returns `error`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
pub(in crate::backends::go::gen_bindings) fn gen_param_to_c(
    param: &ParamDef,
    err_return_prefix: &str,
    can_return_error: bool,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_required.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Path => {
            if param.optional {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_required.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Bytes => {
            out.push_str(&crate::backends::go::template_env::render(
                "bytes_to_c_pointer.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_param => &go_param,
                },
            ));
            out.push('\n');
        }
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                // `param_opaque_cast.jinja` emits `{{ c_name }} := {{ go_param }}.ptr` with no
                // cast — the wrapper struct's `.ptr` field is already declared as the exact C
                // type cgo expects, so a `c_type` value here is dead weight, not a live cast. ~keep
                out.push_str(&crate::backends::go::template_env::render(
                    "param_opaque_cast.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else if ffi_param_enum_names.contains(name) {
                let enum_snake = c_symbols::type_component(name);
                out.push_str(&crate::backends::go::template_env::render(
                    "param_enum_to_i32.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        ffi_prefix => ffi_prefix,
                        enum_snake => &enum_snake,
                    },
                ));
                out.push('\n');
            } else if enum_names.contains(name) {
                let type_snake = c_symbols::type_component(name);
                let last_error_context_fn = c_symbols::last_error_context_symbol(ffi_prefix);
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                let from_json_err_action = if can_return_error {
                    format!("return {err_return_prefix}wrapLastError(\"failed to create {type_snake}\")")
                } else {
                    format!("panic(\"failed to create {type_snake}: \" + C.GoString(C.{last_error_context_fn}()))")
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "param_named_type.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        err_action => &err_action,
                        from_json_err_action => &from_json_err_action,
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                    },
                ));
                out.push('\n');
            } else {
                let type_snake = c_symbols::type_component(name);
                let last_error_context_fn = c_symbols::last_error_context_symbol(ffi_prefix);
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                let from_json_err_action = if can_return_error {
                    format!("return {err_return_prefix}wrapLastError(\"failed to create {type_snake}\")")
                } else {
                    format!("panic(\"failed to create {type_snake}: \" + C.GoString(C.{last_error_context_fn}()))")
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "param_named_type.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        err_action => &err_action,
                        from_json_err_action => &from_json_err_action,
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Json => {
            let err_action = if can_return_error {
                format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
            } else {
                "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
            };
            out.push_str(&crate::backends::go::template_env::render(
                "param_vec_or_map.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_param => &go_param,
                    err_action => &err_action,
                    empty_literal => empty_collection_wire_literal(param),
                },
            ));
            out.push('\n');
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                let c_type = format!("{}{}", c_consumer::export_type_prefix(ffi_prefix), name);
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_opaque.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        c_type => &c_type,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            TypeRef::Named(_) => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_named_inline.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            _ => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_decl.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                    },
                ));
                out.push('\n');
            }
        },
        TypeRef::Primitive(prim) if !param.optional => {
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            if matches!(prim, crate::core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_primitive_numeric.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_ty => &go_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Primitive(prim) if param.optional => {
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            let sentinel = primitive_max_sentinel(prim);

            if matches!(prim, crate::core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_primitive_numeric.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_ty => &go_ty,
                        sentinel => &sentinel,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        _ => {}
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}
