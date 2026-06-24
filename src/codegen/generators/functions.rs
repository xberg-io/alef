use crate::codegen::generators::binding_helpers::{
    gen_async_body, gen_call_args, gen_call_args_cfg, gen_call_args_with_let_bindings_mutex_json_str,
    gen_named_let_bindings, gen_named_let_bindings_by_ref, gen_serde_let_bindings, gen_unimplemented_body,
    has_named_params,
};
use crate::codegen::generators::{AdapterBodies, AsyncPattern, RustBindingConfig};
use crate::codegen::shared::function_sig_defaults;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::{AHashMap, AHashSet};

/// Detect whether the core-call expression already evaluates to `Arc<T>` for the
/// binding's `inner` field. Used to avoid wrapping `Self { inner: Arc::new(self.inner.clone()) }`
/// where `self.inner` is already `Arc<T>`.
fn expr_is_already_arc(expr: &str) -> bool {
    let trimmed = expr.trim();
    trimmed == "self.inner"
        || trimmed == "self.inner.clone()"
        || trimmed.starts_with("self.inner.as_ref()")
        || trimmed.starts_with("self.inner.clone()")
}

/// Build the Arc-wrapping expression for an opaque type's `inner` field. Wraps in a
/// `Mutex` when the opaque type has `&mut self` methods (signalled by `mutex_types`).
fn arc_wrap_expr(val: &str, name: &str, mutex_types: &AHashSet<String>) -> String {
    if mutex_types.contains(name) {
        format!("Arc::new(std::sync::Mutex::new({val}))")
    } else {
        format!("Arc::new({val})")
    }
}

/// Compute the cast target for a leaf primitive given the active wide-integer cast flags.
///
/// Mirrors the extendr type mapper: large ints (`usize`/`u64`/`i64`/`isize`) map to `f64`
/// when `cast_large_ints_to_f64` is set, and small unsigned ints (`u8`/`u16`/`u32`) map to
/// `i32` when `cast_uints_to_i32` is set. Returns `None` for any primitive the mapper leaves
/// unchanged, so backends that do not set these flags (pyo3/napi/wasm) never trigger a cast.
fn wide_int_cast_target(
    prim: &crate::core::ir::PrimitiveType,
    cast_large_ints_to_f64: bool,
    cast_uints_to_i32: bool,
) -> Option<&'static str> {
    use crate::core::ir::PrimitiveType;
    match prim {
        PrimitiveType::Usize | PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Isize
            if cast_large_ints_to_f64 =>
        {
            Some("f64")
        }
        PrimitiveType::U8 | PrimitiveType::U16 | PrimitiveType::U32 if cast_uints_to_i32 => Some("i32"),
        _ => None,
    }
}

