use crate::backends::swift::gen_rust_crate::type_bridge::swift_bridge_rust_type;
use crate::core::ir::{ApiSurface, MethodDef, ParamDef, TypeRef};
use heck::ToSnakeCase;

use super::{inbound_bridge_type, needs_inbound_json_bridge};

/// Returns true if `ty` references a `Named(name)` at any depth where `name` resolves
/// to a trait — either present in `api.types` or stripped from the binding surface
/// (`api.excluded_trait_names`). Such methods return references to trait objects
/// (`&dyn Trait`, `Option<&dyn Trait>`, `Box<dyn Trait>`) which the Rust IR flattens
/// to `Named(name)`. They cannot be bridged across the Swift FFI, so the trait-bridge
/// generator skips them and falls back to the trait's default impl.
fn return_type_references_trait(ty: &TypeRef, api: &ApiSurface) -> bool {
    match ty {
        TypeRef::Named(name) => {
            api.types.iter().any(|t| t.is_trait && &t.name == name) || api.excluded_trait_names.contains(name)
        }
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => return_type_references_trait(inner, api),
        TypeRef::Map(k, v) => return_type_references_trait(k, api) || return_type_references_trait(v, api),
        _ => false,
    }
}

/// Emit one `impl Trait for SwiftWrapper` method body.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_inbound_method_impl(
    out: &mut String,
    method: &MethodDef,
    trait_snake: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
    emit_plugin: bool,
    lifetime_type_names: &std::collections::HashSet<String>,
    api: &ApiSurface,
) {
    if emit_plugin && return_type_references_trait(&method.return_type, api) {
        return;
    }

    let method_snake = method.name.to_snake_case();

    let receiver_token = match &method.receiver {
        Some(crate::core::ir::ReceiverKind::RefMut) => "&mut self",
        Some(crate::core::ir::ReceiverKind::Owned) => "self",
        _ => "&self",
    };
    let mut sig_params = vec![receiver_token.to_string()];
    for p in &method.params {
        let mut prefix = String::new();
        if p.is_ref {
            prefix.push('&');
        }
        if p.is_mut {
            prefix.push_str("mut ");
        }
        let inner_ty = if p.is_ref {
            match &p.ty {
                TypeRef::Vec(inner) => {
                    let elem = inbound_native_ty_owned(inner, source_crate, type_paths);
                    format!("[{elem}]")
                }
                TypeRef::Named(name) => {
                    let base = resolve_named_path(name, source_crate, type_paths);
                    if lifetime_type_names.contains(name.as_str()) {
                        format!("{base}<'_>")
                    } else {
                        base
                    }
                }
                other => inbound_native_ty(other, source_crate, type_paths),
            }
        } else {
            inbound_native_ty_owned(&p.ty, source_crate, type_paths)
        };
        let full_ty = if p.optional {
            format!("Option<{prefix}{inner_ty}>")
        } else {
            format!("{prefix}{inner_ty}")
        };
        sig_params.push(format!("{}: {full_ty}", p.name.to_snake_case()));
    }

    let return_ty = inbound_impl_return_type(method, source_crate, type_paths, error_type);

    let async_kw = if method.is_async { "async " } else { "" };
    let params = sig_params.join(", ");
    out.push_str(&crate::backends::swift::template_env::render(
        "inbound_method_open.rs.jinja",
        minijinja::context! {
            async_kw => async_kw,
            method_snake => &method_snake,
            params => &params,
            return_ty => &return_ty,
        },
    ));

    for p in &method.params {
        if let Some(line) = inbound_param_to_bridge(p) {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_binding.rs.jinja",
                minijinja::context! {
                    line => &line,
                },
            ));
        }
    }

    let call_args: Vec<String> = method.params.iter().map(inbound_local_name).collect();
    let call_expr = format!("self.inner.alef_{method_snake}({})", call_args.join(", "));

    let is_mime_types_pattern = method.returns_ref
        && matches!(&method.return_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));

    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_result_unit.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            ));
        } else {
            let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
            out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_result_value.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                    native_ty => &native_ty,
                },
            ));
        }
    } else if is_mime_types_pattern {
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_mime_types.rs.jinja",
            minijinja::context! {
                call_expr => &call_expr,
            },
        ));
    } else if needs_inbound_json_bridge(&method.return_type) {
        let native_ty = inbound_native_return_ty(&method.return_type, source_crate, type_paths);
        out.push_str(&crate::backends::swift::template_env::render(
            "inbound_method_json_return.rs.jinja",
            minijinja::context! {
                call_expr => &call_expr,
                native_ty => &native_ty,
                trait_snake => trait_snake,
                method_snake => &method_snake,
            },
        ));
    } else {
        match &method.return_type {
            TypeRef::Unit => out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_unit_call.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            )),
            _ => out.push_str(&crate::backends::swift::template_env::render(
                "inbound_method_value_call.rs.jinja",
                minijinja::context! {
                    call_expr => &call_expr,
                },
            )),
        }
    }

    out.push_str("    }\n\n");
}

