use super::types::gen_rustler_wrap_return;
use crate::template_env;
use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::doc_emission;
use alef_codegen::shared;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, ReceiverKind, TypeRef};

fn render_deser_line(template_name: &str, name: &str, core_type: &str) -> String {
    template_env::render(
        template_name,
        minijinja::context! {
            name => name,
            core_type => core_type,
        },
    )
    .trim_end()
    .to_string()
}

fn render_named_deser_line(template_name: &str, name: &str) -> String {
    template_env::render(
        template_name,
        minijinja::context! {
            name => name,
        },
    )
    .trim_end()
    .to_string()
}

fn render_preamble(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", lines.join("\n    "))
    }
}

fn render_result_body(preamble: &str, core_call: &str, wrap: &str) -> String {
    template_env::render(
        "nif_result_body.rs.jinja",
        minijinja::context! {
            preamble => preamble,
            core_call => core_call,
            wrap => wrap,
        },
    )
}

fn render_wrapped_body(preamble: &str, wrap: &str) -> String {
    template_env::render(
        "nif_wrapped_body.rs.jinja",
        minijinja::context! {
            preamble => preamble,
            wrap => wrap,
        },
    )
}

fn render_async_body(template_name: &str, preamble: &str, core_call: &str, result_wrap: &str) -> String {
    template_env::render(
        template_name,
        minijinja::context! {
            preamble => preamble,
            core_call => core_call,
            result_wrap => result_wrap,
        },
    )
}

fn render_method_call(
    template_name: &str,
    core_import: &str,
    struct_name: &str,
    method_name: &str,
    call_args: &str,
) -> String {
    template_env::render(
        template_name,
        minijinja::context! {
            core_import => core_import,
            struct_name => struct_name,
            method_name => method_name,
            call_args => call_args,
        },
    )
    .trim_end()
    .to_string()
}

fn render_method_call_with_preamble(
    preamble: &str,
    core_import: &str,
    struct_name: &str,
    method_name: &str,
    call_args: &str,
) -> String {
    template_env::render(
        "rust_method_static_call_with_preamble.rs.jinja",
        minijinja::context! {
            preamble => preamble,
            core_import => core_import,
            struct_name => struct_name,
            method_name => method_name,
            call_args => call_args,
        },
    )
    .trim_end()
    .to_string()
}

/// Build call argument expressions for Rustler opaque method (receiver is `resource`).
pub(super) fn gen_rustler_method_call_args(
    params: &[ParamDef],
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
            TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                format!("&{}.inner", p.name)
            }
            // Default-typed Named params are passed as Option<String> JSON and decoded
            // by the caller into a `{name}_core` local. Reference that local here.
            TypeRef::Named(name) if default_types.contains(name.as_str()) => {
                if p.optional {
                    format!("{}_core", p.name)
                } else if p.is_ref {
                    format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name)
                } else {
                    format!("{}_core.unwrap_or_default()", p.name)
                }
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
            TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                format!("{}.as_deref()", p.name)
            }
            TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
            TypeRef::String | TypeRef::Char => p.name.clone(),
            TypeRef::Path => {
                if p.is_ref {
                    format!("&std::path::PathBuf::from({})", p.name)
                } else {
                    format!("std::path::PathBuf::from({})", p.name)
                }
            }
            TypeRef::Bytes => {
                if p.is_ref {
                    format!("{}.as_slice()", p.name)
                } else {
                    format!("{}.as_slice().to_vec()", p.name)
                }
            }
            TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
            TypeRef::Vec(_) => {
                if p.is_ref {
                    // `&Vec<T>` derefs to `&[T]`, which matches kreuzberg core for `&[String]`.
                    // For `&[&str]` signatures (Vec<String> inner), a refs intermediate is
                    // emitted in the caller body (gen_nif_function deser_lines) instead.
                    format!("&{}", p.name)
                } else {
                    p.name.to_string()
                }
            }
            _ => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate a Rustler NIF free function using the shared TypeMapper.
