use crate::codegen::generators::{AsyncPattern, RustBindingConfig};

/// Resolve the `.map_err(...)` conversion for a free function's core `Result::Err`.
///
/// PyO3 free functions whose declared `error_type` has a matching `{error}_to_py_err`
/// converter in `cfg.error_converters` (populated by the pyo3 backend from the same
/// `gen_pyo3_error_converter` output the trait-bridge and capsule call sites already route
/// through) convert through that typed converter, so `except {Variant}Error:` catches the
/// right exception instead of every error kind collapsing into a generic
/// `pyo3::exceptions::PyRuntimeError` (alef #452). Every other async pattern, and any pyo3
/// call site that leaves `error_converters` unset, keeps today's generic conversion
/// unchanged. ~keep
pub(crate) fn resolve_err_conv(cfg: &RustBindingConfig<'_>, error_type: Option<&str>) -> String {
    if cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy
        && let Some(converters) = cfg.error_converters
        && let Some(error_type) = error_type
    {
        let short_name = error_type.rsplit("::").next().unwrap_or(error_type);
        let candidate = format!("{}_to_py_err", crate::codegen::naming::pascal_to_snake(short_name));
        if converters.iter().any(|c| c == &candidate) {
            return format!(".map_err({candidate})");
        }
    }
    match cfg.async_pattern {
        AsyncPattern::Pyo3FutureIntoPy => {
            ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
        }
        AsyncPattern::NapiNativeAsync => {
            ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))".to_string()
        }
        AsyncPattern::WasmNativeAsync => ".map_err(|e| JsValue::from_str(&e.to_string()))".to_string(),
        AsyncPattern::TokioBlockOn => {
            ".map_err(|e| extendr_api::Error::Other(e.to_string().replace(\":\", \"_\").replace(\"/\", \"_\").replace(\"-\", \"_\").chars().take(255).collect::<String>()))".to_string()
        }
        _ => ".map_err(|e| e.to_string())".to_string(),
    }
}
