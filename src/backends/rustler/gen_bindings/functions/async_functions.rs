use super::shared::{
    render_async_body, render_deser_line, render_named_deser_line, render_preamble, resolve_core_type_path,
};
use crate::backends::rustler::gen_bindings::types::gen_rustler_wrap_return;
use crate::backends::rustler::template_env;
use crate::backends::rustler::type_map::RustlerMapper;
use crate::codegen::doc_emission;
use crate::codegen::shared;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FunctionDef, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Generate a Rustler NIF async free function (sync wrapper scheduled on DirtyCpu).
pub(in crate::backends::rustler::gen_bindings) fn gen_nif_async_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    types_by_name: &AHashMap<&str, &TypeDef>,
) -> String {
    // If the Rust function name already ends with `_async` (e.g. `embed_texts_async`),
    // do not append another `_async` suffix — the NIF name is already the async variant.
    let nif_fn_name = if func.name.ends_with("_async") {
        func.name.clone()
    } else {
        format!("{}_async", func.name)
    };

    let params_str = func
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    return format!("{}: rustler::ResourceArc<{}>", p.name, n);
                }
                // Default (has_default) types are passed as JSON strings.
                if default_types.contains(n) {
                    return format!("{}: Option<String>", p.name);
                }
                if p.optional {
                    return format!("{}: Option<{}>", p.name, n);
                }
            }
            // Vec<Named> parameters (batch items like Vec<BatchBytesItem>) are
            // marshalled via JSON to avoid Rustler's limitation on decoding
            // complex struct lists.  Mirrors the sync_functions.rs path.
            if let TypeRef::Vec(inner) = &p.ty {
                if let TypeRef::Named(inner_name) = inner.as_ref() {
                    if !opaque_types.contains(inner_name.as_str()) {
                        return format!("{}: Option<String>", p.name);
                    }
                }
            }
            // Rustler 0.37 cannot marshal Vec<u8> from Erlang binaries;
            // use rustler::Binary for NIF function parameters.
            if matches!(&p.ty, TypeRef::Bytes) {
                return if p.optional {
                    format!("{}: Option<rustler::Binary>", p.name)
                } else {
                    format!("{}: rustler::Binary", p.name)
                };
            }
            let mapped = mapper.map_type(&p.ty);
            if p.optional {
                format!("{}: Option<{mapped}>", p.name)
            } else {
                format!("{}: {mapped}", p.name)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type =
        crate::backends::rustler::gen_bindings::helpers::map_return_type(&func.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let has_batch_vec_params = func.params.iter().any(|p| {
        if let TypeRef::Vec(inner) = &p.ty {
            if let TypeRef::Named(inner_name) = inner.as_ref() {
                return !opaque_types.contains(inner_name.as_str());
            }
        }
        false
    });

    let can_delegate =
        shared::can_auto_delegate_function(func, opaque_types) || has_default_params || has_batch_vec_params;

    let body = if can_delegate {
        let mut deser_lines: Vec<String> = Vec::new();
        // For async functions, rustler::Binary cannot be moved to spawn closure (not Send).
        // Convert to Vec<u8> (or &[u8]) before spawn so it can be moved into the closure.
        for p in &func.params {
            if matches!(&p.ty, TypeRef::Bytes) {
                // rustler::Binary borrows from the input Erlang binary; the borrow
                // cannot escape into a `'static` thread::spawn closure. Always
                // convert to an owned `Vec<u8>` (callers that take `&[u8]` re-borrow
                // from the owned buffer at the call site).
                deser_lines.push(render_named_deser_line("bytes_to_vec.rs.jinja", &p.name));
            }
        }
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = resolve_core_type_path(n, types_by_name, core_import);
                        deser_lines.push(render_deser_line(
                            "default_deser_with_error.rs.jinja",
                            &p.name,
                            &core_ty,
                        ));
                        // Handle based on whether core function expects reference or option
                        if p.optional {
                            // Core expects Option<T> → pass as-is
                            return format!("{}_core", p.name);
                        } else if p.is_ref && p.is_mut {
                            // Core expects &mut T → bind a mutable local, then borrow it
                            let mut_name = format!("{}_mut", p.name);
                            deser_lines.push(format!("let mut {mut_name} = {}_core.unwrap_or_default();", p.name));
                            return format!("&mut {mut_name}");
                        } else if p.is_ref {
                            // Core expects &T → use as_ref() to get Option<&T>, then unwrap
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            // Core expects T → unwrap or use default
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
                    }
                }
                // Vec<Named> (batch items): the NIF receives an Option<String> of JSON
                // — deserialize to Vec<core_ty> with an empty-vec fallback so the async
                // closure can move the owned Vec into the spawned task.  Mirrors the
                // sync_functions.rs preamble.
                if let TypeRef::Vec(inner) = &p.ty {
                    if let TypeRef::Named(inner_name) = inner.as_ref() {
                        if !opaque_types.contains(inner_name.as_str()) {
                            let inner_ty = resolve_core_type_path(inner_name, types_by_name, core_import);
                            let core_ty = format!("Vec<{inner_ty}>");
                            deser_lines.push(if func.error_type.is_some() {
                                format!(
                                    "let {pname}_core: {core_ty} = {pname}.map(|s| serde_json::from_str::<{core_ty}>(&s).map_err(|e| e.to_string())).transpose()?.unwrap_or_default();",
                                    pname = p.name,
                                )
                            } else {
                                format!(
                                    "let {pname}_core: {core_ty} = {pname}.and_then(|s| serde_json::from_str::<{core_ty}>(&s).ok()).unwrap_or_default();",
                                    pname = p.name,
                                )
                            });
                            return if p.is_ref {
                                format!("&{}_core", p.name)
                            } else {
                                format!("{}_core", p.name)
                            };
                        }
                    }
                }
                // AHashMap<Cow<'static, str>, Value> params: Rustler receives these as
                // HashMap<String, String> (BEAM maps decoded to Rust). We need a two-step conversion:
                // (1) bind an owned AHashMap to a named `let` before the call so we can borrow it,
                // (2) pass the reference in the call arg.
                if let TypeRef::Map(_, _) = &p.ty {
                    if p.map_is_ahash && p.map_key_is_cow {
                        let bound_name = format!("__{}_ahash", p.name);
                        deser_lines.push(format!(
                            "let {bound_name} = {}.map(|m| m.into_iter().map(|(k, v)| (std::borrow::Cow::Owned(k), serde_json::Value::String(v))).collect::<ahash::AHashMap<std::borrow::Cow<'static, str>, serde_json::Value>>());",
                            p.name
                        ));
                        return if p.optional && p.is_ref {
                            format!("{bound_name}.as_ref()")
                        } else if p.is_ref {
                            format!("{bound_name}.as_ref().unwrap()")
                        } else {
                            bound_name
                        };
                    }
                }
                match &p.ty {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        format!("&{}.inner", p.name)
                    }
                    TypeRef::Named(_) => {
                        if p.optional {
                            if p.is_ref {
                                format!("{}.as_ref().map(Into::into)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
                            }
                        } else if p.is_ref {
                            format!("&{}.clone().into()", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        }
                    }
                    // String params: handle optional and reference cases.
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char => p.name.clone(),
                    TypeRef::Path => {
                        if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => {
                        // After deser_lines, `content` is owned Vec<u8>. Re-borrow
                        // when the core fn takes &[u8], else move the Vec.
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.clone()
                        }
                    }
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(inner) => {
                        // Check if the vector element type is a Named type that needs conversion.
                        if let TypeRef::Named(elem_name) = inner.as_ref() {
                            if !opaque_types.contains(elem_name.as_str()) {
                                // The element type is a binding enum/struct that needs conversion.
                                // Convert each element via .into().
                                if p.is_ref && p.is_mut {
                                    // For &mut refs to Vec<Named>, create a mutable local binding.
                                    // The binding must be fully converted upfront so that the
                                    // core call can borrow it mutably without lifetime issues
                                    // from the closure environment.
                                    let mut_name = format!("{}_mut", p.name);
                                    deser_lines.push(format!("let mut {mut_name} = {}.iter().map(|e| e.clone().into()).collect::<Vec<_>>();", p.name));
                                    format!("&mut {mut_name}")
                                } else if p.is_ref {
                                    format!("&{}.iter().map(|e| e.clone().into()).collect::<Vec<_>>()", p.name)
                                } else {
                                    format!("{}.into_iter().map(Into::into).collect()", p.name)
                                }
                            } else if p.is_ref && p.is_mut {
                                // Opaque types with &mut: create mutable local binding for iter_mut().
                                let mut_name = format!("{}_mut", p.name);
                                deser_lines.push(format!("let mut {mut_name} = {}.clone();", p.name));
                                format!("&mut {mut_name}")
                            } else if p.is_ref {
                                // Opaque types: reference as-is, derefs to slice.
                                format!("&{}", p.name)
                            } else {
                                p.name.to_string()
                            }
                        } else if p.is_ref && p.is_mut {
                            // Non-Named element types with &mut: create mutable binding.
                            let mut_name = format!("{}_mut", p.name);
                            deser_lines.push(format!("let mut {mut_name} = {};", p.name));
                            format!("&mut {mut_name}")
                        } else if p.is_ref {
                            // Non-Named element types (String, etc.): reference as-is, derefs to slice.
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = render_preamble(&deser_lines);

        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({})", call_args.join(", "));
        let result_wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
        if func.error_type.is_some() {
            render_async_body("async_result_body.rs.jinja", &preamble, &core_call, &result_wrap)
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            render_async_body("async_infallible_body.rs.jinja", &preamble, &core_call, &result_wrap)
        }
    } else {
        crate::backends::rustler::gen_bindings::helpers::gen_rustler_unimplemented_body(
            &func.return_type,
            &nif_fn_name,
            true,
        )
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &func.doc, "");
    out.push_str(&template_env::render(
        "dirty_cpu_nif_function.rs.jinja",
        minijinja::context! {
            func_name => &nif_fn_name,
            params_str => &params_str,
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}
