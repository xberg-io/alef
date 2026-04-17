use crate::generators::binding_helpers::{
    gen_async_body, gen_call_args, gen_call_args_with_let_bindings, gen_named_let_bindings, gen_serde_let_bindings,
    gen_unimplemented_body, has_named_params,
};
use crate::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::shared::{function_params, function_sig_defaults};
use crate::type_mapper::TypeMapper;
use ahash::{AHashMap, AHashSet};
use alef_core::ir::{ApiSurface, FunctionDef, TypeRef};
use std::fmt::Write;

/// Generate a free function.
pub fn gen_function(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
) -> String {
    let map_fn = |ty: &alef_core::ir::TypeRef| mapper.map_type(ty);
    let params = function_params(&func.params, &map_fn);
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    // Use let-binding pattern for non-opaque Named params so core fns can take &CoreType
    let use_let_bindings = has_named_params(&func.params, opaque_types);
    let call_args = if use_let_bindings {
        gen_call_args_with_let_bindings(&func.params, opaque_types)
    } else {
        gen_call_args(&func.params, opaque_types)
    };
    let core_import = cfg.core_import;
    let let_bindings = if use_let_bindings {
        gen_named_let_bindings(&func.params, opaque_types, core_import)
    } else {
        String::new()
    };

    // Use the function's rust_path for correct module path resolution
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    let can_delegate = crate::shared::can_auto_delegate_function(func, opaque_types);

    // Backend-specific error conversion string for serde bindings
    let serde_err_conv = match cfg.async_pattern {
        AsyncPattern::Pyo3FutureIntoPy => ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))",
        AsyncPattern::NapiNativeAsync => ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))",
        AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
        _ => ".map_err(|e| e.to_string())",
    };

    // Generate the body based on async pattern
    let body = if !can_delegate {
        // Check if an adapter provides the body
        if let Some(adapter_body) = adapter_bodies.get(&func.name) {
            adapter_body.clone()
        } else if cfg.has_serde && use_let_bindings && func.error_type.is_some() {
            // MARKER_SERDE_PATH
            // Serde-based param conversion: serialize binding types to JSON, deserialize to core types.
            // This handles Named params (e.g., ProcessConfig) that lack binding→core From impls.
            // For async functions with Pyo3FutureIntoPy, serde bindings use indented format.
            let is_async_pyo3 = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
            let (serde_indent, serde_err_async) = if is_async_pyo3 {
                (
                    "        ",
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))",
                )
            } else {
                ("    ", serde_err_conv)
            };
            let serde_bindings =
                gen_serde_let_bindings(&func.params, opaque_types, core_import, serde_err_async, serde_indent);
            let core_call = format!("{core_fn_path}({call_args})");

            // Determine return wrapping strategy for serde async (uses explicit types to avoid E0283)
            let returns_ref = func.returns_ref;
            let wrap_return = |expr: &str| -> String {
                match &func.return_type {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        if returns_ref {
                            format!("{name} {{ inner: Arc::new({expr}.clone()) }}")
                        } else {
                            format!("{name} {{ inner: Arc::new({expr}) }}")
                        }
                    }
                    TypeRef::Named(_) => {
                        // Use explicit type with ::from() to avoid E0283 type inference issues in async context
                        if returns_ref {
                            format!("{return_type}::from({expr}.clone())")
                        } else {
                            format!("{return_type}::from({expr})")
                        }
                    }
                    TypeRef::String | TypeRef::Bytes => format!("{expr}.into()"),
                    TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
                    TypeRef::Json => format!("{expr}.to_string()"),
                    _ => expr.to_string(),
                }
            };

            if is_async_pyo3 {
                // Async serde path: wrap everything in future_into_py
                let is_unit = matches!(func.return_type, TypeRef::Unit);
                let wrapped = wrap_return("result");
                let core_await = format!(
                    "{core_call}.await\n            .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?"
                );
                let inner_body = if is_unit {
                    format!("{serde_bindings}{core_await};\n            Ok(())")
                } else {
                    // When wrapped contains type conversions like .into() or ::from(),
                    // bind to a variable to help type inference for the generic future_into_py.
                    // This avoids E0283 "type annotations needed".
                    if wrapped.contains(".into()") || wrapped.contains("::from(") {
                        // Add explicit type annotation to help type inference
                        format!(
                            "{serde_bindings}let result = {core_await};\n            let wrapped_result: {return_type} = {wrapped};\n            Ok(wrapped_result)"
                        )
                    } else {
                        format!("{serde_bindings}let result = {core_await};\n            Ok({wrapped})")
                    }
                };
                format!("pyo3_async_runtimes::tokio::future_into_py(py, async move {{\n{inner_body}\n        }})")
            } else if func.is_async {
                // Async serde path for other backends (NAPI, etc.): use gen_async_body
                let is_unit = matches!(func.return_type, TypeRef::Unit);
                let wrapped = wrap_return("result");
                let async_body = gen_async_body(
                    &core_call,
                    cfg,
                    func.error_type.is_some(),
                    &wrapped,
                    false,
                    "",
                    is_unit,
                    Some(&return_type),
                );
                format!("{serde_bindings}{async_body}")
            } else if matches!(func.return_type, TypeRef::Unit) {
                // Unit return with error: avoid let_unit_value
                let await_kw = if func.is_async { ".await" } else { "" };
                let debug_marker = if func.is_async { "/*ASYNC_UNIT*/ " } else { "" };
                format!("{serde_bindings}{debug_marker}{core_call}{await_kw}{serde_err_conv}?;\n    Ok(())")
            } else {
                let wrapped = wrap_return("val");
                let await_kw = if func.is_async { ".await" } else { "" };
                if wrapped == "val" {
                    format!("{serde_bindings}{core_call}{await_kw}{serde_err_conv}")
                } else {
                    format!("{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){serde_err_conv}")
                }
            }
        } else {
            // Function can't be auto-delegated — return a default/error based on return type
            gen_unimplemented_body(
                &func.return_type,
                &func.name,
                func.error_type.is_some(),
                cfg,
                &func.params,
            )
        }
    } else if func.is_async {
        // MARKER_DELEGATE_ASYNC
        let core_call = format!("{core_fn_path}({call_args})");
        // In async contexts (future_into_py, etc.), the compiler often can't infer the
        // target type for .into(). Use explicit From::from() / collect::<Vec<T>>() instead.
        let return_wrap = match &func.return_type {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                format!("{n} {{ inner: Arc::new(result) }}")
            }
            TypeRef::Named(_) => {
                format!("{return_type}::from(result)")
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    format!("result.into_iter().map(|v| {n} {{ inner: Arc::new(v) }}).collect::<Vec<_>>()")
                }
                TypeRef::Named(_) => {
                    let inner_mapped = mapper.map_type(inner);
                    format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
                }
                _ => "result".to_string(),
            },
            TypeRef::Unit => "result".to_string(),
            _ => super::binding_helpers::wrap_return(
                "result",
                &func.return_type,
                "",
                opaque_types,
                false,
                func.returns_ref,
                false,
            ),
        };
        let async_body = gen_async_body(
            &core_call,
            cfg,
            func.error_type.is_some(),
            &return_wrap,
            false,
            "",
            matches!(func.return_type, TypeRef::Unit),
            Some(&return_type),
        );
        format!("{let_bindings}{async_body}")
    } else {
        let core_call = format!("{core_fn_path}({call_args})");

        // Determine return wrapping strategy
        let returns_ref = func.returns_ref;
        let wrap_return = |expr: &str| -> String {
            match &func.return_type {
                // Opaque type return: wrap in Arc
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    if returns_ref {
                        format!("{name} {{ inner: Arc::new({expr}.clone()) }}")
                    } else {
                        format!("{name} {{ inner: Arc::new({expr}) }}")
                    }
                }
                // Non-opaque Named: use .into() if From impl exists
                TypeRef::Named(_name) => {
                    if returns_ref {
                        format!("{expr}.clone().into()")
                    } else {
                        format!("{expr}.into()")
                    }
                }
                // String/Bytes: .into() handles &str→String etc.
                TypeRef::String | TypeRef::Bytes => format!("{expr}.into()"),
                // Path: PathBuf→String needs to_string_lossy
                TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
                // Json: serde_json::Value to string
                TypeRef::Json => format!("{expr}.to_string()"),
                // Optional with opaque inner
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        if returns_ref {
                            format!("{expr}.map(|v| {name} {{ inner: Arc::new(v.clone()) }})")
                        } else {
                            format!("{expr}.map(|v| {name} {{ inner: Arc::new(v) }})")
                        }
                    }
                    TypeRef::Named(_) => {
                        if returns_ref {
                            format!("{expr}.map(|v| v.clone().into())")
                        } else {
                            format!("{expr}.map(Into::into)")
                        }
                    }
                    TypeRef::String | TypeRef::Bytes | TypeRef::Path => {
                        format!("{expr}.map(Into::into)")
                    }
                    _ => expr.to_string(),
                },
                // Vec<Named>: map each element through Into
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        if returns_ref {
                            format!("{expr}.into_iter().map(|v| {name} {{ inner: Arc::new(v.clone()) }}).collect()")
                        } else {
                            format!("{expr}.into_iter().map(|v| {name} {{ inner: Arc::new(v) }}).collect()")
                        }
                    }
                    TypeRef::Named(_) => {
                        if returns_ref {
                            format!("{expr}.into_iter().map(|v| v.clone().into()).collect()")
                        } else {
                            format!("{expr}.into_iter().map(Into::into).collect()")
                        }
                    }
                    TypeRef::String | TypeRef::Bytes | TypeRef::Path => {
                        format!("{expr}.into_iter().map(Into::into).collect()")
                    }
                    _ => expr.to_string(),
                },
                _ => expr.to_string(),
            }
        };

        if func.error_type.is_some() {
            // Backend-specific error conversion
            let err_conv = match cfg.async_pattern {
                AsyncPattern::Pyo3FutureIntoPy => {
                    ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))"
                }
                AsyncPattern::NapiNativeAsync => {
                    ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))"
                }
                AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
                _ => ".map_err(|e| e.to_string())",
            };
            let wrapped = wrap_return("val");
            if wrapped == "val" {
                format!("{core_call}{err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            wrap_return(&core_call)
        }
    };

    // Prepend let bindings for non-opaque Named params (sync non-adapter case)
    let body = if !let_bindings.is_empty() && can_delegate && !func.is_async {
        format!("{let_bindings}{body}")
    } else {
        body
    };

    // Wrap long signature if necessary
    let async_kw = if func.is_async { "async " } else { "" };
    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    // For async PyO3 free functions, override return type and add lifetime generic.
    let ret = if func_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };
    let func_lifetime = if func_needs_py { "<'py>" } else { "" };

    let (func_sig, _params_formatted) = if params.len() > 100 {
        // When formatting for long signatures, promote optional params like function_params() does
        let mut seen_optional = false;
        let wrapped_params = func
            .params
            .iter()
            .map(|p| {
                if p.optional {
                    seen_optional = true;
                }
                let ty = if p.optional || seen_optional {
                    format!("Option<{}>", mapper.map_type(&p.ty))
                } else {
                    mapper.map_type(&p.ty)
                };
                format!("{}: {}", p.name, ty)
            })
            .collect::<Vec<_>>()
            .join(",\n    ");

        // For async PyO3, we need special signature handling
        if func_needs_py {
            (
                format!(
                    "pub fn {}{func_lifetime}(py: Python<'py>,\n    {}\n) -> {ret}",
                    func.name,
                    wrapped_params,
                    ret = ret
                ),
                "",
            )
        } else {
            (
                format!(
                    "pub {async_kw}fn {}(\n    {}\n) -> {ret}",
                    func.name,
                    wrapped_params,
                    ret = ret
                ),
                "",
            )
        }
    } else if func_needs_py {
        (
            format!(
                "pub fn {}{func_lifetime}(py: Python<'py>, {params}) -> {ret}",
                func.name
            ),
            "",
        )
    } else {
        (format!("pub {async_kw}fn {}({params}) -> {ret}", func.name), "")
    };

    let mut out = String::with_capacity(1024);
    // Per-item clippy suppression: too_many_arguments when >7 params (including py)
    let total_params = func.params.len() + if func_needs_py { 1 } else { 0 };
    if total_params > 7 {
        writeln!(out, "#[allow(clippy::too_many_arguments)]").ok();
    }
    // Per-item clippy suppression: missing_errors_doc for Result-returning functions
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');
    writeln!(out, "#[{attr_inner}]").ok();
    if cfg.needs_signature {
        let sig = function_sig_defaults(&func.params);
        writeln!(out, "{}{}{}", cfg.signature_prefix, sig, cfg.signature_suffix).ok();
    }
    write!(out, "{} {{\n    {body}\n}}", func_sig,).ok();
    out
}

