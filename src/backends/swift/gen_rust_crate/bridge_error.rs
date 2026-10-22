//! The error type every fallible swift-bridge function returns.
//!
//! A bridged function that returns `Result<T, String>` makes Swift throw a `RustString`, so the
//! caller cannot read the error kind, the HTTP status or the retry delay. Every fallible
//! `extern "Rust"` function returns `Result<T, AlefBridgeError>` instead. The Rust error is
//! converted once, in `alef_bridge_error`, and the Swift facade converts the thrown value into the
//! typed Swift error.

use crate::core::ir::{ApiSurface, ErrorDef, PrimitiveType, TypeRef};

/// Name of the opaque error type in the bridge module.
pub(crate) const BRIDGE_ERROR_TYPE: &str = "AlefBridgeError";

/// The `Result` type of a bridged function whose success type is `ok`.
pub(crate) fn result_type(ok: &str) -> String {
    format!("Result<{ok}, {BRIDGE_ERROR_TYPE}>")
}

/// The `extern "Rust"` block that declares the error type and its accessors.
pub(crate) fn extern_block() -> String {
    format!(
        "    extern \"Rust\" {{\n        type {BRIDGE_ERROR_TYPE};\n\n\
         \x20       fn message(self: &{BRIDGE_ERROR_TYPE}) -> String;\n\
         \x20       fn error_type(self: &{BRIDGE_ERROR_TYPE}) -> String;\n\
         \x20       fn kind(self: &{BRIDGE_ERROR_TYPE}) -> String;\n\
         \x20       fn status(self: &{BRIDGE_ERROR_TYPE}) -> i32;\n\
         \x20       #[swift_bridge(swift_name = \"retryAfterMs\")]\n\
         \x20       fn retry_after_ms(self: &{BRIDGE_ERROR_TYPE}) -> i64;\n    }}\n\n"
    )
}

#[derive(serde::Serialize)]
struct ErrorContext {
    name: String,
    snake: String,
    rust_path: String,
    variants: Vec<VariantContext>,
    status_expr: String,
    retry_after_expr: String,
}

#[derive(serde::Serialize)]
struct VariantContext {
    name: String,
    pattern: String,
}

/// The expression that reads the HTTP status of `e`, or `-1` when the error has no `status_code`
/// method. A status of `0` is the status of a variant that has no HTTP status.
fn status_expr(error: &ErrorDef) -> String {
    let has_status = error.methods.iter().any(|method| {
        method.name == "status_code"
            && !method.is_static
            && !method.is_async
            && method.params.is_empty()
            && matches!(method.return_type, TypeRef::Primitive(PrimitiveType::U16))
    });
    if has_status {
        "i32::from(e.status_code())".to_string()
    } else {
        "-1".to_string()
    }
}

/// The expression that reads `retry_after` of `e` in whole milliseconds, or `-1` when the error
/// has no such method or the delay is absent.
fn retry_after_expr(error: &ErrorDef) -> String {
    let shape = error
        .methods
        .iter()
        .filter(|method| method.name == "retry_after" && !method.is_static && !method.is_async)
        .filter(|method| method.params.is_empty())
        .find_map(|method| crate::codegen::error_gen::duration_shape(&method.return_type));
    match shape {
        Some(crate::codegen::error_gen::DurationShape::Optional) => {
            "e.retry_after().map_or(-1, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))".to_string()
        }
        Some(crate::codegen::error_gen::DurationShape::Bare) => {
            "i64::try_from(e.retry_after().as_millis()).unwrap_or(i64::MAX)".to_string()
        }
        None => "-1".to_string(),
    }
}

fn error_contexts(api: &ApiSurface, source_crate: &str) -> Vec<ErrorContext> {
    api.errors
        .iter()
        .filter(|error| !error.binding_excluded)
        .map(|error| {
            let rust_path = if error.rust_path.is_empty() {
                format!("{source_crate}::{}", error.name)
            } else {
                error.rust_path.replace('-', "_")
            };
            ErrorContext {
                name: error.name.clone(),
                snake: heck::ToSnakeCase::to_snake_case(error.name.as_str()),
                variants: error
                    .variants
                    .iter()
                    .map(|variant| VariantContext {
                        name: variant.name.clone(),
                        pattern: crate::codegen::error_gen::error_variant_wildcard_pattern(&rust_path, variant),
                    })
                    .collect(),
                status_expr: status_expr(error),
                retry_after_expr: retry_after_expr(error),
                rust_path,
            }
        })
        .collect()
}

/// The definition of the error type and of `alef_bridge_error`, which builds it from any error.
pub(crate) fn definition(api: &ApiSurface, source_crate: &str) -> String {
    crate::backends::swift::template_env::render(
        "rust_bridge_error.rs.jinja",
        crate::alef_context! {
            errors => error_contexts(api, source_crate),
        },
    )
}
