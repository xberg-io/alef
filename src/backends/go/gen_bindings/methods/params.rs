use super::super::types::{cgo_type_for_primitive, primitive_max_sentinel};
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::go_type;
use crate::codegen::c_consumer;
use crate::codegen::naming::go_param_name;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};
use std::collections::HashSet;

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

/// Everything `gen_param_to_c` needs to know about the enclosing function besides the parameter.
///
/// `err_return_prefix` is the leading `"<zero>, "` (or `""` for value-less returns) prepended to
/// every `return ... fmt.Errorf(...)` early exit. Callers compute it from the enclosing function's
/// return type — `"nil, "` for pointer/slice/channel returns, `"0, "` / `"false, "` / `"\"\", "`
/// for plain primitive/string returns, and `""` when the function only returns `error`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
pub(in crate::backends::go::gen_bindings) struct GoParamCtx<'a> {
    pub(in crate::backends::go::gen_bindings) err_return_prefix: &'a str,
    pub(in crate::backends::go::gen_bindings) can_return_error: bool,
    pub(in crate::backends::go::gen_bindings) ffi_prefix: &'a str,
    pub(in crate::backends::go::gen_bindings) opaque_names: &'a HashSet<&'a str>,
    pub(in crate::backends::go::gen_bindings) ffi_param_enum_names: &'a HashSet<String>,
}

fn emit(out: &mut String, template: &str, context: minijinja::Value) {
    out.push_str(&crate::backends::go::template_env::render(template, context));
    out.push('\n');
}

fn emit_string_param(out: &mut String, optional: bool, c_name: &str, go_param: &str) {
    let template = if optional {
        "param_string_optional.jinja"
    } else {
        "param_string_required.jinja"
    };
    emit(
        out,
        template,
        crate::alef_context! { c_name => c_name, go_param => go_param },
    );
}

fn marshal_err_action(ctx: &GoParamCtx<'_>) -> String {
    if ctx.can_return_error {
        format!(
            "return {}fmt.Errorf(\"failed to marshal: %w\", err)",
            ctx.err_return_prefix
        )
    } else {
        "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
    }
}

fn emit_named_param(out: &mut String, name: &str, c_name: &str, go_param: &str, ctx: &GoParamCtx<'_>) {
    if ctx.opaque_names.contains(name) {
        // `param_opaque_cast.jinja` emits `{{ c_name }} := {{ go_param }}.ptr` with no
        // cast — the wrapper struct's `.ptr` field is already declared as the exact C
        // type cgo expects, so a `c_type` value here is dead weight, not a live cast. ~keep
        emit(
            out,
            "param_opaque_cast.jinja",
            crate::alef_context! { c_name => c_name, go_param => go_param },
        );
    } else if ctx.ffi_param_enum_names.contains(name) {
        let enum_snake = c_symbols::type_component(name);
        emit(
            out,
            "param_enum_to_i32.jinja",
            crate::alef_context! {
                c_name => c_name,
                go_param => go_param,
                ffi_prefix => ctx.ffi_prefix,
                enum_snake => &enum_snake,
            },
        );
    } else {
        let type_snake = c_symbols::type_component(name);
        let last_error_context_fn = c_symbols::last_error_context_symbol(ctx.ffi_prefix);
        let from_json_err_action = if ctx.can_return_error {
            format!(
                "return {}wrapLastError(\"failed to create {type_snake}\")",
                ctx.err_return_prefix
            )
        } else {
            format!("panic(\"failed to create {type_snake}: \" + C.GoString(C.{last_error_context_fn}()))")
        };
        emit(
            out,
            "param_named_type.jinja",
            crate::alef_context! {
                c_name => c_name,
                go_param => go_param,
                err_action => &marshal_err_action(ctx),
                from_json_err_action => &from_json_err_action,
                ffi_prefix => ctx.ffi_prefix,
                type_snake => &type_snake,
            },
        );
    }
}

fn emit_optional_param(out: &mut String, inner: &TypeRef, c_name: &str, go_param: &str, ctx: &GoParamCtx<'_>) {
    match inner {
        TypeRef::String | TypeRef::Char | TypeRef::Path => emit_string_param(out, true, c_name, go_param),
        TypeRef::Named(name) if ctx.opaque_names.contains(name.as_str()) => {
            let c_type = format!("{}{}", c_consumer::export_type_prefix(ctx.ffi_prefix), name);
            emit(
                out,
                "param_optional_opaque.jinja",
                crate::alef_context! { c_name => c_name, c_type => &c_type, go_param => go_param },
            );
        }
        TypeRef::Named(_) => emit(
            out,
            "param_optional_named_inline.jinja",
            crate::alef_context! { c_name => c_name, go_param => go_param },
        ),
        _ => emit(
            out,
            "param_optional_decl.jinja",
            crate::alef_context! { c_name => c_name },
        ),
    }
}

fn emit_primitive_param(out: &mut String, prim: &PrimitiveType, optional: bool, c_name: &str, go_param: &str) {
    let cgo_ty = cgo_type_for_primitive(prim);
    let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
    let is_bool = matches!(prim, PrimitiveType::Bool);
    if optional {
        if is_bool {
            emit(
                out,
                "param_optional_primitive_bool.jinja",
                crate::alef_context! { c_name => c_name, cgo_ty => &cgo_ty, go_param => go_param },
            );
        } else {
            let sentinel = primitive_max_sentinel(prim);
            emit(
                out,
                "param_optional_primitive_numeric.jinja",
                crate::alef_context! {
                    c_name => c_name,
                    cgo_ty => &cgo_ty,
                    go_ty => &go_ty,
                    sentinel => &sentinel,
                    go_param => go_param,
                },
            );
        }
    } else if is_bool {
        emit(
            out,
            "param_primitive_bool.jinja",
            crate::alef_context! { c_name => c_name, cgo_ty => &cgo_ty, go_param => go_param },
        );
    } else {
        emit(
            out,
            "param_primitive_numeric.jinja",
            crate::alef_context! { c_name => c_name, cgo_ty => &cgo_ty, go_ty => &go_ty, go_param => go_param },
        );
    }
}

/// Generate parameter conversion code from Go to C.
pub(in crate::backends::go::gen_bindings) fn gen_param_to_c(param: &ParamDef, ctx: &GoParamCtx<'_>) -> String {
    let mut out = String::with_capacity(512);
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));

    match &param.ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path => {
            emit_string_param(&mut out, param.optional, &c_name, &go_param)
        }
        TypeRef::Bytes => emit(
            &mut out,
            "bytes_to_c_pointer.jinja",
            crate::alef_context! { c_name => &c_name, go_param => &go_param },
        ),
        TypeRef::Named(name) => emit_named_param(&mut out, name, &c_name, &go_param, ctx),
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Json => emit(
            &mut out,
            "param_vec_or_map.jinja",
            crate::alef_context! {
                c_name => &c_name,
                go_param => &go_param,
                err_action => &marshal_err_action(ctx),
                empty_literal => empty_collection_wire_literal(param),
            },
        ),
        TypeRef::Optional(inner) => emit_optional_param(&mut out, inner, &c_name, &go_param, ctx),
        TypeRef::Primitive(prim) => emit_primitive_param(&mut out, prim, param.optional, &c_name, &go_param),
        _ => {}
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}