/// Collect all unique trait import paths from types' methods.
///
/// Returns a deduplicated, sorted list of trait paths (e.g. `["liter_llm::LlmClient"]`)
/// that need to be imported in generated binding code so that trait methods can be called.
/// Both opaque and non-opaque types are scanned because non-opaque wrapper types also
/// delegate trait method calls to their inner core type.
pub fn collect_trait_imports(api: &ApiSurface) -> Vec<String> {
    let mut traits: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if let Some(ref trait_path) = method.trait_source {
                traits.insert(trait_path.clone());
            }
        }
    }
    let mut sorted: Vec<String> = traits.into_iter().collect();
    sorted.sort();
    sorted
}

/// Check if any type has methods from trait impls whose trait_source could not be resolved.
///
/// When true, the binding crate should add a glob import of the core crate (e.g.
/// `use kreuzberg::*`) to bring all publicly exported traits into scope.
/// This handles traits defined in private submodules that are re-exported.
pub fn has_unresolved_trait_methods(api: &ApiSurface) -> bool {
    // Count method names that appear on multiple non-trait types but lack trait_source.
    // Such methods likely come from trait impls whose trait path could not be resolved
    // (e.g. traits defined in private modules but re-exported via `pub use`).
    let mut method_counts: AHashMap<&str, (usize, usize)> = AHashMap::new(); // (total, with_source)
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if typ.is_trait {
            continue;
        }
        for method in &typ.methods {
            let entry = method_counts.entry(&method.name).or_insert((0, 0));
            entry.0 += 1;
            if method.trait_source.is_some() {
                entry.1 += 1;
            }
        }
    }
    // A method appearing on 3+ types without trait_source on any is almost certainly a trait method
    method_counts
        .values()
        .any(|&(total, with_source)| total >= 3 && with_source == 0)
}

/// Collect explicit type and enum names from the API surface for named imports.
///
/// Returns a sorted, deduplicated list of type and enum names that should be
/// imported from the core crate. This replaces glob imports (`use core::*`)
/// which can cause name conflicts with local binding definitions (e.g. a
/// `convert` function or `Result` type alias from the core crate shadowing
/// the binding's own `convert` wrapper or `std::result::Result`).
///
/// Only struct/enum names are included — functions and type aliases are
/// intentionally excluded because they are the source of conflicts.
pub fn collect_explicit_core_imports(api: &ApiSurface) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        names.insert(typ.name.clone());
    }
    for e in &api.enums {
        names.insert(e.name.clone());
    }
    names.into_iter().collect()
}