/// When a wide-integer cast flag rewrites the return type's leaf primitive to a different
/// R-representable type, cast the core-call result so the wrapper body matches the rendered
/// signature (e.g. body yields `Vec<usize>` but the signature says `Vec<f64>`).
///
/// Returns `None` when no cast is needed (no flags set, or the primitive is left unchanged by
/// the mapper). Handles the four shapes the mapper can rewrite: scalar `P`, `Vec<P>`,
/// `Option<P>`, and `Option<Vec<P>>`. `expr` must already be the unwrapped value (the `Ok`
/// payload / awaited value), never a `Result` or a serialized `String`.
fn cast_return_expr(
    ret: &TypeRef,
    expr: &str,
    cast_large_ints_to_f64: bool,
    cast_uints_to_i32: bool,
) -> Option<String> {
    if !cast_large_ints_to_f64 && !cast_uints_to_i32 {
        return None;
    }
    match ret {
        TypeRef::Primitive(prim) => {
            let target = wide_int_cast_target(prim, cast_large_ints_to_f64, cast_uints_to_i32)?;
            Some(format!("({expr}) as {target}"))
        }
        TypeRef::Vec(inner) => {
            let TypeRef::Primitive(prim) = inner.as_ref() else {
                return None;
            };
            let target = wide_int_cast_target(prim, cast_large_ints_to_f64, cast_uints_to_i32)?;
            Some(format!(
                "{expr}.into_iter().map(|v| v as {target}).collect::<Vec<{target}>>()"
            ))
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Primitive(prim) => {
                let target = wide_int_cast_target(prim, cast_large_ints_to_f64, cast_uints_to_i32)?;
                Some(format!("{expr}.map(|v| v as {target})"))
            }
            TypeRef::Vec(vinner) => {
                let TypeRef::Primitive(prim) = vinner.as_ref() else {
                    return None;
                };
                let target = wide_int_cast_target(prim, cast_large_ints_to_f64, cast_uints_to_i32)?;
                Some(format!(
                    "{expr}.map(|xs| xs.into_iter().map(|v| v as {target}).collect::<Vec<{target}>>())"
                ))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Generate a free function. Equivalent to `gen_function_with_mutex` with no mutex types.
pub fn gen_function(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
) -> String {
    gen_function_with_mutex(func, mapper, cfg, adapter_bodies, opaque_types, &AHashSet::new())
}

/// Generate a free function. `mutex_types` is the subset of opaque types whose `inner`
/// field is `Arc<Mutex<T>>` (because the type has `&mut self` methods); their
/// constructors emit `Arc::new(Mutex::new(v))` instead of `Arc::new(v)`.
pub fn gen_function_with_mutex(
    func: &FunctionDef,
    mapper: &dyn TypeMapper,
    cfg: &RustBindingConfig,
    adapter_bodies: &AdapterBodies,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
) -> String {
    let map_fn = |ty: &crate::core::ir::TypeRef| mapper.map_type(ty);
    // When named_non_opaque_params_by_ref is true (extendr backend), Named non-opaque struct
    // params must use references because extendr only generates TryFrom<&Robj> for &T.
    // - Required params: `&T` (extendr generates TryFrom<&Robj> for &T)
    // - Optional params: `Nullable<&T>` (extendr's Nullable<T: TryFrom<&Robj>>)
    // - Promoted-optional (required following optional): `Nullable<&T>` (treated as optional)
    // After the first optional/Nullable param, all subsequent params are also promoted.
    // Per-parameter `name: type` strings. Computed once here and reused for BOTH the
    // single-line and long-signature (multi-line wrapped) renderings below — the long path
    // must not recompute types, or it diverges from this backend-aware mapping (e.g. dropping
    // extendr's `Nullable<&T>` back to `Option<T>`) and produces a signature whose types
    // disagree with the generated body.
    let param_strings: Vec<String> = if cfg.named_non_opaque_params_by_ref {
        let mut seen_optional = false;
        func.params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                if p.optional {
                    seen_optional = true;
                }
                let promoted =
                    seen_optional && !p.optional && crate::codegen::shared::is_promoted_optional(&func.params, idx);
                let ty = match &p.ty {
                    TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => {
                        // Only genuinely-optional named params become `Nullable<&T>`. A param that is
                        // required in core but merely *follows* an optional one is NOT promoted for
                        // extendr: R imposes no "required-after-optional" ordering constraint (unlike
                        // PyO3), and `Nullable<&T>::into_option()` needs `&T: TryFrom<Robj>`, which
                        // extendr does not implement. Keeping it `&T` (required) matches the by-ref
                        // simple let-binding and the core signature.
                        let _ = promoted;
                        if p.optional {
                            format!("Nullable<&{}>", map_fn(&p.ty))
                        } else {
                            format!("&{}", map_fn(&p.ty))
                        }
                    }
                    TypeRef::Optional(inner) => {
                        // Check if inner is a non-opaque Named struct that should be passed by-ref
                        let inner_str_if_named = if let TypeRef::Named(n) = inner.as_ref() {
                            if !opaque_types.contains(n.as_str()) {
                                Some(n.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Some(inner_name) = inner_str_if_named {
                            // Optional non-opaque Named struct (e.g. a promoted trailing config param whose
                            // core signature is `&T` but which is exposed to R as omittable). extendr needs
                            // `Nullable<&Wrapper>` so the by-ref/promoted let-binding's `.into_option()`
                            // resolves and `TryFrom<&Robj>` is available; `Option<Wrapper>` breaks both.
                            format!("extendr_api::Nullable<&{}>", inner_name)
                        } else if p.optional || seen_optional {
                            format!("Option<{}>", map_fn(&p.ty))
                        } else {
                            map_fn(&p.ty)
                        }
                    }
                    _ => {
                        if p.optional || seen_optional {
                            format!("Option<{}>", map_fn(&p.ty))
                        } else {
                            map_fn(&p.ty)
                        }
                    }
                };
                format!("{}: {}", p.name, ty)
            })
            .collect::<Vec<_>>()
    } else {
        crate::codegen::shared::function_params_vec(&func.params, &map_fn)
    };
    let params = param_strings.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    // Use let-binding pattern for non-opaque Named params so core fns can take &CoreType.
    // The binding signature (above) already has `&T` for Named non-opaque params when
    // named_non_opaque_params_by_ref is true. We do NOT modify is_ref here — we keep
    // the original core function's is_ref so call args can respect by-ref vs by-value semantics.
    // The let-binding creates an owned `_core` value from the borrowed `&param`, and
    // call args will then apply the core function's actual is_ref to determine whether to
    // pass `_core` by-value or `&_core` by-ref.
    let effective_params: std::borrow::Cow<[crate::core::ir::ParamDef]> = std::borrow::Cow::Borrowed(&func.params);
    let use_let_bindings = has_named_params(&effective_params, opaque_types);
    let call_args = if use_let_bindings {
        // Use the mutex-aware variant so opaque params with is_ref && is_mut get
        // `&mut *{name}.inner.lock().unwrap()` instead of `&{name}.inner`. The shared free-function
        // generator is used by PyO3/extendr, whose binding Json type is `String`, so parse Json
        // params into serde_json::Value at the call site.
        gen_call_args_with_let_bindings_mutex_json_str(
            &effective_params,
            opaque_types,
            mutex_types,
            cfg.cast_uints_to_i32,
            cfg.cast_large_ints_to_f64,
        )
    } else if cfg.cast_uints_to_i32 || cfg.cast_large_ints_to_f64 {
        gen_call_args_cfg(
            &effective_params,
            opaque_types,
            cfg.cast_uints_to_i32,
            cfg.cast_large_ints_to_f64,
        )
    } else {
        gen_call_args(&effective_params, opaque_types)
    };
    let core_import = cfg.core_import;
    let let_bindings = if use_let_bindings {
        if cfg.named_non_opaque_params_by_ref {
            // Params are `&T` in the signature — use .clone().into() for conversion.
            gen_named_let_bindings_by_ref(&func.params, opaque_types, core_import)
        } else {
            gen_named_let_bindings(&func.params, opaque_types, core_import)
        }
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

    let can_delegate = crate::codegen::shared::can_auto_delegate_function(func, opaque_types)
        || can_delegate_with_named_let_bindings(func, opaque_types);

    // PyO3 sync free functions hold the GIL while calling into core. When core re-enters
    // Python via a registered trait callback (the bridge runs the host callback on a
    // `spawn_blocking` worker thread that re-acquires the GIL), the worker can never get the
    // GIL this thread holds while parked in the blocking call → deadlock. Release the GIL
    // for the duration of the blocking core call by wrapping it in `py.detach(|| ...)`. The
    // closure touches no Python objects (Rust args in, Rust value out); conversion to Python
    // happens after the call returns. The async path already releases the GIL via future_into_py.
    let pyo3_sync = !func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    let detach_core_call = |core_call: &str| -> String {
        if pyo3_sync {
            format!("py.detach(|| {core_call})")
        } else {
            core_call.to_string()
        }
    };

    // Backend-specific error conversion string for serde bindings
    let serde_err_conv = match cfg.async_pattern {
        AsyncPattern::Pyo3FutureIntoPy => ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))",
        AsyncPattern::NapiNativeAsync => ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))",
        AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))",
        AsyncPattern::TokioBlockOn => {
            ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
        }
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
            // For sync PyO3 the blocking core call is wrapped in `py.detach(|| ...)` to release
            // the GIL (no-op for async/other backends; the async path uses future_into_py).
            let core_call = detach_core_call(&format!("{core_fn_path}({call_args})"));

            // Determine return wrapping strategy for serde async (uses explicit types to avoid E0283)
            let returns_ref = func.returns_ref;
            let wrap_return = |expr: &str| -> String {
                // Cast wide-integer leaf primitives (extendr: usize/u64/i64/isize → f64,
                // u8/u16/u32 → i32) so the value matches the rendered signature. No-op for
                // backends without the cast flags; the unwrapped value is cast, never the Result.
                if let Some(cast) = cast_return_expr(
                    &func.return_type,
                    expr,
                    cfg.cast_large_ints_to_f64,
                    cfg.cast_uints_to_i32,
                ) {
                    return cast;
                }
                match &func.return_type {
                    TypeRef::Vec(inner) => {
                        // Vec<T>: check if elements need conversion
                        match inner.as_ref() {
                            TypeRef::Named(_) => {
                                // Vec<Named>: convert each element using Into::into
                                format!("{expr}.into_iter().map(Into::into).collect()")
                            }
                            _ => expr.to_string(),
                        }
                    }
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        let mapped_name = mapper.named(name);
                        if returns_ref {
                            format!("{mapped_name} {{ inner: Arc::new({expr}.clone()) }}")
                        } else {
                            format!("{mapped_name} {{ inner: Arc::new({expr}) }}")
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
                    // String/Bytes are identity across all backends (String->String,
                    // Vec<u8>->Vec<u8>) — no .into() needed for owned values.
                    TypeRef::String | TypeRef::Bytes => expr.to_string(),
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
                    if wrapped.contains(".into()") || wrapped.contains("::from(") || wrapped.contains("Into::into") {
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
                } else if wrapped == "val.into()" {
                    format!("{serde_bindings}{core_call}{await_kw}.map(Into::into){serde_err_conv}")
                } else if let Some(type_path) = wrapped.strip_suffix("::from(val)") {
                    format!("{serde_bindings}{core_call}{await_kw}.map({type_path}::from){serde_err_conv}")
                } else {
                    format!("{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){serde_err_conv}")
                }
            }
        } else if func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy {
            // Async function that can't be auto-delegated — wrap unimplemented body in future_into_py
            let suppress = if func.params.is_empty() {
                String::new()
            } else {
                let names: Vec<&str> = func.params.iter().map(|p| p.name.as_str()).collect();
                format!("let _ = ({});\n        ", names.join(", "))
            };
            format!(
                "{suppress}Err(pyo3::exceptions::PyNotImplementedError::new_err(\"not implemented: {}\"))",
                func.name
            )
        } else {
            // Function can't be auto-delegated — return a default/error based on return type
            gen_unimplemented_body(
                &func.return_type,
                &func.name,
                func.error_type.is_some(),
                cfg,
                &func.params,
                opaque_types,
            )
        }
    } else if func.is_async {
        // MARKER_DELEGATE_ASYNC
        let core_call = format!("{core_fn_path}({call_args})");
        // In async contexts (future_into_py, etc.), the compiler often can't infer the
        // target type for .into(). Use explicit From::from() / collect::<Vec<T>>() instead.
        let return_wrap = match &func.return_type {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                let mapped_n = mapper.named(n);
                let wrap = arc_wrap_expr("result", n, mutex_types);
                format!("{mapped_n} {{ inner: {wrap} }}")
            }
            TypeRef::Named(_) => {
                format!("{return_type}::from(result)")
            }
            TypeRef::Vec(inner) => match inner.as_ref() {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    let mapped_n = mapper.named(n);
                    let wrap = arc_wrap_expr("v", n, mutex_types);
                    format!("result.into_iter().map(|v| {mapped_n} {{ inner: {wrap} }}).collect::<Vec<_>>()")
                }
                TypeRef::Named(_) => {
                    let inner_mapped = mapper.map_type(inner);
                    format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
                }
                _ => "result".to_string(),
            },
            TypeRef::Unit => "result".to_string(),
            _ => {
                // Cast wide-integer leaf primitives (extendr: usize/u64/i64/isize → f64,
                // u8/u16/u32 → i32) so the awaited value matches the rendered signature.
                // No-op for backends that do not set the cast flags. The shared helper handles
                // scalar/Vec/Option/Option<Vec> shapes; everything else passes through
                // binding_helpers::wrap_return unchanged.
                let cast = cast_return_expr(
                    &func.return_type,
                    "result",
                    cfg.cast_large_ints_to_f64,
                    cfg.cast_uints_to_i32,
                );
                cast.unwrap_or_else(|| {
                    super::binding_helpers::wrap_return(
                        "result",
                        &func.return_type,
                        "",
                        opaque_types,
                        false,
                        func.returns_ref,
                        false,
                    )
                })
            }
        };

        // For Pyo3 async functions with let_bindings that create temporary borrows,
        // the bindings must be moved INSIDE the `async move` block to extend their
        // lifetime past the function return (when the future executes).
        // The serde async path (lines 233-255) already does this correctly.
        if cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy && !let_bindings.is_empty() {
            let is_unit = matches!(func.return_type, TypeRef::Unit);
            let result_handling = if func.error_type.is_some() {
                format!(
                    "let result = {core_call}.await\n            \
                     .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?;"
                )
            } else if is_unit {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let (ok_expr, extra_binding) = if is_unit && func.error_type.is_none() {
                ("()".to_string(), String::new())
            } else if return_wrap.contains(".into()") || return_wrap.contains("::from(") {
                let wrapped_var = "wrapped_result";
                let binding = if let Some(ret_type) = Some(&return_type) {
                    format!("let {wrapped_var}: {ret_type} = {return_wrap};\n            ")
                } else {
                    format!("let {wrapped_var} = {return_wrap};\n            ")
                };
                (wrapped_var.to_string(), binding)
            } else {
                (return_wrap.to_string(), String::new())
            };
            // Move let_bindings inside the async block
            let inner_body = format!("{let_bindings}{result_handling}\n            {extra_binding}Ok({ok_expr})");
            format!("pyo3_async_runtimes::tokio::future_into_py(py, async move {{\n{inner_body}\n        }})")
        } else {
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
        }
    } else {
        // For sync PyO3 the blocking core call is wrapped in `py.detach(|| ...)` to release
        // the GIL (no-op for other backends).
        let core_call = detach_core_call(&format!("{core_fn_path}({call_args})"));

        // When a wide-integer cast flag rewrites the return type's leaf primitive (extendr maps
        // usize/u64/i64/isize → f64 and u8/u16/u32 → i32), cast the core-call value so the body
        // matches the rendered signature. `cast_value` casts the unwrapped value expression
        // (scalar / Vec / Option / Option<Vec> shapes); it is a no-op for backends that do not
        // set the flags. Applied to the `Ok` payload in the Result path and to the bare value
        // otherwise — never to a Result or serialized String.
        let cast_value = |expr: &str| -> String {
            cast_return_expr(
                &func.return_type,
                expr,
                cfg.cast_large_ints_to_f64,
                cfg.cast_uints_to_i32,
            )
            .unwrap_or_else(|| expr.to_string())
        };

        // Determine return wrapping strategy
        let returns_ref = func.returns_ref;
        let wrap_return = |expr: &str| -> String {
            match &func.return_type {
                // Opaque type return: wrap in Arc (or Arc<Mutex<_>> when the type
                // has &mut self methods). Skip the wrap if `expr` is already Arc<T>.
                TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                    let mapped_name = mapper.named(name);
                    if expr_is_already_arc(expr) {
                        format!("{mapped_name} {{ inner: {expr} }}")
                    } else if returns_ref {
                        let wrap = arc_wrap_expr(&format!("{expr}.clone()"), name, mutex_types);
                        format!("{mapped_name} {{ inner: {wrap} }}")
                    } else {
                        let wrap = arc_wrap_expr(expr, name, mutex_types);
                        format!("{mapped_name} {{ inner: {wrap} }}")
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
                // String/Bytes: .into() handles &str→String, skip for owned
                TypeRef::String | TypeRef::Bytes => {
                    if returns_ref {
                        format!("{expr}.into()")
                    } else {
                        expr.to_string()
                    }
                }
                // Path: PathBuf→String needs to_string_lossy
                TypeRef::Path => format!("{expr}.to_string_lossy().to_string()"),
                // Json: serde_json::Value to string
                TypeRef::Json => format!("{expr}.to_string()"),
                // Optional with opaque inner
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        let mapped_name = mapper.named(name);
                        if returns_ref {
                            let wrap = arc_wrap_expr("v.clone()", name, mutex_types);
                            format!("{expr}.map(|v| {mapped_name} {{ inner: {wrap} }})")
                        } else {
                            let wrap = arc_wrap_expr("v", name, mutex_types);
                            format!("{expr}.map(|v| {mapped_name} {{ inner: {wrap} }})")
                        }
                    }
                    TypeRef::Named(_) => {
                        if returns_ref {
                            format!("{expr}.map(|v| v.clone().into())")
                        } else {
                            format!("{expr}.map(Into::into)")
                        }
                    }
                    TypeRef::Path => {
                        format!("{expr}.map(|v| v.to_string_lossy().to_string())")
                    }
                    TypeRef::String | TypeRef::Bytes => {
                        if returns_ref {
                            format!("{expr}.map(Into::into)")
                        } else {
                            expr.to_string()
                        }
                    }
                    TypeRef::Vec(vi) => match vi.as_ref() {
                        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                            let mapped_name = mapper.named(name);
                            let wrap = arc_wrap_expr("x", name, mutex_types);
                            format!(
                                "{expr}.map(|v| v.into_iter().map(|x| {mapped_name} {{ inner: {wrap} }}).collect())"
                            )
                        }
                        TypeRef::Named(_) => {
                            format!("{expr}.map(|v| v.into_iter().map(Into::into).collect())")
                        }
                        _ => expr.to_string(),
                    },
                    _ => expr.to_string(),
                },
                // Vec<Named>: map each element through Into
                TypeRef::Vec(inner) => match inner.as_ref() {
                    TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
                        let mapped_name = mapper.named(name);
                        if returns_ref {
                            let wrap = arc_wrap_expr("v.clone()", name, mutex_types);
                            format!("{expr}.into_iter().map(|v| {mapped_name} {{ inner: {wrap} }}).collect()")
                        } else {
                            let wrap = arc_wrap_expr("v", name, mutex_types);
                            format!("{expr}.into_iter().map(|v| {mapped_name} {{ inner: {wrap} }}).collect()")
                        }
                    }
                    TypeRef::Named(_) => {
                        if returns_ref {
                            // `&[T]` → `Vec<U>`: use `.iter()` not `.into_iter()`
                            // to avoid clippy::into_iter_on_ref under -D warnings.
                            format!("{expr}.iter().map(|v| v.clone().into()).collect()")
                        } else {
                            format!("{expr}.into_iter().map(Into::into).collect()")
                        }
                    }
                    TypeRef::Path => {
                        format!("{expr}.into_iter().map(|v| v.to_string_lossy().to_string()).collect()")
                    }
                    TypeRef::String => {
                        if returns_ref {
                            // `&[&str]` → `Vec<String>`. `Into::into` would require
                            // `impl From<&&str> for String` (which doesn't exist).
                            format!("{expr}.iter().map(|s| s.to_string()).collect()")
                        } else {
                            expr.to_string()
                        }
                    }
                    TypeRef::Bytes => {
                        if returns_ref {
                            format!("{expr}.iter().map(|b| b.to_vec()).collect()")
                        } else {
                            expr.to_string()
                        }
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
                AsyncPattern::TokioBlockOn => {
                    ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))"
                }
                _ => ".map_err(|e| e.to_string())",
            };
            let wrapped = wrap_return("val");
            // Cast the `Ok` payload first; wide-int return shapes (scalar/Vec/Option primitives)
            // make `wrap_return("val") == "val"`, so the cast IS the wrapped value.
            let cast_val = cast_value("val");
            if wrapped == "val" {
                if cast_val == "val" {
                    format!("{core_call}{err_conv}")
                } else {
                    format!("{core_call}.map(|val| {cast_val}){err_conv}")
                }
            } else if wrapped == "val.into()" {
                format!("{core_call}.map(Into::into){err_conv}")
            } else if let Some(type_path) = wrapped.strip_suffix("::from(val)") {
                format!("{core_call}.map({type_path}::from){err_conv}")
            } else {
                format!("{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            let cast = cast_value(&core_call);
            wrap_return(&cast)
        }
    };

    // Prepend let bindings for non-opaque Named params (sync delegate case).
    // Only prepend when can_delegate is true — the !can_delegate serde path does its own bindings.
    // However, always prepend Vec<String> ref bindings (names_refs) since serde path doesn't handle them.
    let body = if !let_bindings.is_empty() && !func.is_async {
        if can_delegate {
            format!("{let_bindings}{body}")
        } else {
            // For the !can_delegate path, only prepend Vec<String>+is_ref bindings (names_refs)
            // since serde bindings handle Named type conversions.
            let vec_str_bindings: String = func.params.iter().filter(|p| {
                p.is_ref && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char))
            }).map(|p| {
                // Handle both Vec<String> and Option<Vec<String>> parameters.
                // When p.optional=true, p.ty is the inner type (Vec<String>), so we need to unwrap first.
                if p.optional {
                    format!("let {}_refs: Vec<&str> = {}.as_ref().map(|v| v.iter().map(|s| s.as_str()).collect()).unwrap_or_default();\n    ", p.name, p.name)
                } else {
                    format!("let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();\n    ", p.name, p.name)
                }
            }).collect();
            if !vec_str_bindings.is_empty() {
                format!("{vec_str_bindings}{body}")
            } else {
                body
            }
        }
    } else {
        body
    };

    // Wrap long signature if necessary
    // TokioBlockOn functions block synchronously inside the body — the generated function
    // must NOT be `async fn` because extendr's `#[extendr]` cannot return a Future.
    let async_kw = if func.is_async && cfg.async_pattern != AsyncPattern::TokioBlockOn {
        "async "
    } else {
        ""
    };
    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;

    // For async PyO3 free functions, override return type and add lifetime generic.
    let ret = if func_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };
    let func_lifetime = if func_needs_py { "<'py>" } else { "" };

    // Sync PyO3 free functions take an injected `py: Python<'_>` handle so the body can call
    // `py.detach(...)` to release the GIL across the blocking core call (see `pyo3_sync` above).
    // PyO3 supplies this argument automatically; it is excluded from `#[pyo3(signature = (...))]`.
    let sync_py_prefix = if pyo3_sync { "py: Python<'_>, " } else { "" };

    let (func_sig, _params_formatted) = if params.len() > 100 {
        // Wrap the signature across multiple lines for readability. Reuse the exact
        // per-parameter strings computed above (`param_strings`) — recomputing the types here
        // would drop backend-aware mappings such as extendr's `Nullable<&T>` back to `Option<T>`,
        // yielding a signature that disagrees with the generated body.
        let wrapped_params = param_strings.join(",\n    ");

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
                    "pub {async_kw}fn {}(\n    {sync_py_prefix}{}\n) -> {ret}",
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
        (
            format!("pub {async_kw}fn {}({sync_py_prefix}{params}) -> {ret}", func.name),
            "",
        )
    };

    let total_params = func.params.len() + if func_needs_py || pyo3_sync { 1 } else { 0 };
    let sig_defaults = if cfg.needs_signature {
        function_sig_defaults(&func.params)
    } else {
        String::new()
    };
    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    crate::codegen::template_env::render(
        "generators/functions/function_definition.jinja",
        minijinja::context! {
            has_too_many_arguments => total_params > 7,
            has_missing_errors_doc => func.error_type.is_some(),
            attr_inner => attr_inner,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_defaults => sig_defaults,
            signature_suffix => cfg.signature_suffix,
            func_sig => func_sig,
            body => body,
        },
    )
}

