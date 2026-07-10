use crate::codegen::generators::{AsyncPattern, RustBindingConfig};

/// Generate the body for an async call, unified across methods, static methods, and free functions.
///
/// - `core_call`: the expression to await, e.g. `inner.method(args)` or `CoreType::fn(args)`.
///   For Pyo3FutureIntoPy opaque methods this should reference `inner` (the Arc clone);
///   for all other patterns it may reference `self.inner` or a static call expression.
/// - `cfg`: binding configuration (determines which async pattern to emit)
/// - `has_error`: whether the core call returns a `Result`
/// - `return_wrap`: expression to produce the binding return value from `result`,
///   e.g. `"result"` or `"TypeName::from(result)"`
///
/// - `is_opaque`: whether the binding type is Arc-wrapped (affects TokioBlockOn wrapping)
/// - `inner_clone_line`: optional statement emitted before the pattern-specific body,
///   e.g. `"let inner = self.inner.clone();\n        "` for opaque instance methods, or `""`.
///   Required when `core_call` references `inner` (Pyo3FutureIntoPy opaque case).
#[allow(clippy::too_many_arguments)]
pub fn gen_async_body(
    core_call: &str,
    cfg: &RustBindingConfig,
    has_error: bool,
    return_wrap: &str,
    is_opaque: bool,
    inner_clone_line: &str,
    is_unit_return: bool,
    return_type: Option<&str>,
) -> String {
    let pattern_body = match cfg.async_pattern {
        AsyncPattern::Pyo3FutureIntoPy => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n            \
                     .map_err(|e| PyErr::new::<PyRuntimeError, _>(e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let (ok_expr, extra_binding) = if is_unit_return && !has_error {
                ("()".to_string(), String::new())
            } else if return_wrap.contains(".into()") || return_wrap.contains("::from(") {
                let wrapped_var = "wrapped_result";
                let binding = if let Some(ret_type) = return_type {
                    format!("let {wrapped_var}: {ret_type} = {return_wrap};\n            ")
                } else {
                    format!("let {wrapped_var} = {return_wrap};\n            ")
                };
                (wrapped_var.to_string(), binding)
            } else {
                (return_wrap.to_string(), String::new())
            };
            crate::codegen::template_env::render(
                "binding_helpers/async_body_pyo3.jinja",
                minijinja::context! {
                    result_handling => result_handling,
                    extra_binding => extra_binding,
                    ok_expr => ok_expr,
                },
            )
        }
        AsyncPattern::WasmNativeAsync => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n        \
                     .map_err(|e| JsValue::from_str(&e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let ok_expr = if is_unit_return && !has_error {
                "()"
            } else {
                return_wrap
            };
            crate::codegen::template_env::render(
                "binding_helpers/async_body_wasm.jinja",
                minijinja::context! {
                    result_handling => result_handling,
                    ok_expr => ok_expr,
                },
            )
        }
        AsyncPattern::NapiNativeAsync => {
            let result_handling = if has_error {
                format!(
                    "let result = {core_call}.await\n            \
                     .map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))?;"
                )
            } else if is_unit_return {
                format!("{core_call}.await;")
            } else {
                format!("let result = {core_call}.await;")
            };
            let (needs_ok_wrapper, ok_expr) = if !has_error && !is_unit_return {
                (false, return_wrap.to_string())
            } else {
                let expr = if is_unit_return && !has_error {
                    "()".to_string()
                } else {
                    return_wrap.to_string()
                };
                (true, expr)
            };
            crate::codegen::template_env::render(
                "binding_helpers/async_body_napi.jinja",
                minijinja::context! {
                    result_handling => result_handling,
                    needs_ok_wrapper => needs_ok_wrapper,
                    ok_expr => ok_expr,
                    return_wrap => return_wrap,
                },
            )
        }
        AsyncPattern::TokioBlockOn => {
            let rt_new = "tokio::runtime::Runtime::new()\
                          .map_err(|e| extendr_api::Error::Other(e.to_string()))?";
            let err_map = ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))";
            crate::codegen::template_env::render(
                "binding_helpers/async_body_tokio.jinja",
                minijinja::context! {
                    has_error => has_error,
                    is_opaque => is_opaque,
                    is_unit_return => is_unit_return,
                    core_call => core_call,
                    return_wrap => return_wrap,
                    rt_new => rt_new,
                    err_map => err_map,
                },
            )
        }
        AsyncPattern::None => {
            "compile_error!(\"async delegation is not supported by this backend; exclude the item or configure an adapter\")"
                .to_string()
        }
    };
    if inner_clone_line.is_empty() {
        pattern_body
    } else {
        format!("{inner_clone_line}{pattern_body}")
    }
}
