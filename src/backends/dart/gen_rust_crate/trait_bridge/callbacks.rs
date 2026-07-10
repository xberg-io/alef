use crate::core::ir::MethodDef;

use super::excluded::substitute_excluded_carriers_in_rust_type;
use crate::backends::dart::gen_rust_crate::conversions::frb_rust_type_excluded_aware;

/// Build the callback closure type stored in the bridge struct field.
///
/// Closures always accept **owned** FRB-friendly mirror types (the Dart FFI layer
/// decodes arguments as mirror types, not source-crate types). Returns a
/// `DartFnFuture<T>` wrapping the FRB-friendly mirror return type.
///
/// For excluded named types, substitutes the JSON-backed bridge
/// type so FRB generates a constructible Dart object without exposing the internal
/// Rust struct as a public DTO.
///
/// Example: `Box<dyn Fn(Vec<u8>, OcrConfig) -> DartFnFuture<HiddenDocumentBridge> + Send + Sync>`
pub(super) fn dart_fn_future_callback_type(
    method: &MethodDef,
    source_crate_name: &str,
    _type_paths: &std::collections::HashMap<String, String>,
    excluded_type_paths: &std::collections::HashMap<String, String>,
) -> String {
    let (params_str, dart_fn_ret) = dart_fn_future_params_and_ret(method, source_crate_name, excluded_type_paths);
    format!("Box<dyn Fn({params_str}) -> {dart_fn_ret} + Send + Sync>")
}

/// Build the factory-parameter closure type for a non-`type_alias` trait bridge.
///
/// FRB v2 only generates Dart-callable function types for closure parameters when
/// the Rust signature uses the bare `impl Fn(...) -> DartFnFuture<R> + Send + Sync
/// + 'static` shape — `Box<dyn Fn(...)>` parameters render as opaque `BoxFn…`
/// classes that cannot be constructed from Dart user code. Closure struct fields
/// stay `Box<dyn Fn(...)>` (see `dart_fn_future_callback_type`); the factory
/// boxes each `impl Fn` argument as it stores it.
///
/// Example: `impl Fn(Vec<u8>, OcrConfig) -> DartFnFuture<HiddenDocumentBridge> + Send + Sync + 'static`
pub(super) fn dart_fn_future_factory_param_type(
    method: &MethodDef,
    source_crate_name: &str,
    _type_paths: &std::collections::HashMap<String, String>,
    excluded_type_paths: &std::collections::HashMap<String, String>,
) -> String {
    let (params_str, dart_fn_ret) = dart_fn_future_params_and_ret(method, source_crate_name, excluded_type_paths);
    format!("impl Fn({params_str}) -> {dart_fn_ret} + Send + Sync + 'static")
}

fn dart_fn_future_params_and_ret(
    method: &MethodDef,
    source_crate_name: &str,
    excluded_type_paths: &std::collections::HashMap<String, String>,
) -> (String, String) {
    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = frb_rust_type_excluded_aware(&p.ty, p.optional, excluded_type_paths);
            substitute_excluded_carriers_in_rust_type(&ty, source_crate_name, excluded_type_paths)
        })
        .collect();

    let ret = frb_rust_type_excluded_aware(&method.return_type, false, excluded_type_paths);
    let ret_substituted = substitute_excluded_carriers_in_rust_type(&ret, source_crate_name, excluded_type_paths);
    let dart_fn_ret = format!("DartFnFuture<{ret_substituted}>");

    (params.join(", "), dart_fn_ret)
}