/// Convert a trait param into its bridged FFI form via a `let` binding when needed.
fn inbound_param_to_bridge(p: &ParamDef) -> Option<String> {
    let local = inbound_local_name(p);
    let name = p.name.to_snake_case();

    if needs_inbound_json_bridge(&p.ty) {
        if p.optional {
            return Some(format!(
                "let {local} = {name}.map(|v| ::serde_json::to_string(&v).expect(\"serializable param {name}\"));"
            ));
        }
        return Some(format!(
            "let {local} = ::serde_json::to_string(&{name}).expect(\"serializable param {name}\");"
        ));
    }

    if p.optional {
        return match &p.ty {
            TypeRef::Path => Some(format!(
                "let {local} = {name}.map(|v| v.to_string_lossy().into_owned());"
            )),
            TypeRef::Bytes if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            TypeRef::String if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_string());")),
            TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.map(|v| v.to_vec());")),
            _ => None,
        };
    }

    match &p.ty {
        TypeRef::Path => Some(format!("let {local} = {name}.to_string_lossy().into_owned();")),
        TypeRef::Bytes => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_vec();"))
            } else {
                None
            }
        }
        TypeRef::String => {
            if p.is_ref {
                Some(format!("let {local} = {name}.to_string();"))
            } else {
                None
            }
        }
        TypeRef::Vec(_) if p.is_ref => Some(format!("let {local} = {name}.to_vec();")),
        _ => None,
    }
}

fn inbound_local_name(p: &ParamDef) -> String {
    p.name.to_snake_case()
}

/// FFI shim return type for `extern "Swift"` declarations.
///
/// Returns `String` for fallible methods (carrying a JSON envelope `{"ok": ...}` /
/// `{"err": "..."}`) instead of `Result<T, String>`. swift-bridge 0.1.59's
/// `Result<RustString, RustString>` codegen has a bug — `convert_ffi_result_ok_value_to_rust_value`
/// emits `result.ok_or_err` on a bare `*mut RustString` instead of the `ResultPtrAndPtr`
/// wrapper, producing `error[E0609]: no field 'ok_or_err' on type '*mut RustString'`.
/// Encoding the result as a JSON envelope sidesteps the limitation while preserving the
/// error-channel semantics; the Rust-side wrapper deserialises and reconstitutes the
/// `Result` after the FFI call.
pub(super) fn inbound_return_type(method: &MethodDef) -> String {
    if method.error_type.is_some() {
        return "String".to_string();
    }
    inbound_bridge_type(&method.return_type)
}

fn inbound_impl_return_type(
    method: &MethodDef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
    error_type: &str,
) -> String {
    if method.returns_ref {
        if let TypeRef::Vec(inner) = &method.return_type {
            let elem = match inner.as_ref() {
                TypeRef::String => "&'static str".to_string(),
                other => inbound_native_ty(other, source_crate, type_paths),
            };
            return format!("&'static [{elem}]");
        }
    }

    let inner = inbound_native_ty_owned(&method.return_type, source_crate, type_paths);
    if method.error_type.is_some() {
        if matches!(method.return_type, TypeRef::Unit) {
            result_type(source_crate, error_type, "()")
        } else {
            result_type(source_crate, error_type, &inner)
        }
    } else {
        inner
    }
}

pub(super) fn result_type(source_crate: &str, error_type: &str, ok_type: &str) -> String {
    format!(
        "std::result::Result<{ok_type}, {}>",
        error_type_path(source_crate, error_type)
    )
}

pub(super) fn error_type_path(source_crate: &str, error_type: &str) -> String {
    if error_type.contains("::") || error_type.contains('<') {
        error_type.to_string()
    } else {
        format!("{source_crate}::{error_type}")
    }
}

/// Resolve a Named type to its fully-qualified Rust path. Falls back to `{source_crate}::{name}`
/// when the lookup misses (covers shared types declared at the crate root).
fn resolve_named_path(
    name: &str,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    if let Some(path) = type_paths.get(name) {
        return path.replace('-', "_");
    }
    format!("{source_crate}::{name}")
}

/// Render the owned native return type (used in JSON-deserialise calls). Named types are
/// resolved via `type_paths`. Inner types in containers use the owned form.
fn inbound_native_return_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_return_ty(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_return_ty(k, source_crate, type_paths),
            inbound_native_return_ty(v, source_crate, type_paths)
        ),
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Render a TypeRef in its native (non-bridged) Rust form, qualifying Named types via
/// `type_paths`. Used for the `impl Trait` signature.
fn inbound_native_ty(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::Unit => "()".to_string(),
        TypeRef::String => "str".to_string(),
        TypeRef::Bytes => "[u8]".to_string(),
        TypeRef::Path => "::std::path::Path".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Json => "::serde_json::Value".to_string(),
        TypeRef::Duration => "::std::time::Duration".to_string(),
        TypeRef::Primitive(p) => primitive_str(p).to_string(),
        TypeRef::Named(name) => resolve_named_path(name, source_crate, type_paths),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_native_ty_owned(inner, source_crate, type_paths)),
        TypeRef::Map(k, v) => format!(
            "::std::collections::HashMap<{}, {}>",
            inbound_native_ty_owned(k, source_crate, type_paths),
            inbound_native_ty_owned(v, source_crate, type_paths)
        ),
    }
}

/// Owned form (for use inside `Vec`/`Option`/`HashMap`): swap unsized types (`str`,
/// `[u8]`, `Path`) with their owned equivalents.
fn inbound_native_ty_owned(
    ty: &TypeRef,
    source_crate: &str,
    type_paths: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeRef::String => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Path => "::std::path::PathBuf".to_string(),
        _ => inbound_native_ty(ty, source_crate, type_paths),
    }
}

fn primitive_str(p: &crate::core::ir::PrimitiveType) -> &'static str {
    use crate::core::ir::PrimitiveType::*;
    match p {
        Bool => "bool",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        Isize => "isize",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        Usize => "usize",
        F32 => "f32",
        F64 => "f64",
    }
}