fn can_delegate_with_named_let_bindings(func: &FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && crate::codegen::shared::is_delegatable_return(&func.return_type)
}

/// Collect all unique trait import paths from types' methods.
///
/// Returns a deduplicated, sorted list of trait paths (e.g. `["sample_llm::LlmClient"]`)
/// that need to be imported in generated binding code so that trait methods can be called.
/// Both opaque and non-opaque types are scanned because non-opaque wrapper types also
/// delegate trait method calls to their inner core type.
pub fn collect_trait_imports(api: &ApiSurface) -> Vec<String> {
    // Collect all trait paths, then deduplicate by last segment (trait name).
    // When two paths resolve to the same trait name (e.g. `mylib_core::Dependency`
    // and `mylib_core::di::Dependency`), only one import is needed. Keep the
    // shorter (public re-export) path to avoid E0252 duplicate-import errors.
    let mut traits: AHashSet<String> = AHashSet::new();
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if let Some(ref trait_path) = method.trait_source {
                traits.insert(trait_path.clone());
            }
        }
    }

    // Deduplicate by last path segment: keep the shortest path for each trait name.
    let mut by_name: AHashMap<String, String> = AHashMap::new();
    for path in traits {
        let name = path.split("::").last().unwrap_or(&path).to_string();
        let entry = by_name.entry(name).or_insert_with(|| path.clone());
        // Prefer shorter paths (public re-exports are shorter than internal paths)
        if path.len() < entry.len() {
            *entry = path;
        }
    }

    let mut sorted: Vec<String> = by_name.into_values().collect();
    sorted.sort();
    sorted
}

/// Check if any type has methods from trait impls whose trait_source could not be resolved.
///
/// When true, the binding crate should add a glob import of the core crate (e.g.
/// `use sample_core::*`) to bring all publicly exported traits into scope.
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