pub(super) fn gen_nif_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    cpu_bound_functions: &AHashSet<String>,
) -> String {
    let params_str = func
        .params
        .iter()
        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    return format!("{}: rustler::ResourceArc<{}>", p.name, n);
                }
                // Default (has_default) types are passed as JSON strings so that
                // partial maps work — serde_json::from_str respects #[serde(default)].
                if default_types.contains(n) {
                    return format!("{}: Option<String>", p.name);
                }
                if p.optional {
                    return format!("{}: Option<{}>", p.name, n);
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
                format!("{}: Option<{}>", p.name, mapped)
            } else {
                format!("{}: {}", p.name, mapped)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let return_type = super::helpers::map_return_type(&func.return_type, mapper, opaque_types);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    // A function can be auto-delegated when all params (after JSON deserialization)
    // map cleanly to core types.  We treat default-typed params as delegatable by
    // building the JSON deserialization preamble ourselves.
    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types) || has_default_params;

    let body = if can_delegate {
        // Build per-param deserialization lines and call-arg expressions.
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        // Optional JSON string → Option<CoreType> via serde.
                        // For Result-returning fns we use `?` to surface deser errors;
                        // for non-Result fns (e.g. `from`/`default` static helpers) we
                        // fall back to `.ok().flatten()` so a malformed JSON yields
                        // None instead of an unrecoverable panic from `?`.
                        let deser_line = if func.error_type.is_some() {
                            render_deser_line("default_deser_with_error.rs.jinja", &p.name, &core_ty)
                        } else {
                            render_deser_line("default_deser_without_error.rs.jinja", &p.name, &core_ty)
                        };
                        deser_lines.push(deser_line);
                        // Handle based on whether core function expects reference or option
                        if p.optional {
                            // Core expects Option<T> → pass as-is
                            return format!("{}_core", p.name);
                        } else if p.is_ref {
                            // Core expects &T → use as_ref() to get Option<&T>, then unwrap
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            // Core expects T → unwrap or use default
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
                    }
                }
                // Fall back to the standard call-arg logic for all other types.
                match &p.ty {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        format!("&{}.inner", p.name)
                    }
                    TypeRef::Named(_) => {
                        if p.optional {
                            if p.is_ref {
                                // Option<T> where core expects &T → use .as_ref()
                                format!("{}.as_ref().map(Into::into)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
                            }
                        } else if p.is_ref {
                            // T where core expects &T → take reference of converted value
                            format!("&{}.clone().into()", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        }
                    }
                    // String params: handle optional and reference cases.
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        // Option<String> where core expects Option<&str>
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => {
                        // Option<String> where core expects Option<String>
                        p.name.to_string()
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        // String where core expects &str
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char => {
                        // String where core expects String
                        p.name.clone()
                    }
                    TypeRef::Path => {
                        if p.is_ref {
                            // &Path expected → pass reference to PathBuf
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            // PathBuf expected
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => {
                        if p.is_ref {
                            format!("{}.as_slice()", p.name)
                        } else {
                            format!("{}.as_slice().to_vec()", p.name)
                        }
                    }
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(inner) if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                        // Core expects &[&str]; Vec<String> does not coerce, so build an intermediate refs vec.
                        if p.optional {
                            deser_lines.push(render_named_deser_line("vec_str_refs_optional.rs.jinja", &p.name));
                        } else {
                            deser_lines.push(render_named_deser_line("vec_str_refs_required.rs.jinja", &p.name));
                        }
                        format!("&{}_refs", p.name)
                    }
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            // &Vec<T> derefs to &[T] which matches kreuzberg core in all known sites.
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
        if func.error_type.is_some() {
            let wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
            render_result_body(&preamble, &core_call, &wrap)
        } else {
            let wrap = gen_rustler_wrap_return(&core_call, &func.return_type, "", opaque_types, func.returns_ref);
            render_wrapped_body(&preamble, &wrap)
        }
    } else if !func.sanitized && func.error_type.is_some() {
        // Serde recovery path: the function cannot be auto-delegated (e.g. a Named param is
        // passed by reference and has no From impl), but the function returns a Result so we
        // can propagate deserialization errors.  We serialize each non-opaque, non-default
        // Named binding param to JSON and deserialize directly into the matching core type.
        // This works because binding structs derive Serialize and core types derive Deserialize.
        let mut deser_lines: Vec<String> = Vec::new();
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if opaque_types.contains(n) {
                        return format!("&{}.inner", p.name);
                    }
                    if default_types.contains(n) {
                        // Default types already handled in the can_delegate branch above.
                        // They cannot appear here, but guard for completeness.
                        let core_ty = format!("{core_import}::{n}");
                        deser_lines.push(render_deser_line(
                            "default_deser_with_error.rs.jinja",
                            &p.name,
                            &core_ty,
                        ));
                        return if p.optional {
                            format!("{}_core", p.name)
                        } else if p.is_ref {
                            format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name)
                        } else {
                            format!("{}_core.unwrap_or_default()", p.name)
                        };
                    }
                    // Non-opaque Named param: round-trip via serde_json.
                    let core_ty = format!("{core_import}::{n}");
                    deser_lines.push(render_named_deser_line("named_param_to_json.rs.jinja", &p.name));
                    deser_lines.push(render_deser_line("named_param_from_json.rs.jinja", &p.name, &core_ty));
                    return if p.is_ref {
                        format!("&{}_core", p.name)
                    } else {
                        format!("{}_core", p.name)
                    };
                }
                // Non-Named params: use the same expressions as the delegate path.
                match &p.ty {
                    TypeRef::String | TypeRef::Char if p.optional && p.is_ref => {
                        format!("{}.as_deref()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.optional => p.name.to_string(),
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    TypeRef::String | TypeRef::Char => p.name.clone(),
                    TypeRef::Path => {
                        if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => {
                        if p.is_ref {
                            format!("{}.as_slice()", p.name)
                        } else {
                            format!("{}.as_slice().to_vec()", p.name)
                        }
                    }
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(inner) if p.is_ref && matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) => {
                        // Core expects &[&str]; Vec<String> does not coerce, so build an intermediate refs vec.
                        if p.optional {
                            deser_lines.push(render_named_deser_line("vec_str_refs_optional.rs.jinja", &p.name));
                        } else {
                            deser_lines.push(render_named_deser_line("vec_str_refs_required.rs.jinja", &p.name));
                        }
                        format!("&{}_refs", p.name)
                    }
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            // `&Vec<T>` derefs to `&[T]`, which is what kreuzberg core
                            // takes for `&[String]` callers.
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
        let wrap = gen_rustler_wrap_return("result", &func.return_type, "", opaque_types, func.returns_ref);
        render_result_body(&preamble, &core_call, &wrap)
    } else {
        super::helpers::gen_rustler_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some())
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &func.doc, "");
    let template_name = if cpu_bound_functions.contains(func.name.as_str()) {
        "dirty_cpu_nif_function.rs.jinja"
    } else {
        "nif_function.rs.jinja"
    };
    out.push_str(&template_env::render(
        template_name,
        minijinja::context! {
            func_name => &func.name,
            params_str => &params_str,
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}

/// Generate a Rustler NIF async free function (sync wrapper scheduled on DirtyCpu).
pub(super) fn gen_nif_async_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
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

    let return_type = super::helpers::map_return_type(&func.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let has_default_params = func
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));

    let can_delegate = shared::can_auto_delegate_function(func, opaque_types) || has_default_params;

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
                        let core_ty = format!("{core_import}::{n}");
                        deser_lines.push(render_deser_line(
                            "default_deser_with_error.rs.jinja",
                            &p.name,
                            &core_ty,
                        ));
                        // Handle based on whether core function expects reference or option
                        if p.optional {
                            // Core expects Option<T> → pass as-is
                            return format!("{}_core", p.name);
                        } else if p.is_ref {
                            // Core expects &T → use as_ref() to get Option<&T>, then unwrap
                            return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                        } else {
                            // Core expects T → unwrap or use default
                            return format!("{}_core.unwrap_or_default()", p.name);
                        }
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
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            // `&Vec<T>` derefs to `&[T]`, which is what kreuzberg core
                            // takes for every Vec param we've encountered so far. Rustler
                            // previously force-converted `Vec<String>` to `Vec<&str>` —
                            // that broke the `&[String]` callers (batch_reduce_tokens,
                            // chunk_texts_batch). If a future core fn wants `&[&str]`,
                            // handle it via an explicit conversion override at the call
                            // site rather than re-introducing the lossy default.
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
        super::helpers::gen_rustler_unimplemented_body(&func.return_type, &nif_fn_name, true)
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

/// Generate a Rustler NIF method for a struct using the shared TypeMapper.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_nif_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
) -> String {
    let method_fn_name = format!("{}_{}", struct_name.to_lowercase(), method.name);

    let mut params = if method.receiver.is_some() {
        if is_opaque {
            vec![format!("resource: rustler::ResourceArc<{}>", struct_name)]
        } else {
            vec![format!("obj: {}", struct_name)]
        }
    } else {
        vec![]
    };

    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                continue;
            }
            // Default (has_default) types are passed as JSON strings so partial maps
            // work — serde_json::from_str respects #[serde(default)]. Mirrors the
            // free-function pattern in `gen_nif_function`.
            if default_types.contains(n) {
                params.push(format!("{}: Option<String>", p.name));
                continue;
            }
            // Optional Named non-opaque params must be Option<T> so callers can
            // pass nil (Elixir) and the NIF receives None rather than a decode error.
            if p.optional {
                params.push(format!("{}: Option<{}>", p.name, n));
                continue;
            }
        }
        let param_type = mapper.map_type(&p.ty);
        if p.optional {
            params.push(format!("{}: Option<{}>", p.name, param_type));
        } else {
            params.push(format!("{}: {}", p.name, param_type));
        }
    }

    let return_type = super::helpers::map_return_type(&method.return_type, mapper, opaque_types);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let has_default_params = method
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));
    let can_delegate = shared::can_auto_delegate(method, opaque_types) || has_default_params;

    // Build deserialization preamble for default-typed (JSON-string) params.
    let deser_preamble =
        build_default_deser_preamble(&method.params, default_types, core_import, method.error_type.is_some());

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types, default_types);
        let core_call = if let (true, Some(receiver)) = (is_opaque, method.receiver.as_ref()) {
            // For &self: Arc<T> derefs to T, no clone needed (and avoids the
            // noop_method_call lint that the previous as_ref().clone() tripped).
            // For &mut self / self: clone the inner T to get an owned value the
            // method can consume — requires T: Clone (callers needing non-Clone
            // opaque types with mutating methods should configure exclude_methods).
            match receiver {
                ReceiverKind::Ref => format!("resource.inner.{}({})", method.name, call_args),
                ReceiverKind::RefMut | ReceiverKind::Owned => {
                    format!("(*resource.inner).clone().{}({})", method.name, call_args)
                }
            }
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            render_method_call(
                "rust_method_static_call.rs.jinja",
                core_import,
                struct_name,
                &method.name,
                &call_args,
            )
        } else if method.receiver.is_some() {
            // Instance method on non-opaque: convert binding struct to core type, then call
            render_method_call(
                "rust_method_instance_call.rs.jinja",
                core_import,
                struct_name,
                &method.name,
                &call_args,
            )
        } else {
            // Static method on non-opaque: call directly on core type.
            // Named (non-opaque) params use `.into()` which can be ambiguous when multiple
            // From impls exist. Emit explicit let bindings with annotated core types so
            // Rust can resolve the conversion without ambiguity.
            // Skip default-typed params — those are already deserialized by the
            // `deser_preamble` from `build_default_deser_preamble`, which produces
            // its own `{name}_core` binding. Emitting another would duplicate.
            let named_params: Vec<&ParamDef> = method
                .params
                .iter()
                .filter(|p| matches!(&p.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str()) && !default_types.contains(n.as_str())))
                .collect();
            if named_params.is_empty() {
                render_method_call(
                    "rust_method_static_call.rs.jinja",
                    core_import,
                    struct_name,
                    &method.name,
                    &call_args,
                )
            } else {
                // Build annotated let-bindings for each Named param and substitute in call_args.
                let mut preamble = String::new();
                let mut resolved_args = call_args.clone();
                for p in named_params {
                    if let TypeRef::Named(type_name) = &p.ty {
                        let core_var = format!("{}_core", p.name);
                        let core_type = format!("{core_import}::{type_name}");
                        let src = if p.optional {
                            format!("{}.map(Into::into)", p.name)
                        } else {
                            format!("{}.into()", p.name)
                        };
                        preamble.push_str(&template_env::render(
                            "rust_let_binding.jinja",
                            minijinja::context! {
                                var_name => &core_var,
                                var_type => &core_type,
                                expr => &src,
                            },
                        ));
                        // Replace the generated expression in call_args with the variable name.
                        if p.optional {
                            resolved_args = resolved_args.replace(&format!("{}.map(Into::into)", p.name), &core_var);
                        } else {
                            resolved_args = resolved_args.replace(&format!("{}.into()", p.name), &core_var);
                        }
                    }
                }
                render_method_call_with_preamble(&preamble, core_import, struct_name, &method.name, &resolved_args)
            }
        };
        if method.error_type.is_some() {
            let wrap = gen_rustler_wrap_return(
                "result",
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            );
            render_result_body(&deser_preamble, &core_call, &wrap)
        } else {
            let inner = gen_rustler_wrap_return(
                &core_call,
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            );
            if deser_preamble.is_empty() {
                inner
            } else {
                render_wrapped_body(&deser_preamble, &inner)
            }
        }
    } else {
        let adapter_key = format!("{struct_name}.{}", method.name);
        if let Some(body) = adapter_bodies.get(&adapter_key) {
            body.clone()
        } else {
            super::helpers::gen_rustler_unimplemented_body(
                &method.return_type,
                &method_fn_name,
                method.error_type.is_some(),
            )
        }
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &method.doc, "");
    out.push_str(&template_env::render(
        "nif_function.rs.jinja",
        minijinja::context! {
            func_name => &method_fn_name,
            params_str => &params.join(", "),
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}

/// Build the deserialization preamble for `Option<String>` JSON params that
/// correspond to default-typed core types. Returns an empty string when no
/// param needs JSON deserialization.
fn build_default_deser_preamble(
    params: &[ParamDef],
    default_types: &AHashSet<String>,
    core_import: &str,
    has_error: bool,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    for p in params {
        if let TypeRef::Named(n) = &p.ty {
            if default_types.contains(n) {
                let core_ty = format!("{core_import}::{n}");
                let line = if has_error {
                    render_deser_line("default_deser_with_error.rs.jinja", &p.name, &core_ty)
                } else {
                    render_deser_line("default_deser_without_error.rs.jinja", &p.name, &core_ty)
                };
                lines.push(line);
            }
        }
    }
    render_preamble(&lines)
}

/// Generate a Rustler NIF async method for a struct (sync wrapper scheduled on DirtyCpu).
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_nif_async_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
    adapter_bodies: &alef_adapters::AdapterBodies,
) -> String {
    let method_fn_name = format!("{}_{}_async", struct_name.to_lowercase(), method.name);

    let mut params = if method.receiver.is_some() {
        if is_opaque {
            vec![format!("resource: rustler::ResourceArc<{}>", struct_name)]
        } else {
            vec![format!("obj: {}", struct_name)]
        }
    } else {
        vec![]
    };

    for p in &method.params {
        if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                params.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                continue;
            }
            // Default (has_default) types are passed as JSON strings so partial maps work.
            if default_types.contains(n) {
                params.push(format!("{}: Option<String>", p.name));
                continue;
            }
            // Optional Named non-opaque params must be Option<T> so callers can
            // pass nil (Elixir) and the NIF receives None rather than a decode error.
            if p.optional {
                params.push(format!("{}: Option<{}>", p.name, n));
                continue;
            }
        }
        let param_type = mapper.map_type(&p.ty);
        if p.optional {
            params.push(format!("{}: Option<{}>", p.name, param_type));
        } else {
            params.push(format!("{}: {}", p.name, param_type));
        }
    }

    let return_type = super::helpers::map_return_type(&method.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // method itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let has_default_params = method
        .params
        .iter()
        .any(|p| matches!(&p.ty, TypeRef::Named(n) if default_types.contains(n)));
    let can_delegate = shared::can_auto_delegate(method, opaque_types) || has_default_params;

    // Build deserialization preamble for default-typed (JSON-string) params.
    let deser_preamble =
        build_default_deser_preamble(&method.params, default_types, core_import, method.error_type.is_some());

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types, default_types);
        let core_call = if let (true, Some(receiver)) = (is_opaque, method.receiver.as_ref()) {
            // For &self: Arc<T> derefs to T, no clone needed (and avoids the
            // noop_method_call lint that the previous as_ref().clone() tripped).
            // For &mut self / self: clone the inner T to get an owned value the
            // method can consume — requires T: Clone (callers needing non-Clone
            // opaque types with mutating methods should configure exclude_methods).
            match receiver {
                ReceiverKind::Ref => format!("resource.inner.{}({})", method.name, call_args),
                ReceiverKind::RefMut | ReceiverKind::Owned => {
                    format!("(*resource.inner).clone().{}({})", method.name, call_args)
                }
            }
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            render_method_call(
                "rust_method_static_call.rs.jinja",
                core_import,
                struct_name,
                &method.name,
                &call_args,
            )
        } else if method.receiver.is_some() {
            render_method_call(
                "rust_method_instance_call.rs.jinja",
                core_import,
                struct_name,
                &method.name,
                &call_args,
            )
        } else {
            // Static method on non-opaque: call directly on core type
            render_method_call(
                "rust_method_static_call.rs.jinja",
                core_import,
                struct_name,
                &method.name,
                &call_args,
            )
        };
        let result_wrap = gen_rustler_wrap_return(
            "result",
            &method.return_type,
            struct_name,
            opaque_types,
            method.returns_ref,
        );
        if method.error_type.is_some() {
            render_async_body("async_result_body.rs.jinja", &deser_preamble, &core_call, &result_wrap)
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            render_async_body(
                "async_infallible_body.rs.jinja",
                &deser_preamble,
                &core_call,
                &result_wrap,
            )
        }
    } else {
        let adapter_key = format!("{struct_name}.{}", method.name);
        if let Some(body) = adapter_bodies.get(&adapter_key) {
            body.clone()
        } else {
            super::helpers::gen_rustler_unimplemented_body(&method.return_type, &method_fn_name, true)
        }
    };
    let mut out = String::new();
    doc_emission::emit_rustdoc(&mut out, &method.doc, "");
    out.push_str(&template_env::render(
        "dirty_cpu_nif_function.rs.jinja",
        minijinja::context! {
            func_name => &method_fn_name,
            params_str => &params.join(", "),
            ret => &return_annotation,
            body => &body,
        },
    ));
    out
}
