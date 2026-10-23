//! The error type every fallible swift-bridge function returns.
//!
//! A bridged function that returns `Result<T, String>` makes Swift throw a `RustString`, so the
//! caller cannot read the error kind, the HTTP status or the retry delay. Every fallible
//! `extern "Rust"` function returns `Result<T, AlefBridgeError>` instead. The Rust error is
//! converted once, in `alef_bridge_error`, and the Swift facade converts the thrown value into the
//! typed Swift error.

use crate::core::ir::{ApiSurface, ErrorDef, FieldDef, PrimitiveType, TypeRef};

/// Name of the opaque error type in the bridge module.
pub(crate) const BRIDGE_ERROR_TYPE: &str = "AlefBridgeError";

/// The `Result` type of a bridged function whose success type is `ok`.
pub(crate) fn result_type(ok: &str) -> String {
    format!("Result<{ok}, {BRIDGE_ERROR_TYPE}>")
}

/// The `extern "Rust"` block that declares the error type and its accessors.
pub(crate) fn extern_block() -> String {
    crate::backends::swift::template_env::render(
        "rust_bridge_error_extern.rs.jinja",
        crate::alef_context! { bridge_error_type => BRIDGE_ERROR_TYPE },
    )
}

#[derive(serde::Serialize)]
struct ErrorContext {
    name: String,
    snake: String,
    rust_path: String,
    variants: Vec<VariantContext>,
    has_payload_fields: bool,
    status_expr: String,
    retry_after_expr: String,
}

#[derive(serde::Serialize)]
struct VariantContext {
    name: String,
    pattern: String,
    fields: Vec<PayloadField>,
}

#[derive(serde::Serialize)]
struct PayloadField {
    key: String,
    binding: String,
}

fn payload_field_supported(field: &FieldDef) -> bool {
    !field.binding_excluded && super::super::gen_bindings::errors::bridge_payload_supported(&field.ty)
}

fn payload_fields(variant: &crate::core::ir::ErrorVariant) -> Vec<PayloadField> {
    variant
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| payload_field_supported(field))
        .map(|(index, field)| PayloadField {
            key: serde_json::to_string(&field.name).expect("field names are strings"),
            binding: format!("__alef_field_{index}"),
        })
        .collect()
}

fn payload_pattern(rust_path: &str, variant: &crate::core::ir::ErrorVariant) -> String {
    if variant.is_unit || variant.fields.is_empty() {
        return crate::codegen::error_gen::error_variant_wildcard_pattern(rust_path, variant);
    }
    let tuple = crate::codegen::conversions::is_tuple_variant(&variant.fields);
    let fields: Vec<String> = variant
        .fields
        .iter()
        .enumerate()
        .filter(|(_, field)| tuple || payload_field_supported(field))
        .map(|(index, field)| {
            let binding = if payload_field_supported(field) {
                format!("__alef_field_{index}")
            } else {
                "_".to_string()
            };
            if tuple {
                binding
            } else {
                format!(
                    "{}: {binding}",
                    crate::codegen::naming::internal_rust_identifier(&field.name)
                )
            }
        })
        .collect();
    if fields.is_empty() {
        return crate::codegen::error_gen::error_variant_wildcard_pattern(rust_path, variant);
    }
    let joined = fields.join(", ");
    if tuple {
        format!("{rust_path}::{}({joined})", variant.name)
    } else {
        format!("{rust_path}::{} {{ {joined}, .. }}", variant.name)
    }
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
                        pattern: payload_pattern(&rust_path, variant),
                        fields: payload_fields(variant),
                    })
                    .collect(),
                has_payload_fields: error.variants.iter().any(|variant| !payload_fields(variant).is_empty()),
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

#[cfg(test)]
#[path = "bridge_error_pattern_tests.rs"]
mod tests;
