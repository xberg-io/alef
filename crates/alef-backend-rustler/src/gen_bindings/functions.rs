use super::types::gen_rustler_wrap_return;
use crate::type_map::RustlerMapper;
use ahash::AHashSet;
use alef_codegen::shared;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, TypeRef};

/// Build call argument expressions for Rustler opaque method (receiver is `resource`).
pub(super) fn gen_rustler_method_call_args(params: &[ParamDef], opaque_types: &AHashSet<String>) -> String {
    params
        .iter()
        .map(|p| match &p.ty {
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
            TypeRef::Bytes => format!("&{}", p.name),
            TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
            TypeRef::Vec(_) => {
                if p.is_ref {
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
                        // Optional JSON string → Option<CoreType> via serde
                        deser_lines.push(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s)).transpose().map_err(|e| e.to_string())?;",
                            p.name, core_ty
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
                    TypeRef::Bytes => format!("&{}", p.name),
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            // Vec<T> where core expects &[T] → pass as slice
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = if deser_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n    ", deser_lines.join("\n    "))
        };

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
            format!("{preamble}let result = {core_call}.map_err(|e| e.to_string())?;\n    Ok({wrap})")
        } else {
            format!(
                "{preamble}{}",
                gen_rustler_wrap_return(&core_call, &func.return_type, "", opaque_types, func.returns_ref)
            )
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
                        deser_lines.push(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s)).transpose().map_err(|e| e.to_string())?;",
                            p.name, core_ty
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
                    deser_lines.push(format!(
                        "let {0}_json = serde_json::to_string(&{0}).map_err(|e| e.to_string())?;",
                        p.name
                    ));
                    deser_lines.push(format!(
                        "let {0}_core: {1} = serde_json::from_str(&{0}_json).map_err(|e| e.to_string())?;",
                        p.name, core_ty
                    ));
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
                    TypeRef::Bytes => format!("&{}", p.name),
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = if deser_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n    ", deser_lines.join("\n    "))
        };

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
        format!("{preamble}let result = {core_call}.map_err(|e| e.to_string())?;\n    Ok({wrap})")
    } else {
        super::helpers::gen_rustler_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some())
    };
    format!(
        "#[rustler::nif]\npub fn {}({params_str}) -> {return_annotation} {{\n    \
         {body}\n}}",
        func.name
    )
}

/// Generate a Rustler NIF async free function (sync wrapper scheduled on DirtyCpu).
pub(super) fn gen_nif_async_function(
    func: &FunctionDef,
    mapper: &RustlerMapper,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    core_import: &str,
) -> String {
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
        let call_args: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        deser_lines.push(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s)).transpose().map_err(|e| e.to_string())?;",
                            p.name, core_ty
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
                    TypeRef::String | TypeRef::Char if p.optional => {
                        p.name.to_string()
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => {
                        format!("&{}", p.name)
                    }
                    TypeRef::String | TypeRef::Char => {
                        p.name.clone()
                    }
                    TypeRef::Path => {
                        if p.is_ref {
                            format!("&std::path::PathBuf::from({})", p.name)
                        } else {
                            format!("std::path::PathBuf::from({})", p.name)
                        }
                    }
                    TypeRef::Bytes => format!("&{}", p.name),
                    TypeRef::Duration => format!("std::time::Duration::from_millis({})", p.name),
                    TypeRef::Vec(_) => {
                        if p.is_ref {
                            format!("&{}", p.name)
                        } else {
                            p.name.to_string()
                        }
                    }
                    _ => p.name.clone(),
                }
            })
            .collect();

        let preamble = if deser_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n    ", deser_lines.join("\n    "))
        };

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
            format!(
                "{preamble}let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| e.to_string())?;\n    \
                 Ok({result_wrap})"
            )
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            format!(
                "{preamble}let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 Ok({result_wrap})"
            )
        }
    } else {
        super::helpers::gen_rustler_unimplemented_body(&func.return_type, &format!("{}_async", func.name), true)
    };
    format!(
        "#[rustler::nif(schedule = \"DirtyCpu\")]\npub fn {}_async({params_str}) -> {return_annotation} {{\n    \
         {body}\n\
         }}",
        func.name
    )
}

/// Generate a Rustler NIF method for a struct using the shared TypeMapper.
pub(super) fn gen_nif_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
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
        }
        let param_type = mapper.map_type(&p.ty);
        params.push(format!("{}: {}", p.name, param_type));
    }

    let return_type = super::helpers::map_return_type(&method.return_type, mapper, opaque_types);
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types);
        let core_call = if is_opaque && method.receiver.is_some() {
            format!("resource.inner.as_ref().clone().{}({})", method.name, call_args)
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            let inner_ty = format!("{core_import}::{struct_name}");
            format!("{inner_ty}::{}({})", method.name, call_args)
        } else if method.receiver.is_some() {
            // Instance method on non-opaque: convert binding struct to core type, then call
            format!(
                "{core_import}::{}::from(obj).{}({})",
                struct_name, method.name, call_args
            )
        } else {
            // Static method on non-opaque: call directly on core type.
            // Named (non-opaque) params use `.into()` which can be ambiguous when multiple
            // From impls exist. Emit explicit let bindings with annotated core types so
            // Rust can resolve the conversion without ambiguity.
            let named_params: Vec<&ParamDef> = method
                .params
                .iter()
                .filter(|p| matches!(&p.ty, TypeRef::Named(n) if !opaque_types.contains(n.as_str())))
                .collect();
            if named_params.is_empty() {
                format!("{core_import}::{}::{}({})", struct_name, method.name, call_args)
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
                        preamble.push_str(&format!("let {core_var}: {core_type} = {src};\n    "));
                        // Replace the generated expression in call_args with the variable name.
                        if p.optional {
                            resolved_args = resolved_args.replace(&format!("{}.map(Into::into)", p.name), &core_var);
                        } else {
                            resolved_args = resolved_args.replace(&format!("{}.into()", p.name), &core_var);
                        }
                    }
                }
                format!(
                    "{preamble}{core_import}::{}::{}({})",
                    struct_name, method.name, resolved_args
                )
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
            format!("let result = {core_call}.map_err(|e| e.to_string())?;\n    Ok({wrap})")
        } else {
            gen_rustler_wrap_return(
                &core_call,
                &method.return_type,
                struct_name,
                opaque_types,
                method.returns_ref,
            )
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
    format!(
        "#[rustler::nif]\npub fn {}({}) -> {} {{\n    \
         {body}\n}}",
        method_fn_name,
        params.join(", "),
        return_annotation
    )
}

/// Generate a Rustler NIF async method for a struct (sync wrapper scheduled on DirtyCpu).
pub(super) fn gen_nif_async_method(
    struct_name: &str,
    method: &MethodDef,
    mapper: &RustlerMapper,
    is_opaque: bool,
    opaque_types: &AHashSet<String>,
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
        }
        let param_type = mapper.map_type(&p.ty);
        params.push(format!("{}: {}", p.name, param_type));
    }

    let return_type = super::helpers::map_return_type(&method.return_type, mapper, opaque_types);
    // Async NIFs always return Result because Runtime::new() can fail, even when the core
    // method itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let can_delegate = shared::can_auto_delegate(method, opaque_types);

    let body = if can_delegate {
        let call_args = gen_rustler_method_call_args(&method.params, opaque_types);
        let core_call = if is_opaque && method.receiver.is_some() {
            format!("resource.inner.as_ref().clone().{}({})", method.name, call_args)
        } else if is_opaque {
            // Static method on opaque type: call directly on the inner core type
            let inner_ty = format!("{core_import}::{struct_name}");
            format!("{inner_ty}::{}({})", method.name, call_args)
        } else if method.receiver.is_some() {
            format!(
                "{core_import}::{}::from(obj).{}({})",
                struct_name, method.name, call_args
            )
        } else {
            // Static method on non-opaque: call directly on core type
            format!("{core_import}::{}::{}({})", struct_name, method.name, call_args)
        };
        let result_wrap = gen_rustler_wrap_return(
            "result",
            &method.return_type,
            struct_name,
            opaque_types,
            method.returns_ref,
        );
        if method.error_type.is_some() {
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }}).map_err(|e| e.to_string())?;\n    \
                 Ok({result_wrap})"
            )
        } else {
            // No error type, but Runtime::new() can still fail — use map_err and Ok().
            format!(
                "let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;\n    \
                 let result = rt.block_on(async {{ {core_call}.await }});\n    \
                 Ok({result_wrap})"
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
    format!(
        "#[rustler::nif(schedule = \"DirtyCpu\")]\npub fn {}({}) -> {} {{\n    \
         {body}\n\
         }}",
        method_fn_name,
        params.join(", "),
        return_annotation
    )
}
